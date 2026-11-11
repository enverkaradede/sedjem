/** Lightweight toast notifications for the settings window. */

import { CircleAlert, CircleCheck, Info, X } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode } from "react";

type ToastKind = "success" | "error" | "info";

interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

interface ToastApi {
  push: (kind: ToastKind, message: string) => void;
}

const DISMISS_MS: Record<ToastKind, number> = { success: 2600, info: 4200, error: 6500 };
const MAX_VISIBLE = 4;

const ToastContext = createContext<ToastApi | null>(null);

export function useToasts(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToasts must be used inside <ToastProvider>");
  return ctx;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => setToasts((all) => all.filter((t) => t.id !== id)), []);

  const push = useCallback(
    (kind: ToastKind, message: string) => {
      const id = nextId.current++;
      setToasts((all) => [...all.slice(-(MAX_VISIBLE - 1)), { id, kind, message }]);
      window.setTimeout(() => dismiss(id), DISMISS_MS[kind]);
    },
    [dismiss],
  );

  const api = useMemo(() => ({ push }), [push]);

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="pointer-events-none fixed right-5 bottom-5 z-50 flex w-[360px] flex-col gap-2">
        <AnimatePresence initial={false}>
          {toasts.map((toast) => (
            <motion.div
              key={toast.id}
              layout
              initial={{ opacity: 0, y: 16, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, x: 40, transition: { duration: 0.2 } }}
              transition={{ type: "spring", stiffness: 500, damping: 34 }}
              className="pointer-events-auto flex items-start gap-3 rounded-xl border border-white/10 bg-lapis-850/95 p-3.5 shadow-[0_18px_40px_-12px_rgba(0,0,0,0.8)] backdrop-blur-xl"
            >
              <ToastIcon kind={toast.kind} />
              <p className="flex-1 text-[13px] leading-relaxed text-ink-50">{toast.message}</p>
              <button
                type="button"
                aria-label="Kapat"
                onClick={() => dismiss(toast.id)}
                className="text-ink-500 transition-colors hover:text-ink-50"
              >
                <X className="size-3.5" />
              </button>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </ToastContext.Provider>
  );
}

function ToastIcon({ kind }: { kind: ToastKind }) {
  if (kind === "success") return <CircleCheck className="mt-0.5 size-4 shrink-0 text-jade" />;
  if (kind === "error") return <CircleAlert className="mt-0.5 size-4 shrink-0 text-rose-glow" />;
  return <Info className="mt-0.5 size-4 shrink-0 text-lapis-300" />;
}
