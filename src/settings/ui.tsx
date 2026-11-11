/** Shared settings-window primitives. */

import { LoaderCircle } from "lucide-react";
import { motion } from "motion/react";
import {
  useEffect,
  useId,
  useState,
  type ButtonHTMLAttributes,
  type ComponentType,
  type KeyboardEvent,
  type ReactNode,
} from "react";

type IconType = ComponentType<{ className?: string; strokeWidth?: number }>;

export function Card({
  title,
  description,
  icon: Icon,
  actions,
  children,
}: {
  title?: string;
  description?: ReactNode;
  icon?: IconType;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <section className="rounded-2xl border border-white/[0.06] bg-gradient-to-b from-lapis-850/80 to-lapis-900/70 p-5 shadow-[0_1px_0_rgba(255,255,255,0.04)_inset,0_12px_32px_-18px_rgba(0,0,0,0.7)]">
      {(title ?? actions) !== undefined && (
        <header className="mb-4 flex items-start justify-between gap-4">
          <div className="flex items-start gap-3">
            {Icon && (
              <span className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg bg-gold-400/10 text-gold-300 ring-1 ring-gold-400/20">
                <Icon className="size-4" strokeWidth={2.2} />
              </span>
            )}
            <div>
              {title && <h3 className="font-display text-[15px] font-semibold tracking-tight text-ink-50">{title}</h3>}
              {description && <p className="mt-0.5 text-[13px] leading-relaxed text-ink-400">{description}</p>}
            </div>
          </div>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
      )}
      {children}
    </section>
  );
}

