import { Pencil, Plus, Search, Sparkles, Trash } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useMemo, useState } from "react";

import { newId } from "../../lib/format";
import type { Settings, Snippet, SnippetMatch } from "../../lib/types";
import { Button, SectionHeader, Segmented, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

const MATCH_OPTIONS = [
  { value: "exact", label: "Tam eşleşme" },
  { value: "inline", label: "Cümle içinde" },
] as const satisfies ReadonlyArray<{ value: SnippetMatch; label: string }>;

const MATCH_HELP: Record<SnippetMatch, string> = {
  exact: "Yalnızca tetikleyiciyi tek başına söylediğinizde metnin tamamı bununla değişir.",
  inline: "Tetikleyici cümlenin neresinde geçerse geçsin, sadece o kısım değişir.",
};

export function SnippetsSection({ settings, update }: { settings: Settings; update: UpdateSettings }) {
  const snippets = settings.text.snippets;
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Snippet | null>(null);
  const isNew = editing !== null && !snippets.some((s) => s.id === editing.id);

  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase("tr");
    if (!needle) return snippets;
    return snippets.filter(
      (s) => s.trigger.toLocaleLowerCase("tr").includes(needle) || s.expansion.toLocaleLowerCase("tr").includes(needle),
    );
  }, [snippets, query]);

  const save = async (snippet: Snippet) => {
    const ok = await update((d) => {
      const index = d.text.snippets.findIndex((s) => s.id === snippet.id);
      if (index >= 0) d.text.snippets[index] = snippet;
      else d.text.snippets.unshift(snippet);
    });
    if (ok) setEditing(null);
  };

  const startNew = () =>
    setEditing({ id: newId(), trigger: "", expansion: "", matchMode: "exact", enabled: true });

  return (
    <>
      <SectionHeader
        title="Snippet'ler"
        subtitle="Bir kelime grubunu söyleyin, hazır bir metin yazılsın. Büyük/küçük harf ve noktalama önemsizdir."
      />
      <div className="mb-4 flex items-center gap-3">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-ink-500" />
          <input
            aria-label="Snippet ara"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Ara…"
            className="h-9 w-full rounded-xl border border-white/10 bg-lapis-900/70 pr-3 pl-9 text-[13px] text-ink-50 outline-none placeholder:text-ink-500 focus:border-gold-400/60"
          />
        </div>
        <Button variant="primary" icon={Plus} onClick={startNew} disabled={isNew}>
          Yeni snippet
        </Button>
      </div>

      <div className="space-y-2.5">
        <AnimatePresence initial={false}>
          {isNew && editing && (
            <SnippetEditor key={editing.id} initial={editing} onCancel={() => setEditing(null)} onSave={(s) => void save(s)} />
          )}
          {visible.map((snippet) =>
            editing?.id === snippet.id ? (
              <SnippetEditor key={snippet.id} initial={editing} onCancel={() => setEditing(null)} onSave={(s) => void save(s)} />
            ) : (
              <SnippetRow
                key={snippet.id}
                snippet={snippet}
                onEdit={() => setEditing(snippet)}
                onToggle={(enabled) =>
                  void update((d) => {
                    const target = d.text.snippets.find((s) => s.id === snippet.id);
                    if (target) target.enabled = enabled;
                  })
                }
                onDelete={() => void update((d) => void (d.text.snippets = d.text.snippets.filter((s) => s.id !== snippet.id)))}
              />
            ),
          )}
        </AnimatePresence>
        {snippets.length === 0 && !isNew && <EmptyState onCreate={startNew} />}
        {snippets.length > 0 && visible.length === 0 && (
          <p className="py-8 text-center text-[13px] text-ink-400">“{query}” ile eşleşen snippet yok.</p>
        )}
      </div>
    </>
  );
}

