import { AudioLines, Check, Lock, Mic, MicOff, Settings2, TriangleAlert, X } from "lucide-react";
import { AnimatePresence, motion, useMotionValue, useSpring, useTransform, type MotionValue } from "motion/react";
import { useCallback, useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";

import { api, subscribe } from "../lib/ipc";
import type { DictationState } from "../lib/types";
import { useWindowDrag } from "./useWindowDrag";

type Flash = "success" | "empty" | "error" | "busy";

const FLASH_MS: Record<Flash, number> = { success: 1100, empty: 900, error: 2600, busy: 450 };
const LEVEL_BARS = 28;
const IDLE_STATE: DictationState = { phase: "idle", latch: null, source: null };
const SHAKE = { x: [0, -5, 5, -4, 4, -2, 0] };

/** The floating microphone: press = start, quick release = toggle, hold = push-to-talk. */
export function PanelApp() {
  const [state, setState] = useState<DictationState>(IDLE_STATE);
  const [flash, setFlash] = useState<Flash | null>(null);
  const [hovered, setHovered] = useState(false);
  const flashTimer = useRef<number | undefined>(undefined);
  const pressChain = useRef<Promise<void>>(Promise.resolve());
  const pressed = useRef(false);

  const level = useMotionValue(0);
  const smoothLevel = useSpring(level, { stiffness: 420, damping: 30, mass: 0.5 });
  const auraScale = useTransform(smoothLevel, [0, 1], [1, 1.5]);
  const auraOpacity = useTransform(smoothLevel, [0, 1], [0.35, 0.95]);
  const barsScale = useTransform(smoothLevel, [0, 0.15, 1], [0.45, 0.8, 1.12]);

  const showFlash = useCallback((kind: Flash) => {
    window.clearTimeout(flashTimer.current);
    setFlash(kind);
    flashTimer.current = window.setTimeout(() => setFlash(null), FLASH_MS[kind]);
  }, []);

  useEffect(() => {
    api.getDictationState().then(setState).catch((err: unknown) => console.error("initial state", err));
    const disposers = [
      subscribe("dictation:state", (next) => {
        setState(next);
        if (next.phase !== "recording") level.set(0);
      }),
      subscribe("dictation:level", (value) => level.set(value)),
      subscribe("dictation:result", (result) => showFlash(result.inserted ? "success" : "error")),
      subscribe("dictation:no-speech", () => showFlash("empty")),
      subscribe("dictation:busy", () => showFlash("busy")),
      subscribe("dictation:error", () => showFlash("error")),
    ];
    return () => {
      disposers.forEach((dispose) => dispose());
      window.clearTimeout(flashTimer.current);
    };
  }, [level, showFlash]);

  // Release is chained after press so a fast tap can never reorder over IPC.
  const onPointerDown = (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    pressed.current = true;
    pressChain.current = api.panelPress().catch((err: unknown) => console.error("press failed", err));
  };
  const endPress = () => {
    if (!pressed.current) return;
    pressed.current = false;
    pressChain.current = pressChain.current
      .then(() => api.panelRelease())
      .catch((err: unknown) => console.error("release failed", err));
  };

  const recording = state.phase === "recording";
  const processing = state.phase === "processing";
  const toggled = recording && state.latch === "toggle";
  const visual: Flash | "recording" | "processing" | "idle" = flash ?? (recording ? "recording" : processing ? "processing" : "idle");

  return (
    <div
      className="relative grid h-screen w-screen place-items-center"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {/* Voice aura and ripples while recording */}
      <AnimatePresence>
        {recording && (
          <motion.div
            key="aura"
            className="pointer-events-none absolute size-16 rounded-full"
            style={{
              scale: auraScale,
              opacity: auraOpacity,
              background: "radial-gradient(circle, rgba(242,210,131,0.55) 0%, rgba(226,182,86,0.18) 55%, transparent 72%)",
            }}
            initial={{ opacity: 0 }}
            exit={{ opacity: 0, transition: { duration: 0.25 } }}
          />
        )}
      </AnimatePresence>
      {recording && (
        <>
          <span className="pointer-events-none absolute size-16 rounded-full border border-gold-300/50 [animation:ripple_1.8s_var(--ease-out-expo)_infinite]" />
          <span className="pointer-events-none absolute size-16 rounded-full border border-gold-300/40 [animation:ripple_1.8s_var(--ease-out-expo)_0.9s_infinite]" />
          <LevelBars scale={barsScale} />
        </>
      )}

      {/* Processing orbit */}
      <AnimatePresence>
        {processing && (
          <motion.div
            key="orbit"
            className="pointer-events-none absolute size-[78px] rounded-full [animation:spin-slow_1.1s_linear_infinite]"
            style={{
              background: "conic-gradient(from 0deg, transparent 0%, transparent 55%, #5a85e0 78%, #f2d283 96%, transparent 100%)",
              WebkitMask: "radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 2.5px))",
              mask: "radial-gradient(farthest-side, transparent calc(100% - 3px), #000 calc(100% - 2.5px))",
            }}
            initial={{ opacity: 0, scale: 0.9 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 1.1 }}
          />
        )}
      </AnimatePresence>

      <motion.button
        type="button"
        aria-label={recording ? "Kaydı bitir" : "Konuşmaya başla"}
        className="relative z-10 grid size-16 cursor-pointer place-items-center rounded-full outline-none"
        onPointerDown={onPointerDown}
        onPointerUp={endPress}
        onPointerCancel={endPress}
        onLostPointerCapture={endPress}
        onContextMenu={(event) => event.preventDefault()}
        whileHover={{ scale: 1.06 }}
        whileTap={{ scale: 0.93 }}
        animate={visual === "error" || visual === "empty" || visual === "busy" ? SHAKE : { x: 0 }}
        transition={{ type: "spring", stiffness: 500, damping: 26 }}
      >
        <ButtonFace visual={visual} />
        <span className="relative z-10 grid place-items-center">
          <AnimatePresence mode="popLayout" initial={false}>
            <motion.span
              key={visual}
              initial={{ opacity: 0, scale: 0.5, rotate: -30 }}
              animate={{ opacity: 1, scale: 1, rotate: 0 }}
              exit={{ opacity: 0, scale: 0.5, rotate: 30 }}
              transition={{ type: "spring", stiffness: 520, damping: 28 }}
              className="grid place-items-center"
            >
              <FaceIcon visual={visual} />
            </motion.span>
          </AnimatePresence>
        </span>
        <AnimatePresence>
          {toggled && (
            <motion.span
              key="lock"
              className="absolute -right-0.5 -bottom-0.5 z-20 grid size-5 place-items-center rounded-full bg-lapis-900 ring-1 ring-gold-400/70"
              initial={{ scale: 0 }}
              animate={{ scale: 1 }}
              exit={{ scale: 0 }}
            >
              <Lock className="size-2.5 text-gold-300" strokeWidth={2.6} />
            </motion.span>
          )}
        </AnimatePresence>
      </motion.button>

      <DragHandle emphasized={hovered} />

      {/* Hover chips: settings always, cancel while recording */}
      <AnimatePresence>
        {(hovered || flash === "error") && (
          <Chip key="settings" position="right-[14px] top-[14px]" label="Ayarlar" alert={flash === "error"} onClick={() => void api.showSettings()}>
            <Settings2 className="size-3" strokeWidth={2.4} />
          </Chip>
        )}
        {hovered && recording && (
          <Chip key="cancel" position="left-[14px] top-[14px]" label="İptal" onClick={() => void api.cancelDictation()}>
            <X className="size-3" strokeWidth={2.6} />
          </Chip>
        )}
      </AnimatePresence>
    </div>
  );
}

function ButtonFace({ visual }: { visual: string }) {
  const faces: Record<string, string> = {
    idle: "bg-[radial-gradient(circle_at_30%_25%,#1a3a86_0%,#0b1a3e_55%,#050b1f_100%)] ring-1 ring-gold-400/35 shadow-[0_8px_24px_rgba(0,0,0,0.55),inset_0_1px_0_rgba(255,255,255,0.08)]",
    recording: "bg-[radial-gradient(circle_at_30%_25%,#fbe7a6_0%,#e2b656_50%,#a37a26_100%)] ring-1 ring-gold-200/80 shadow-[0_0_28px_rgba(242,210,131,0.55),0_8px_22px_rgba(0,0,0,0.5)]",
    processing: "bg-[radial-gradient(circle_at_30%_25%,#173275_0%,#0b1a3e_60%,#050b1f_100%)] ring-1 ring-lapis-400/50 shadow-[0_0_22px_rgba(90,133,224,0.35),0_8px_22px_rgba(0,0,0,0.5)]",
    success: "bg-[radial-gradient(circle_at_30%_25%,#a8f0cc_0%,#5fd4a0_55%,#2f9b6c_100%)] ring-1 ring-white/40 shadow-[0_0_26px_rgba(95,212,160,0.55)]",
    empty: "bg-[radial-gradient(circle_at_30%_25%,#2a3354_0%,#151c33_70%)] ring-1 ring-ink-500/50 shadow-[0_8px_22px_rgba(0,0,0,0.5)]",
    busy: "bg-[radial-gradient(circle_at_30%_25%,#173275_0%,#0b1a3e_60%,#050b1f_100%)] ring-1 ring-lapis-400/50",
    error: "bg-[radial-gradient(circle_at_30%_25%,#ff9a9a_0%,#ff6b6b_50%,#b33a3a_100%)] ring-1 ring-white/40 shadow-[0_0_26px_rgba(255,107,107,0.5)]",
  };
  return (
    <motion.span
      layout
      className={`absolute inset-0 rounded-full transition-[background,box-shadow] duration-300 ${faces[visual] ?? faces.idle}`}
    />
  );
}

function FaceIcon({ visual }: { visual: string }) {
  const common = { className: "size-6", strokeWidth: 2.2 } as const;
  switch (visual) {
    case "recording":
      return <Mic {...common} className="size-6 text-lapis-950" />;
    case "processing":
    case "busy":
      return <AudioLines {...common} className="size-6 animate-pulse text-gold-300" />;
    case "success":
      return <Check {...common} className="size-6 text-lapis-950" strokeWidth={3} />;
    case "empty":
      return <MicOff {...common} className="size-6 text-ink-400" />;
    case "error":
      return <TriangleAlert {...common} className="size-6 text-white" />;
    default:
      return <Mic {...common} className="size-6 text-gold-300 drop-shadow-[0_0_6px_rgba(242,210,131,0.45)]" />;
  }
}

/** Radial equaliser whose reach follows the voice level. */
function LevelBars({ scale }: { scale: MotionValue<number> }) {
  return (
    <motion.div className="pointer-events-none absolute size-[104px]" style={{ scale }}>
      {Array.from({ length: LEVEL_BARS }, (_, i) => {
        const angle = (360 / LEVEL_BARS) * i;
        // Deterministic per-bar variety so the ring never pulses in lockstep.
        const duration = 0.55 + ((i * 37) % 9) / 16;
        return (
          // Zero-size pivot at the centre; rotating it fans the bars out.
          <span key={angle} className="absolute top-1/2 left-1/2 size-0" style={{ transform: `rotate(${angle}deg)` }}>
            <span
              className="absolute top-[-48px] left-[-1.5px] block h-2.5 w-[3px] origin-bottom rounded-full bg-gradient-to-t from-gold-500/50 to-gold-200"
              style={{ animation: `bar-wobble ${duration}s ease-in-out ${(i % 7) * 0.07}s infinite` }}
            />
          </span>
        );
      })}
    </motion.div>
  );
}

/**
 * Always-visible grip under the button. It must be painted (non-zero alpha):
 * macOS passes clicks on fully transparent pixels of a borderless window
 * through to the app below, which is why the empty margin cannot be grabbed.
 */
function DragHandle({ emphasized }: { emphasized: boolean }) {
  const handlers = useWindowDrag();
  return (
    <motion.div
      {...handlers}
      role="button"
      aria-label="Taşı"
      title="Sürükleyerek taşı"
      className="absolute bottom-[7px] left-1/2 z-30 flex h-[14px] w-9 -translate-x-1/2 cursor-grab items-center justify-center gap-[3px] rounded-full border border-white/10 bg-lapis-900/80 backdrop-blur-md active:cursor-grabbing"
      animate={{ opacity: emphasized ? 1 : 0.55, scale: emphasized ? 1.08 : 1 }}
      transition={{ type: "spring", stiffness: 500, damping: 30 }}
    >
      {[0, 1, 2].map((dot) => (
        <span key={dot} className="pointer-events-none size-[3px] rounded-full bg-gold-300/80" />
      ))}
    </motion.div>
  );
}

interface ChipProps {
  position: string;
  label: string;
  alert?: boolean;
  onClick: () => void;
  children: ReactNode;
}

function Chip({ position, label, alert = false, onClick, children }: ChipProps) {
  return (
    <motion.button
      type="button"
      aria-label={label}
      title={label}
      className={`absolute ${position} z-30 grid size-[22px] place-items-center rounded-full border backdrop-blur-md ${
        alert ? "border-rose-glow/70 bg-rose-glow/90 text-white" : "border-white/15 bg-lapis-900/85 text-ink-200 hover:text-gold-200"
      }`}
      initial={{ opacity: 0, scale: 0.6 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.6 }}
      whileHover={{ scale: 1.12 }}
      onPointerDown={(event) => event.stopPropagation()}
      onClick={onClick}
    >
      {children}
    </motion.button>
  );
}
