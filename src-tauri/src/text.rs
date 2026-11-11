//! Transcript post-processing: hallucination filtering and snippet expansion.
//!
//! Matching works on *word tokens* (maximal runs of alphanumeric chars)
//! compared after case folding, so punctuation Whisper adds ("İmzam.") and
//! Turkish dotted/dotless I variants never prevent a match. Replacement
//! splices the original text by byte span, so everything that is not a
//! trigger is preserved exactly as transcribed.

use crate::settings::{Snippet, SnippetMatch, TextSettings};

/// Whole-transcript phrases Whisper produces over silence or noise (learned
/// from subtitled video). Matched against the ENTIRE transcript only, never
/// a substring, so real dictation containing these words is untouched.
const WHOLE_HALLUCINATIONS: &[&str] = &[
    "altyazı m.k.",
    "altyazı m.k",
    "altyazılar amara.org topluluğu tarafından",
    "izlediğiniz için teşekkürler",
    "izlediğiniz için teşekkür ederim",
    "abone olmayı unutmayın",
    "videoyu beğenmeyi ve abone olmayı unutmayın",
    "thanks for watching",
    "thank you for watching",
    "thank you for watching!",
    "please subscribe",
    "subtitles by the amara.org community",
    "transcription by castingwords",
];

/// Credits Whisper appends after real speech that ends on a pause.
const TRAILING_HALLUCINATIONS: &[&str] = &["altyazı m.k.", "subtitles by the amara.org community"];

/// A word token: folded form plus its byte span in the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    folded: String,
    start: usize,
    end: usize,
}

/// Case-folds one char for matching. All four Turkish I variants collapse to
/// `i` so Turkish and English spellings of a trigger match each other.
fn fold_char(c: char, out: &mut String) {
    match c {
        'I' | 'İ' | 'ı' | 'i' => out.push('i'),
        _ => out.extend(c.to_lowercase().filter(|&l| l != '\u{0307}')),
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (idx, c) in text.char_indices() {
        if c.is_alphanumeric() {
            let (_, folded) = current.get_or_insert_with(|| (idx, String::new()));
            fold_char(c, folded);
        } else if let Some((start, folded)) = current.take() {
            tokens.push(Token {
                folded,
                start,
                end: idx,
            });
        }
    }
    if let Some((start, folded)) = current {
        tokens.push(Token {
            folded,
            start,
            end: text.len(),
        });
    }
    tokens
}

fn folded_words(text: &str) -> Vec<String> {
    tokenize(text).into_iter().map(|t| t.folded).collect()
}

fn tokens_equal(tokens: &[Token], words: &[String]) -> bool {
    tokens.len() == words.len() && tokens.iter().zip(words).all(|(t, w)| &t.folded == w)
}

/// Returns an empty string for known silence hallucinations and strips
/// trailing subtitle credits from otherwise real speech.
pub fn strip_hallucinations(text: &str) -> String {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return String::new();
    }
    if WHOLE_HALLUCINATIONS
        .iter()
        .any(|phrase| tokens_equal(&tokens, &folded_words(phrase)))
    {
        return String::new();
    }
    for phrase in TRAILING_HALLUCINATIONS {
        let words = folded_words(phrase);
        if tokens.len() > words.len() && tokens_equal(&tokens[tokens.len() - words.len()..], &words)
        {
            let cut = tokens[tokens.len() - words.len()].start;
            return text[..cut].trim_end().to_owned();
        }
    }
    text.to_owned()
}

/// A snippet prepared for matching: trigger tokens folded once up front.
struct PreparedSnippet<'a> {
    words: Vec<String>,
    expansion: &'a str,
}

fn prepare<'a>(snippets: &'a [Snippet], mode: SnippetMatch) -> Vec<PreparedSnippet<'a>> {
    let mut prepared: Vec<PreparedSnippet<'a>> = snippets
        .iter()
        .filter(|s| s.enabled && s.match_mode == mode)
        .map(|s| PreparedSnippet {
            words: folded_words(&s.trigger),
            expansion: &s.expansion,
        })
        .filter(|p| !p.words.is_empty())
        .collect();
    // Longest trigger first so "e-posta imzam" wins over "imzam".
    prepared.sort_by_key(|p| std::cmp::Reverse(p.words.len()));
    prepared
}

/// If the whole utterance equals an exact-mode trigger, returns its expansion.
fn expand_exact<'a>(text: &str, snippets: &'a [Snippet]) -> Option<&'a str> {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return None;
    }
    prepare(snippets, SnippetMatch::Exact)
        .into_iter()
        .find(|p| tokens_equal(&tokens, &p.words))
        .map(|p| p.expansion)
}

/// Replaces inline-mode triggers wherever they appear as whole words.
/// Single left-to-right pass: an expansion is never re-scanned, so a snippet
/// cannot trigger another one.
fn expand_inline(text: &str, snippets: &[Snippet]) -> String {
    let prepared = prepare(snippets, SnippetMatch::Inline);
    if prepared.is_empty() {
        return text.to_owned();
    }
    let tokens = tokenize(text);
    let mut out = String::with_capacity(text.len());
    let mut copied_until = 0;
    let mut i = 0;
    while i < tokens.len() {
        let hit = prepared.iter().find(|p| {
            let end = i + p.words.len();
            end <= tokens.len() && tokens_equal(&tokens[i..end], &p.words)
        });
        match hit {
            Some(p) => {
                let last = &tokens[i + p.words.len() - 1];
                out.push_str(&text[copied_until..tokens[i].start]);
                out.push_str(p.expansion);
                copied_until = last.end;
                i += p.words.len();
            }
            None => i += 1,
        }
    }
    out.push_str(&text[copied_until..]);
    out
}