export function Row({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 border-t border-white/[0.05] py-3.5 first:border-t-0 first:pt-0 last:pb-0">
      <div className="min-w-0">
        <div className="text-[13.5px] font-medium text-ink-50">{label}</div>
        {hint && <div className="mt-0.5 text-[12.5px] leading-relaxed text-ink-400">{hint}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled = false,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative h-[26px] w-[46px] rounded-full p-[3px] transition-colors duration-300 disabled:opacity-40 ${
        checked ? "bg-gradient-to-r from-gold-500 to-gold-300 shadow-[0_0_14px_rgba(226,182,86,0.35)]" : "bg-white/10"
      }`}
    >
      <motion.span
        layout
        transition={{ type: "spring", stiffness: 700, damping: 34 }}
        className={`block size-5 rounded-full shadow-md ${checked ? "ml-auto bg-lapis-950" : "bg-ink-200"}`}
      />
    </button>
  );
}

type ButtonVariant = "primary" | "ghost" | "danger" | "subtle";

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    "bg-gradient-to-b from-gold-300 to-gold-500 text-lapis-950 shadow-[0_6px_18px_-8px_rgba(226,182,86,0.8)] hover:brightness-110",
  ghost: "border border-white/10 bg-white/[0.04] text-ink-50 hover:bg-white/[0.08]",
  subtle: "text-ink-200 hover:bg-white/[0.06] hover:text-ink-50",
  danger: "border border-rose-glow/30 bg-rose-glow/10 text-rose-glow hover:bg-rose-glow/20",
};

export function Button({
  variant = "ghost",
  icon: Icon,
  loading = false,
  children,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; icon?: IconType; loading?: boolean }) {
  return (
    <button
      type="button"
      {...rest}
      disabled={(rest.disabled ?? false) || loading}
      className={`inline-flex h-8 items-center gap-1.5 rounded-lg px-3 text-[13px] font-medium whitespace-nowrap transition-all duration-200 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-45 ${BUTTON_VARIANTS[variant]} ${className}`}
    >
      {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : Icon ? <Icon className="size-3.5" strokeWidth={2.3} /> : null}
      {children}
    </button>
  );
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  label,
  className = "",
}: {
  value: T;
  options: ReadonlyArray<{ value: T; label: string }>;
  onChange: (next: T) => void;
  label: string;
  className?: string;
}) {
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(event) => {
        const next = options.find((o) => o.value === event.target.value);
        if (next) onChange(next.value);
      }}
      className={`h-8 rounded-lg border border-white/10 bg-lapis-900 px-2.5 pr-8 text-[13px] text-ink-50 outline-none focus:border-gold-400/60 ${className}`}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

/** Range input that previews while dragging and commits on release. */
export function Slider({
  value,
  min,
  max,
  step,
  onCommit,
  format,
  label,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onCommit: (next: number) => void;
  format: (value: number) => string;
  label: string;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  const percent = ((draft - min) / (max - min)) * 100;
  return (
    <div className="flex w-[260px] items-center gap-3">
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={draft}
        onChange={(event) => setDraft(Number(event.target.value))}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
        className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full accent-gold-400"
        style={{
          background: `linear-gradient(90deg, var(--color-gold-400) ${percent}%, rgba(255,255,255,0.1) ${percent}%)`,
        }}
      />
      <span className="w-16 text-right font-mono text-[12px] text-gold-200 tabular-nums">{format(draft)}</span>
    </div>
  );
}

/** Text input with local draft state; commits on blur or Enter. */
export function TextField({
  value,
  onCommit,
  placeholder,
  type = "text",
  mono = false,
  className = "",
  label,
  multiline = false,
}: {
  value: string;
  onCommit: (next: string) => void;
  placeholder?: string;
  type?: "text" | "password" | "number";
  mono?: boolean;
  className?: string;
  label: string;
  multiline?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") event.currentTarget.blur();
    if (event.key === "Escape") {
      setDraft(value);
      event.currentTarget.blur();
    }
  };
  const base = `rounded-lg border border-white/10 bg-lapis-950/70 px-3 text-[13px] text-ink-50 placeholder:text-ink-500 outline-none transition-colors focus:border-gold-400/60 ${
    mono ? "font-mono text-[12.5px]" : ""
  } ${className}`;
  if (multiline) {
    return (
      <textarea
        aria-label={label}
        value={draft}
        placeholder={placeholder}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={commit}
        rows={3}
        className={`${base} w-full resize-y py-2 leading-relaxed`}
      />
    );
  }
  return (
    <input
      aria-label={label}
      type={type}
      value={draft}
      placeholder={placeholder}
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      onChange={(event) => setDraft(event.target.value)}
      onBlur={commit}
      onKeyDown={onKeyDown}
      className={`${base} h-8`}
    />
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: ReadonlyArray<{ value: T; label: string }>;
  onChange: (next: T) => void;
}) {
  const groupId = useId();
  return (
    <div className="inline-flex rounded-lg border border-white/10 bg-lapis-950/60 p-0.5">
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            onClick={() => onChange(option.value)}
            className={`relative h-7 rounded-md px-3 text-[12.5px] font-medium transition-colors ${
              active ? "text-lapis-950" : "text-ink-400 hover:text-ink-50"
            }`}
          >
            {active && (
              <motion.span
                layoutId={`seg-${groupId}`}
                className="absolute inset-0 rounded-md bg-gradient-to-b from-gold-300 to-gold-500"
                transition={{ type: "spring", stiffness: 600, damping: 38 }}
              />
            )}
            <span className="relative">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}

export type Tone = "ok" | "warn" | "error" | "busy" | "idle";

const TONE_CLASS: Record<Tone, string> = {
  ok: "bg-jade shadow-[0_0_8px_rgba(95,212,160,0.8)]",
  warn: "bg-gold-400 shadow-[0_0_8px_rgba(226,182,86,0.8)]",
  error: "bg-rose-glow shadow-[0_0_8px_rgba(255,107,107,0.8)]",
  busy: "bg-lapis-400 animate-pulse",
  idle: "bg-ink-500",
};

export function StatusDot({ tone }: { tone: Tone }) {
  return <span className={`inline-block size-2 shrink-0 rounded-full ${TONE_CLASS[tone]}`} />;
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-grid h-7 min-w-7 place-items-center rounded-md border border-white/15 border-b-white/25 bg-gradient-to-b from-lapis-800 to-lapis-900 px-2 font-sans text-[13px] font-semibold text-gold-200 shadow-[0_2px_0_rgba(0,0,0,0.4)]">
      {children}
    </kbd>
  );
}

export function SectionHeader({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div className="mb-6">
      <h1 className="font-display text-[26px] font-semibold tracking-tight text-ink-50">{title}</h1>
      <p className="mt-1 text-[13.5px] text-ink-400">{subtitle}</p>
    </div>
  );
}

export function ProgressBar({ value }: { value: number }) {
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-white/10">
      <motion.div
        className="h-full rounded-full bg-gradient-to-r from-gold-500 to-gold-200"
        initial={false}
        animate={{ width: `${Math.max(0, Math.min(100, value * 100))}%` }}
        transition={{ ease: "easeOut", duration: 0.25 }}
      />
    </div>
  );
}
