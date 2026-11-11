import { ArrowUpCircle, Clock, Cpu, Keyboard, LoaderCircle, Mic, ShieldCheck, SlidersHorizontal, Sparkles, TriangleAlert } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState, type ComponentType } from "react";

import { api, subscribe } from "../lib/ipc";
import type { AppInfo, EngineStatus, Permissions, UpdateInfo } from "../lib/types";
import { EngineSection } from "./sections/EngineSection";
import { GeneralSection } from "./sections/GeneralSection";
import { HistorySection } from "./sections/HistorySection";
import { PermissionsSection } from "./sections/PermissionsSection";
import { RecordingSection } from "./sections/RecordingSection";
import { ShortcutSection } from "./sections/ShortcutSection";
import { SnippetsSection } from "./sections/SnippetsSection";
import { UpdatesSection } from "./sections/UpdatesSection";
import { ToastProvider, useToasts } from "./toasts";
import { StatusDot, type Tone } from "./ui";
import { useSettings } from "./useSettings";

type SectionId = "general" | "shortcut" | "engine" | "snippets" | "recording" | "history" | "permissions" | "updates";

const NAV: ReadonlyArray<{ id: SectionId; label: string; icon: ComponentType<{ className?: string }> }> = [
  { id: "general", label: "Genel", icon: SlidersHorizontal },
  { id: "shortcut", label: "Kısayol", icon: Keyboard },
  { id: "engine", label: "Motor & Model", icon: Cpu },
  { id: "snippets", label: "Snippet'ler", icon: Sparkles },
  { id: "recording", label: "Kayıt", icon: Mic },
  { id: "history", label: "Geçmiş", icon: Clock },
  { id: "permissions", label: "İzinler", icon: ShieldCheck },
  { id: "updates", label: "Güncellemeler", icon: ArrowUpCircle },
];

function isSectionId(value: string): value is SectionId {
  return NAV.some((item) => item.id === value);
}

const PERMISSION_POLL_MS = 2_500;

export function SettingsApp() {
  return (
    <ToastProvider>
      <SettingsShell />
    </ToastProvider>
  );
}

function SettingsShell() {
  const { settings, update } = useSettings();
  const { push } = useToasts();
  const [section, setSection] = useState<SectionId>("general");
  const [engineStatus, setEngineStatus] = useState<EngineStatus | null>(null);
  const [permissions, setPermissions] = useState<Permissions | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);

  useEffect(() => {
    api.getEngineStatus().then(setEngineStatus).catch(() => undefined);
    api.getAppInfo().then(setInfo).catch(() => undefined);
    api.getUpdateInfo().then(setUpdateInfo).catch(() => undefined);
    api
      .takeStartupNotices()
      .then((notices) => notices.forEach((notice) => push("info", notice)))
      .catch(() => undefined);
    const disposers = [
      subscribe("engine:status", setEngineStatus),
      subscribe("dictation:error", (err) => push("error", err.message)),
      subscribe("update:state", setUpdateInfo),
      subscribe("ui:navigate", (target) => {
        if (isSectionId(target)) setSection(target);
      }),
    ];
    return () => disposers.forEach((dispose) => dispose());
  }, [push]);

  // Permissions change outside the app (System Settings), so poll gently.
  useEffect(() => {
    const refresh = () => api.getPermissions().then(setPermissions).catch(() => undefined);
    void refresh();
    const timer = window.setInterval(refresh, PERMISSION_POLL_MS);
    return () => window.clearInterval(timer);
  }, []);

  const needsAccessibility = permissions !== null && !permissions.accessibility;

  return (
    <div className="relative flex h-full overflow-hidden">
      <Backdrop />
      <aside className="relative z-10 flex w-[236px] shrink-0 flex-col border-r border-white/[0.06] bg-lapis-900/60 backdrop-blur-2xl">
        <div data-tauri-drag-region className="h-[54px] shrink-0" />
        <Brand />
        <nav className="mt-5 flex flex-col gap-0.5 px-3">
          {NAV.map((item) => (
            <NavButton
              key={item.id}
              item={item}
              active={section === item.id}
              badge={(item.id === "permissions" && needsAccessibility) || (item.id === "updates" && updateInfo?.state === "available")}
              onSelect={() => setSection(item.id)}
            />
          ))}
        </nav>
        <div className="mt-auto space-y-3 p-4">
          <EnginePill status={engineStatus} onClick={() => setSection("engine")} />
          <p className="px-1 text-[11px] text-ink-500">Sedjem {info?.version ?? ""} · yerel, çevrimdışı</p>
        </div>
      </aside>

      <main className="scroll-quiet relative z-10 flex-1 overflow-y-auto">
        <div data-tauri-drag-region className="sticky top-0 z-20 h-[54px]" />
        <div className="mx-auto max-w-[720px] px-10 pb-16">
          <AnimatePresence>
            {needsAccessibility && section !== "permissions" && (
              <motion.button
                type="button"
                initial={{ opacity: 0, y: -8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -8 }}
                onClick={() => setSection("permissions")}
                className="mb-6 flex w-full items-center gap-3 rounded-xl border border-gold-400/30 bg-gold-400/10 p-3.5 text-left text-[13px] text-gold-200 transition-colors hover:bg-gold-400/15"
              >
                <TriangleAlert className="size-4 shrink-0" />
                Metni yazdığınız uygulamaya yapıştırabilmek için Erişilebilirlik izni gerekli. Vermek için tıklayın.
              </motion.button>
            )}
          </AnimatePresence>

          {settings === null ? (
            <div className="grid h-64 place-items-center text-ink-400">
              <LoaderCircle className="size-5 animate-spin" />
            </div>
          ) : (
            <AnimatePresence mode="wait">
              <motion.div
                key={section}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -6 }}
                transition={{ duration: 0.22, ease: [0.16, 1, 0.3, 1] }}
              >
                {section === "general" && <GeneralSection settings={settings} update={update} />}
                {section === "shortcut" && <ShortcutSection settings={settings} update={update} />}
                {section === "engine" && <EngineSection settings={settings} update={update} status={engineStatus} />}
                {section === "snippets" && <SnippetsSection settings={settings} update={update} />}
                {section === "recording" && <RecordingSection settings={settings} update={update} />}
                {section === "history" && <HistorySection />}
                {section === "permissions" && <PermissionsSection permissions={permissions} onRefresh={setPermissions} />}
                {section === "updates" && <UpdatesSection settings={settings} update={update} info={updateInfo} />}
              </motion.div>
            </AnimatePresence>
          )}
        </div>
      </main>
    </div>
  );
}

