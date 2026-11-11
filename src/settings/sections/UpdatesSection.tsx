import { ArrowUpCircle, CircleCheck, Download, LoaderCircle, RefreshCw, RotateCw, ShieldCheck, Sparkles } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";

import { formatBytes, formatPercent } from "../../lib/format";
import { api } from "../../lib/ipc";
import { toAppError, type Settings, type UpdateInfo } from "../../lib/types";
import { useToasts } from "../toasts";
import { Button, Card, ProgressBar, Row, SectionHeader, StatusDot, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

/** Empty string for a missing/invalid timestamp, so the UI never shows "Invalid Date". */
function formatDate(epochMs: number | undefined): string {
  if (typeof epochMs !== "number" || !Number.isFinite(epochMs)) return "";
  return new Date(epochMs).toLocaleString("tr-TR", { day: "numeric", month: "long", hour: "2-digit", minute: "2-digit" });
}

export function UpdatesSection({
  settings,
  update,
  info,
}: {
  settings: Settings;
  update: UpdateSettings;
  info: UpdateInfo | null;
}) {
  const { push } = useToasts();
  const [installing, setInstalling] = useState(false);
  const u = settings.updates;
  const busy = info?.state === "checking" || info?.state === "downloading" || info?.state === "installing";

  const check = () =>
    api.checkForUpdate().catch((err: unknown) => push("error", toAppError(err).message));

  const install = () => {
    setInstalling(true);
    api.installUpdate().catch((err: unknown) => {
      setInstalling(false);
      push("error", toAppError(err).message);
    });
  };

  return (
    <>
      <SectionHeader title="Güncellemeler" subtitle="Sedjem kendini GitHub üzerinden, imzası doğrulanmış sürümlerle günceller." />
      <div className="space-y-4">
        <HeroCard info={info} busy={busy} installing={installing} onCheck={() => void check()} onInstall={install} />

        <Card title="Otomatik güncelleme" icon={RefreshCw}>
          <Row label="Güncellemeleri otomatik denetle" hint="Açılıştan kısa süre sonra ve 6 saatte bir kontrol eder.">
            <Toggle label="Güncellemeleri otomatik denetle" checked={u.autoCheck} onChange={(v) => void update((d) => void (d.updates.autoCheck = v))} />
          </Row>
          <Row
            label="Bulunca kendiliğinden kur"
            hint="Dikte sürerken asla yeniden başlatmaz; boşta olduğunuz ilk anı bekler."
          >
            <Toggle
              label="Bulunca kendiliğinden kur"
              checked={u.autoInstall}
              disabled={!u.autoCheck}
              onChange={(v) => void update((d) => void (d.updates.autoInstall = v))}
            />
          </Row>
        </Card>

        <div className="flex items-start gap-3 rounded-2xl border border-white/[0.06] bg-lapis-900/40 p-4 text-[12.5px] leading-relaxed text-ink-400">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-jade" />
          Her güncelleme, uygulamaya gömülü açık anahtarla doğrulanır. İmzası tutmayan bir dosya asla kurulmaz; ayarlarınız, snippet'leriniz ve
          izinleriniz güncellemeden etkilenmez.
        </div>
      </div>
    </>
  );
}

function HeroCard({
  info,
  busy,
  installing,
  onCheck,
  onInstall,
}: {
  info: UpdateInfo | null;
  busy: boolean;
  installing: boolean;
  onCheck: () => void;
  onInstall: () => void;
}) {
  const available = info?.state === "available";
  return (
    <motion.section
      layout
      className={`relative overflow-hidden rounded-2xl border p-6 ${
        available ? "border-gold-400/40 bg-gradient-to-br from-gold-400/[0.14] via-lapis-850/80 to-lapis-900/80" : "border-white/[0.06] bg-gradient-to-b from-lapis-850/80 to-lapis-900/70"
      }`}
    >
      {available && <div className="pointer-events-none absolute -top-24 -right-16 size-64 rounded-full bg-gold-400/20 blur-[80px]" />}
      <div className="relative flex items-start justify-between gap-6">
        <div className="flex items-start gap-4">
          <span
            className={`grid size-12 shrink-0 place-items-center rounded-2xl ring-1 ${
              available ? "bg-gold-400/20 text-gold-200 ring-gold-400/40" : "bg-white/[0.05] text-ink-200 ring-white/10"
            }`}
          >
            <HeroIcon info={info} />
          </span>
          <div>
            <div className="text-[12px] tracking-wide text-ink-400 uppercase">Yüklü sürüm</div>
            <div className="font-display text-[22px] font-semibold tracking-tight text-ink-50">v{info?.currentVersion ?? "…"}</div>
            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={info?.state ?? "none"}
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -4 }}
                className="mt-1.5 flex items-center gap-2 text-[13px] text-ink-200"
              >
                <StatusLine info={info} />
              </motion.div>
            </AnimatePresence>
          </div>
        </div>
        <div className="flex shrink-0 gap-2">
          {available ? (
            <Button variant="primary" icon={Download} loading={installing} onClick={onInstall}>
              İndir ve yeniden başlat
            </Button>
          ) : (
            <Button icon={RotateCw} loading={info?.state === "checking"} disabled={busy} onClick={onCheck}>
              Şimdi denetle
            </Button>
          )}
        </div>
      </div>

      {info?.state === "downloading" && (
        <div className="relative mt-5 flex items-center gap-3">
          <ProgressBar value={info.total ? info.downloaded / info.total : 0} />
          <span className="w-28 text-right font-mono text-[11.5px] text-gold-200 tabular-nums">
            {info.total ? formatPercent(info.downloaded, info.total) : ""} · {formatBytes(info.downloaded)}
          </span>
        </div>
      )}

      {available && info.notes && (
        <div className="relative mt-5 rounded-xl border border-white/[0.06] bg-lapis-950/50 p-4">
          <div className="mb-2 flex items-center gap-2 text-[12px] font-medium text-gold-200">
            <Sparkles className="size-3.5" /> v{info.version} ile gelenler
          </div>
          <p className="text-[13px] leading-relaxed whitespace-pre-wrap text-ink-200">{info.notes}</p>
        </div>
      )}
    </motion.section>
  );
}

function HeroIcon({ info }: { info: UpdateInfo | null }) {
  switch (info?.state) {
    case "checking":
    case "downloading":
    case "installing":
      return <LoaderCircle className="size-6 animate-spin" />;
    case "available":
      return <ArrowUpCircle className="size-6" />;
    case "upToDate":
      return <CircleCheck className="size-6 text-jade" />;
    default:
      return <RefreshCw className="size-6" />;
  }
}

function StatusLine({ info }: { info: UpdateInfo | null }) {
  switch (info?.state) {
    case "checking":
      return <>Denetleniyor…</>;
    case "upToDate":
      return (
        <>
          <StatusDot tone="ok" /> En güncel sürümü kullanıyorsunuz
          {formatDate(info.checkedAtMs) && <span className="text-ink-400">· {formatDate(info.checkedAtMs)}</span>}
        </>
      );
    case "available":
      return (
        <>
          <StatusDot tone="warn" /> Yeni sürüm hazır: <span className="font-semibold text-gold-200">v{info.version}</span>
        </>
      );
    case "downloading":
      return <>v{info.version} indiriliyor…</>;
    case "installing":
      return <>v{info.version} kuruluyor; Sedjem birazdan yeniden başlayacak…</>;
    case "failed":
      return (
        <>
          <StatusDot tone="error" /> {info.message}
        </>
      );
    default:
      return <>Henüz denetlenmedi.</>;
  }
}