function SnippetRow({
  snippet,
  onEdit,
  onToggle,
  onDelete,
}: {
  snippet: Snippet;
  onEdit: () => void;
  onToggle: (enabled: boolean) => void;
  onDelete: () => void;
}) {
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.98 }}
      className={`group flex items-center gap-4 rounded-2xl border border-white/[0.06] bg-lapis-900/60 p-4 transition-opacity ${
        snippet.enabled ? "" : "opacity-55"
      }`}
    >
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-display text-[14px] font-semibold text-gold-200">“{snippet.trigger}”</span>
          <span className="rounded-full bg-white/[0.06] px-2 py-px text-[10.5px] text-ink-400">
            {snippet.matchMode === "exact" ? "tam" : "cümle içi"}
          </span>
        </div>
        <p className="mt-1 truncate text-[12.5px] text-ink-400">{snippet.expansion.replaceAll("\n", " ⏎ ") || "(boş metin)"}</p>
      </div>
      <div className="flex items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
        <Button variant="subtle" aria-label="Düzenle" icon={Pencil} onClick={onEdit} />
        <Button variant="subtle" aria-label="Sil" icon={Trash} onClick={onDelete} />
      </div>
      <Toggle label="Etkin" checked={snippet.enabled} onChange={onToggle} />
    </motion.div>
  );
}

function SnippetEditor({
  initial,
  onSave,
  onCancel,
}: {
  initial: Snippet;
  onSave: (snippet: Snippet) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const valid = /[\p{L}\p{N}]/u.test(draft.trigger);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: -6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -6 }}
      className="rounded-2xl border border-gold-400/35 bg-gradient-to-b from-lapis-850 to-lapis-900 p-5 shadow-[0_0_0_4px_rgba(226,182,86,0.06)]"
    >
      <label className="block text-[12px] font-medium text-ink-400">
        Söylediğiniz
        <input
          autoFocus
          value={draft.trigger}
          maxLength={100}
          onChange={(event) => setDraft({ ...draft, trigger: event.target.value })}
          placeholder="ör. imzam"
          className="mt-1.5 h-9 w-full rounded-lg border border-white/10 bg-lapis-950/70 px-3 text-[14px] text-ink-50 outline-none placeholder:text-ink-500 focus:border-gold-400/60"
        />
      </label>
      <label className="mt-4 block text-[12px] font-medium text-ink-400">
        Yazılacak metin
        <textarea
          value={draft.expansion}
          maxLength={10_000}
          rows={4}
          onChange={(event) => setDraft({ ...draft, expansion: event.target.value })}
          placeholder={"Saygılarımla,\nEnver"}
          className="mt-1.5 w-full resize-y rounded-lg border border-white/10 bg-lapis-950/70 px-3 py-2 text-[13px] leading-relaxed text-ink-50 outline-none placeholder:text-ink-500 focus:border-gold-400/60"
        />
      </label>
      <div className="mt-4 flex items-center justify-between gap-4">
        <div>
          <Segmented value={draft.matchMode} options={MATCH_OPTIONS} onChange={(matchMode) => setDraft({ ...draft, matchMode })} />
          <p className="mt-2 max-w-[360px] text-[12px] text-ink-400">{MATCH_HELP[draft.matchMode]}</p>
        </div>
        <div className="flex gap-2">
          <Button variant="subtle" onClick={onCancel}>
            Vazgeç
          </Button>
          <Button variant="primary" disabled={!valid} onClick={() => onSave({ ...draft, trigger: draft.trigger.trim() })}>
            Kaydet
          </Button>
        </div>
      </div>
    </motion.div>
  );
}

function EmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex flex-col items-center rounded-2xl border border-dashed border-white/10 py-14 text-center">
      <span className="mb-4 grid size-12 place-items-center rounded-2xl bg-gold-400/10 text-gold-300 ring-1 ring-gold-400/25">
        <Sparkles className="size-5" />
      </span>
      <p className="font-display text-[15px] font-semibold text-ink-50">Henüz snippet yok</p>
      <p className="mt-1 max-w-[340px] text-[13px] text-ink-400">Örneğin “imzam” dediğinizde e-posta imzanızın tamamı yazılsın.</p>
      <Button className="mt-5" variant="primary" icon={Plus} onClick={onCreate}>
        İlk snippet'i oluştur
      </Button>
    </div>
  );
}
