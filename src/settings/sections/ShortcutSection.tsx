import { Hand, Info, Keyboard, MousePointerClick } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";

import { acceleratorFromKeys, formatAccelerator, usesControlKey } from "../../lib/accelerator";
import type { Settings } from "../../lib/types";
import { Card, Kbd, Row, SectionHeader, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

export function ShortcutSection({ settings, update }: { settings: Settings; update: UpdateSettings }) {
  const shortcut = settings.shortcut;
  return (
    <>
      <SectionHeader title="Kısayol" subtitle="Mikrofonu klavyeden, butonla aynı mantıkla kontrol edin." />
      <div className="space-y-4">
        <Card title="Genel kısayol" icon={Keyboard} description="Hangi uygulamada olursanız olun çalışır.">
          <Row label="Kısayolu etkinleştir">
            <Toggle label="Kısayolu etkinleştir" checked={shortcut.enabled} onChange={(v) => void update((d) => void (d.shortcut.enabled = v))} />
          </Row>
          <Row label="Tuş kombinasyonu" hint="Değiştirmek için tıklayın ve yeni kombinasyona basın. Esc ile vazgeçin.">
            <ShortcutRecorder
              value={shortcut.accelerator}
              disabled={!shortcut.enabled}
              onChange={(accelerator) => void update((d) => void (d.shortcut.accelerator = accelerator))}
            />
          </Row>
          <AnimatePresence initial={false}>
            {shortcut.enabled && usesControlKey(shortcut.accelerator) && <ControlKeyNote />}
          </AnimatePresence>
        </Card>

        <div className="grid grid-cols-2 gap-4">
          <ModeCard
            icon={MousePointerClick}
            title="Dokun"
            text="Bir kez basıp bırakın: kayıt başlar. Tekrar basınca biter ve yazıya dökülür."
            chips={formatAccelerator(shortcut.accelerator)}
            suffix="× 2"
          />
          <ModeCard
            icon={Hand}
            title="Basılı tut"
            text="Basılı tuttuğunuz sürece dinler; bıraktığınız anda yazıya döker."
            chips={formatAccelerator(shortcut.accelerator)}
            suffix="⟶ bırak"
          />
        </div>
      </div>
    </>
  );
}

/** macOS treats Control+click as a right click; warn when the shortcut holds Control. */
function ControlKeyNote() {
  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }}
      animate={{ opacity: 1, height: "auto" }}
      exit={{ opacity: 0, height: 0 }}
      transition={{ duration: 0.25, ease: [0.16, 1, 0.3, 1] }}
      className="overflow-hidden"
    >
      <div className="mt-4 flex items-start gap-3 rounded-xl border border-gold-400/25 bg-gold-400/[0.07] p-3.5">
        <span className="mt-px grid size-6 shrink-0 place-items-center rounded-lg bg-gold-400/15 text-gold-300">
          <Info className="size-3.5" />
        </span>
        <p className="text-[12.5px] leading-relaxed text-ink-200">
          <span className="font-medium text-gold-200">Kısayolunuz ⌃ Control içeriyor.</span> macOS, Control basılıyken yapılan her tıklamayı
          sağ tık sayar: kısayolu basılı tutarken başka uygulamalarda tıklarsanız sağ tık menüsü açılabilir. Sedjem'in kendi düğmeleri
          bunu tanır ve normal tıklama gibi davranır. Sorun yaşarsanız ⌥ Option tabanlı bir kısayol daha rahat olur.
        </p>
      </div>
    </motion.div>
  );
}

function ModeCard({
  icon: Icon,
  title,
  text,
  chips,
  suffix,
}: {
  icon: typeof Hand;
  title: string;
  text: string;
  chips: string[];
  suffix: string;
}) {
  return (
    <div className="rounded-2xl border border-white/[0.06] bg-lapis-900/60 p-5">
      <div className="mb-3 flex items-center gap-2 text-gold-300">
        <Icon className="size-4" />
        <span className="font-display text-[14px] font-semibold text-ink-50">{title}</span>
      </div>
      <p className="mb-4 text-[12.5px] leading-relaxed text-ink-400">{text}</p>
      <div className="flex items-center gap-1.5">
        {chips.map((chip) => (
          <Kbd key={chip}>{chip}</Kbd>
        ))}
        <span className="ml-1 text-[12px] text-ink-400">{suffix}</span>
      </div>
    </div>
  );
}

function ShortcutRecorder({
  value,
  disabled,
  onChange,
}: {
  value: string;
  disabled: boolean;
  onChange: (accelerator: string) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [preview, setPreview] = useState<string[]>([]);
  const [hint, setHint] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.code === "Escape") {
        setRecording(false);
        return;
      }
      const result = acceleratorFromKeys(event);
      if (result.kind === "ok") {
        setRecording(false);
        setHint(null);
        if (result.accelerator !== value) onChange(result.accelerator);
        return;
      }
      const held = [event.metaKey && "⌘", event.ctrlKey && "⌃", event.altKey && "⌥", event.shiftKey && "⇧"].filter(
        (x): x is string => typeof x === "string",
      );
      setPreview(held);
      if (result.kind === "invalid") {
        setHint(result.reason === "needs-modifier" ? "En az bir ⌘, ⌃ veya ⌥ tuşu ekleyin." : "Bu tuş desteklenmiyor.");
      }
    };
    const stop = () => setRecording(false);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", stop);
    };
  }, [recording, value, onChange]);

  const chips = recording ? preview : formatAccelerator(value);

  return (
    <div className="flex flex-col items-end gap-1.5">
      <button
        type="button"
        disabled={disabled}
        onClick={() => {
          setPreview([]);
          setHint(null);
          setRecording((r) => !r);
        }}
        className={`flex h-10 min-w-[150px] items-center justify-center gap-1.5 rounded-xl border px-3 transition-all disabled:opacity-40 ${
          recording
            ? "border-gold-400/70 bg-gold-400/10 shadow-[0_0_0_4px_rgba(226,182,86,0.12)]"
            : "border-white/10 bg-lapis-950/60 hover:border-gold-400/40"
        }`}
      >
        <AnimatePresence mode="popLayout" initial={false}>
          {chips.length === 0 ? (
            <motion.span key="listening" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="text-[12.5px] text-gold-200">
              Kombinasyona basın…
            </motion.span>
          ) : (
            chips.map((chip, i) => (
              <motion.span key={`${chip}-${i}`} initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }}>
                <Kbd>{chip}</Kbd>
              </motion.span>
            ))
          )}
        </AnimatePresence>
      </button>
      {recording && hint && <span className="text-[11.5px] text-gold-300">{hint}</span>}
    </div>
  );
}
