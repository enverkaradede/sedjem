import { Clock, Copy, Trash } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";

import { formatClock } from "../../lib/format";
import { api, subscribe } from "../../lib/ipc";
import { toAppError, type HistoryEntry } from "../../lib/types";
import { useToasts } from "../toasts";
import { Button, SectionHeader } from "../ui";

export function HistorySection() {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const { push } = useToasts();

  const refresh = useCallback(() => {
    api.getHistory().then(setEntries).catch(() => setEntries([]));
  }, []);

  useEffect(() => {
    refresh();
    return subscribe("dictation:result", refresh);
  }, [refresh]);

  const copy = (text: string) =>
    api
      .copyToClipboard(text)
      .then(() => push("success", "Panoya kopyalandı."))
      .catch((err: unknown) => push("error", toAppError(err).message));

  const clear = () =>
    api
      .clearHistory()
      .then(refresh)
      .catch((err: unknown) => push("error", toAppError(err).message));

  return (
    <>
      <SectionHeader title="Geçmiş" subtitle="Son 20 dikte. Yalnızca bellekte durur; Sedjem kapanınca silinir." />
      {entries.length > 0 && (
        <div className="mb-4 flex justify-end">
          <Button variant="subtle" icon={Trash} onClick={() => void clear()}>
            Temizle
          </Button>
        </div>
      )}
      <div className="space-y-2.5">
        <AnimatePresence initial={false}>
          {entries.map((entry) => (
            <motion.div
              key={entry.id}
              layout
              initial={{ opacity: 0, y: -8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="group flex items-start gap-4 rounded-2xl border border-white/[0.06] bg-lapis-900/60 p-4"
            >
              <div className="w-12 shrink-0 pt-0.5 font-mono text-[11.5px] text-ink-500">
                {formatClock(entry.createdAtMs)}
                {entry.language && <div className="mt-1 text-[10.5px] tracking-wide text-gold-300/70 uppercase">{entry.language}</div>}
              </div>
              <p className="flex-1 text-[13.5px] leading-relaxed whitespace-pre-wrap text-ink-50">{entry.text}</p>
              <Button
                variant="subtle"
                aria-label="Kopyala"
                icon={Copy}
                className="opacity-0 transition-opacity group-hover:opacity-100"
                onClick={() => void copy(entry.text)}
              />
            </motion.div>
          ))}
        </AnimatePresence>
        {entries.length === 0 && (
          <div className="flex flex-col items-center rounded-2xl border border-dashed border-white/10 py-14 text-center">
            <Clock className="mb-3 size-6 text-ink-500" />
            <p className="text-[13px] text-ink-400">Henüz dikte yok. Mikrofona basıp konuşmaya başlayın.</p>
          </div>
        )}
      </div>
    </>
  );
}