/// Full post-processing pipeline applied to every raw engine transcript.
pub fn process(raw: &str, settings: &TextSettings) -> String {
    let trimmed = raw.trim();
    let filtered = if settings.filter_hallucinations {
        strip_hallucinations(trimmed)
    } else {
        trimmed.to_owned()
    };
    if filtered.is_empty() {
        return filtered;
    }
    if let Some(expansion) = expand_exact(&filtered, &settings.snippets) {
        return expansion.to_owned();
    }
    expand_inline(&filtered, &settings.snippets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(trigger: &str, expansion: &str, mode: SnippetMatch) -> Snippet {
        Snippet {
            id: trigger.into(),
            trigger: trigger.into(),
            expansion: expansion.into(),
            match_mode: mode,
            enabled: true,
        }
    }

    fn settings(snippets: Vec<Snippet>) -> TextSettings {
        TextSettings {
            filter_hallucinations: true,
            snippets,
        }
    }

    #[test]
    fn exact_snippet_ignores_case_and_punctuation() {
        let s = settings(vec![snippet(
            "imzam",
            "Saygılarımla,\nEnver",
            SnippetMatch::Exact,
        )]);
        assert_eq!(process("İmzam.", &s), "Saygılarımla,\nEnver");
        assert_eq!(process("  IMZAM!  ", &s), "Saygılarımla,\nEnver");
    }

    #[test]
    fn exact_snippet_does_not_fire_inside_a_sentence() {
        let s = settings(vec![snippet("imzam", "SIG", SnippetMatch::Exact)]);
        assert_eq!(
            process("Mektubun sonuna imzam gelsin.", &s),
            "Mektubun sonuna imzam gelsin."
        );
    }

    #[test]
    fn inline_snippet_replaces_only_the_trigger_words() {
        let s = settings(vec![snippet(
            "mail adresim",
            "enver@example.com",
            SnippetMatch::Inline,
        )]);
        assert_eq!(
            process("Bana mail adresim üzerinden yaz.", &s),
            "Bana enver@example.com üzerinden yaz."
        );
    }

    #[test]
    fn inline_keeps_trailing_punctuation_after_trigger() {
        let s = settings(vec![snippet("imzam", "SIG", SnippetMatch::Inline)]);
        assert_eq!(process("Sonuna ekle: imzam.", &s), "Sonuna ekle: SIG.");
    }

    #[test]
    fn inline_does_not_match_inside_a_longer_word() {
        let s = settings(vec![snippet("imza", "SIG", SnippetMatch::Inline)]);
        assert_eq!(process("imzalar hazır", &s), "imzalar hazır");
    }

    #[test]
    fn longest_inline_trigger_wins() {
        let s = settings(vec![
            snippet("imzam", "SHORT", SnippetMatch::Inline),
            snippet("resmi imzam", "LONG", SnippetMatch::Inline),
        ]);
        assert_eq!(
            process("buraya resmi imzam gelsin", &s),
            "buraya LONG gelsin"
        );
    }

    #[test]
    fn expansion_is_not_rescanned_for_triggers() {
        let s = settings(vec![
            snippet("a b", "c d", SnippetMatch::Inline),
            snippet("c d", "LOOP", SnippetMatch::Inline),
        ]);
        assert_eq!(process("x a b y", &s), "x c d y");
    }

    #[test]
    fn disabled_snippet_is_ignored() {
        let mut snip = snippet("imzam", "SIG", SnippetMatch::Exact);
        snip.enabled = false;
        assert_eq!(process("imzam", &settings(vec![snip])), "imzam");
    }

    #[test]
    fn turkish_dotless_and_dotted_i_fold_together() {
        let s = settings(vec![snippet("ışık", "LIGHT", SnippetMatch::Exact)]);
        assert_eq!(process("IŞIK", &s), "LIGHT");
        let s = settings(vec![snippet("istanbul", "IST", SnippetMatch::Exact)]);
        assert_eq!(process("İstanbul.", &s), "IST");
    }

    #[test]
    fn whole_transcript_hallucination_is_dropped() {
        assert_eq!(process("Altyazı M.K.", &settings(vec![])), "");
        assert_eq!(
            process(" İzlediğiniz için teşekkürler. ", &settings(vec![])),
            ""
        );
        assert_eq!(process("Thanks for watching!", &settings(vec![])), "");
    }

    #[test]
    fn hallucination_words_inside_real_speech_are_kept() {
        let text = "Bu videoyu izlediğiniz için teşekkürler demeyi unutma.";
        assert_eq!(process(text, &settings(vec![])), text);
    }

    #[test]
    fn trailing_subtitle_credit_is_stripped() {
        assert_eq!(
            process(
                "Toplantı yarın saat üçte... Altyazı M.K.",
                &settings(vec![])
            ),
            "Toplantı yarın saat üçte..."
        );
    }

    #[test]
    fn filter_can_be_disabled() {
        let s = TextSettings {
            filter_hallucinations: false,
            snippets: vec![],
        };
        assert_eq!(process("Altyazı M.K.", &s), "Altyazı M.K.");
    }

    #[test]
    fn empty_and_whitespace_input_yield_empty() {
        assert_eq!(process("   ", &settings(vec![])), "");
        assert_eq!(process("...", &settings(vec![])), "");
    }
}