function Backdrop() {
  return (
    <div className="pointer-events-none absolute inset-0">
      <div className="absolute -top-40 right-[-10%] size-[620px] rounded-full bg-lapis-600/25 blur-[120px]" />
      <div className="absolute bottom-[-30%] left-[10%] size-[520px] rounded-full bg-gold-500/[0.07] blur-[120px]" />
    </div>
  );
}

function Brand() {
  return (
    <div className="flex items-center gap-3 px-5">
      <div className="relative grid size-10 place-items-center rounded-xl bg-gradient-to-br from-lapis-600 to-lapis-950 ring-1 ring-gold-400/30 shadow-[0_8px_20px_-8px_rgba(52,97,196,0.8)]">
        <svg viewBox="0 0 1024 1024" className="size-7" aria-hidden="true">
          <defs>
            <linearGradient id="brand-gold" x1="0" y1="0" x2="0.4" y2="1">
              <stop offset="0" stopColor="#FBE7A6" />
              <stop offset="1" stopColor="#C99A3A" />
            </linearGradient>
          </defs>
          <g fill="none" stroke="url(#brand-gold)" strokeLinecap="round">
            <path d="M548 232 C 700 232 790 344 790 466 C 790 562 732 614 690 660 C 646 706 666 766 608 804 C 550 842 474 822 456 760" strokeWidth="70" />
            <path d="M498 462 C 498 390 548 352 598 352 C 660 352 696 402 696 462 C 696 524 636 540 622 590" strokeWidth="56" />
            <path d="M372 410 C 340 446 340 526 372 562" strokeWidth="52" />
            <path d="M262 340 C 206 408 206 578 262 646" strokeWidth="48" opacity="0.6" />
          </g>
        </svg>
      </div>
      <div>
        <div className="font-display text-[17px] font-semibold tracking-tight text-ink-50">Sedjem</div>
        <div className="text-[11.5px] text-gold-300/80">İşitmenin tanrısı</div>
      </div>
    </div>
  );
}

function NavButton({
  item,
  active,
  badge,
  onSelect,
}: {
  item: (typeof NAV)[number];
  active: boolean;
  badge: boolean;
  onSelect: () => void;
}) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`relative flex h-9 items-center gap-2.5 rounded-lg px-3 text-[13.5px] transition-colors ${
        active ? "text-ink-50" : "text-ink-400 hover:bg-white/[0.04] hover:text-ink-50"
      }`}
    >
      {active && (
        <motion.span
          layoutId="nav-active"
          className="absolute inset-0 rounded-lg bg-gradient-to-r from-gold-400/[0.16] to-gold-400/[0.04] ring-1 ring-gold-400/25"
          transition={{ type: "spring", stiffness: 500, damping: 38 }}
        />
      )}
      <Icon className={`relative size-4 ${active ? "text-gold-300" : ""}`} />
      <span className="relative">{item.label}</span>
      {badge && <span className="relative ml-auto size-2 rounded-full bg-gold-400 shadow-[0_0_8px_rgba(226,182,86,0.9)]" />}
    </button>
  );
}

const ENGINE_NAMES: Record<string, string> = {
  mlx: "Yerel MLX",
  whisperCpp: "whisper.cpp",
  wyoming: "Wyoming",
  openai: "HTTP",
};

function EnginePill({ status, onClick }: { status: EngineStatus | null; onClick: () => void }) {
  const tone: Tone = status === null ? "idle" : status.state === "ready" ? "ok" : status.state === "loading" ? "busy" : "error";
  const text =
    status === null
      ? "Motor durumu bilinmiyor"
      : status.state === "ready"
        ? "Hazır"
        : status.state === "loading"
          ? "Yükleniyor…"
          : "Yapılandırma gerekli";
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-2.5 rounded-xl border border-white/[0.06] bg-lapis-950/50 p-3 text-left transition-colors hover:bg-lapis-850/70"
    >
      <StatusDot tone={tone} />
      <div className="min-w-0">
        <div className="truncate text-[12.5px] font-medium text-ink-50">{status ? ENGINE_NAMES[status.kind] : "Motor"}</div>
        <div className="truncate text-[11.5px] text-ink-400">{text}</div>
      </div>
    </button>
  );
}
