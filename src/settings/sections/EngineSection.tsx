import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Cpu,
  Download,
  FolderOpen,
  Globe,
  KeyRound,
  Languages,
  LoaderCircle,
  RotateCw,
  Radio,
  Server,
  Trash,
  Wand,
  X,
  Zap,
} from "lucide-react";
import { motion } from "motion/react";
import { useCallback, useEffect, useState, type ComponentType } from "react";

import { formatBytes, formatPercent } from "../../lib/format";
import { api, subscribe } from "../../lib/ipc";
import {
  toAppError,
  type DownloadEvent,
  type EngineKind,
  type EngineStatus,
  type MlxModel,
  type ModelStatus,
  type PythonCheck,
  type Settings,
} from "../../lib/types";
import { useToasts } from "../toasts";
import { Button, Card, ProgressBar, Row, SectionHeader, Select, StatusDot, TextField, Toggle } from "../ui";
import type { UpdateSettings } from "../useSettings";

const LANGUAGES = [
  { value: "auto", label: "Otomatik algıla" },
  { value: "tr", label: "Türkçe" },
  { value: "en", label: "İngilizce" },
  { value: "de", label: "Almanca" },
  { value: "fr", label: "Fransızca" },
  { value: "es", label: "İspanyolca" },
  { value: "it", label: "İtalyanca" },
  { value: "pt", label: "Portekizce" },
  { value: "nl", label: "Felemenkçe" },
  { value: "ru", label: "Rusça" },
  { value: "ar", label: "Arapça" },
  { value: "fa", label: "Farsça" },
  { value: "ja", label: "Japonca" },
  { value: "zh", label: "Çince" },
  { value: "ko", label: "Korece" },
] as const;

const ENGINES: ReadonlyArray<{ kind: EngineKind; title: string; subtitle: string; icon: ComponentType<{ className?: string }> }> = [
  { kind: "mlx", title: "Yerel MLX", subtitle: "Apple GPU · diskteki model", icon: Cpu },
  { kind: "whisperCpp", title: "Gömülü whisper.cpp", subtitle: "Python'suz · Metal", icon: Zap },
  { kind: "wyoming", title: "Wyoming sunucusu", subtitle: "Ağdaki faster-whisper", icon: Server },
  { kind: "openai", title: "OpenAI uyumlu HTTP", subtitle: "/audio/transcriptions", icon: Globe },
];

interface SectionProps {
  settings: Settings;
  update: UpdateSettings;
}

export function EngineSection({ settings, update, status }: SectionProps & { status: EngineStatus | null }) {
  const t = settings.transcription;
  return (
    <>
      <SectionHeader title="Motor & Model" subtitle="Sesi hangi modelin yazıya dökeceğini seçin. Hepsi aynı arayüzü kullanır; değiştirmek tek tık." />
      <div className="space-y-4">
        <Card title="Dil" icon={Languages}>
          <Row label="Konuşma dili" hint="Tek dilde konuşuyorsanız sabitlemek kısa cümlelerde doğruluğu artırır.">
            <Select
              label="Konuşma dili"
              value={t.language}
              options={LANGUAGES}
              onChange={(value) => void update((d) => void (d.transcription.language = value))}
            />
          </Row>
        </Card>

        <div className="grid grid-cols-2 gap-3">
          {ENGINES.map((engine) => (
            <EngineCard
              key={engine.kind}
              engine={engine}
              active={t.engine === engine.kind}
              status={status?.kind === engine.kind && t.engine === engine.kind ? status : null}
              onSelect={() => void update((d) => void (d.transcription.engine = engine.kind))}
            />
          ))}
        </div>

        <EngineStatusLine status={status} active={t.engine} />

        {t.engine === "mlx" && <MlxConfig settings={settings} update={update} />}
        {t.engine === "whisperCpp" && <WhisperCppConfig settings={settings} update={update} />}
        {t.engine === "wyoming" && <WyomingConfig settings={settings} update={update} />}
        {t.engine === "openai" && <OpenAiConfig settings={settings} update={update} />}

        <Card
          title="Sözlük ipucu"
          icon={Wand}
          description="Sık kullandığınız özel isimleri ve terimleri yazın (ör. Sedjem, Tauri, Kubernetes). Model bunları doğru yazmaya yönelir."
        >
          <TextField
            label="Sözlük ipucu"
            multiline
            value={t.initialPrompt}
            placeholder="Sedjem, Enver, Tauri, Rust, Monster…"
            onCommit={(value) => void update((d) => void (d.transcription.initialPrompt = value))}
          />
        </Card>

        <Card
          title="Canlı yazıya dökme"
          icon={Radio}
          description="Altyapı hazır: streaming destekleyen bir motor eklendiğinde ses konuşurken parça parça işlenir ve ara metin canlı gösterilir. Whisper toplu çalışır, bu yüzden metin bıraktığınızda gelir."
        />
      </div>
    </>
  );
}

function EngineCard({
  engine,
  active,
  status,
  onSelect,
}: {
  engine: (typeof ENGINES)[number];
  active: boolean;
  status: EngineStatus | null;
  onSelect: () => void;
}) {
  const Icon = engine.icon;
  return (
    <motion.button
      type="button"
      onClick={onSelect}
      whileHover={{ y: -2 }}
      whileTap={{ scale: 0.98 }}
      className={`relative overflow-hidden rounded-2xl border p-4 text-left transition-colors ${
        active ? "border-gold-400/50 bg-gradient-to-br from-gold-400/[0.14] to-lapis-850/60" : "border-white/[0.07] bg-lapis-900/60 hover:border-white/15"
      }`}
    >
      <div className="flex items-start justify-between">
        <span
          className={`grid size-9 place-items-center rounded-xl ring-1 ${
            active ? "bg-gold-400/20 text-gold-200 ring-gold-400/40" : "bg-white/[0.05] text-ink-200 ring-white/10"
          }`}
        >
          <Icon className="size-[18px]" />
        </span>
        {active && (
          <span className="flex items-center gap-1.5 rounded-full bg-lapis-950/70 px-2 py-0.5 text-[11px] text-gold-200 ring-1 ring-gold-400/30">
            {status?.state === "loading" ? <LoaderCircle className="size-3 animate-spin" /> : <Check className="size-3" />}
            Aktif
          </span>
        )}
      </div>
      <div className="mt-3 font-display text-[14.5px] font-semibold text-ink-50">{engine.title}</div>
      <div className="text-[12px] text-ink-400">{engine.subtitle}</div>
    </motion.button>
  );
}

function EngineStatusLine({ status, active }: { status: EngineStatus | null; active: EngineKind }) {
  if (status === null || status.kind !== active) return null;
  const retry = () => void api.reloadEngine();
  return (
    <div
      className={`flex items-center gap-3 rounded-xl border px-4 py-3 text-[13px] ${
        status.state === "failed" ? "border-rose-glow/25 bg-rose-glow/[0.07]" : "border-white/[0.06] bg-lapis-900/50"
      }`}
    >
      <StatusDot tone={status.state === "ready" ? "ok" : status.state === "loading" ? "busy" : "error"} />
      <span className="flex-1 text-ink-200">
        {status.state === "ready" && <>Hazır — {status.description}</>}
        {status.state === "loading" && <>Model yükleniyor… İlk yükleme birkaç saniye sürebilir.</>}
        {status.state === "failed" && <>{status.message}</>}
      </span>
      {status.state !== "loading" && (
        <Button variant="subtle" icon={RotateCw} onClick={retry}>
          Yeniden yükle
        </Button>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ MLX --

function MlxConfig({ settings, update }: SectionProps) {
  const mlx = settings.transcription.mlx;
  const { push } = useToasts();
  const [models, setModels] = useState<MlxModel[] | null>(null);
  const [check, setCheck] = useState<PythonCheck | null>(null);
  const [setupLog, setSetupLog] = useState<string[]>([]);
  const [installing, setInstalling] = useState(false);

  const recheck = useCallback(() => {
    setCheck(null);
    api
      .checkMlxPython(mlx.pythonPath)
      .then(setCheck)
      .catch((err: unknown) => setCheck({ ok: false, mlxWhisperVersion: null, message: toAppError(err).message }));
  }, [mlx.pythonPath]);

  useEffect(() => {
    api.discoverMlxModels().then(setModels).catch(() => setModels([]));
  }, []);
  useEffect(recheck, [recheck]);
  useEffect(
    () =>
      subscribe("mlx:setup", (event) => {
        if (event.state === "log") {
          setSetupLog((lines) => [...lines.slice(-7), event.line]);
        } else {
          setInstalling(false);
          if (event.state === "done") {
            push("success", "MLX ortamı kuruldu; model yükleniyor.");
            recheck();
          } else {
            push("error", `Kurulum başarısız: ${event.message}`);
          }
        }
      }),
    [push, recheck],
  );

  const install = () => {
    setSetupLog([]);
    setInstalling(true);
    api.setupMlxEnv().catch((err: unknown) => {
      setInstalling(false);
      push("error", toAppError(err).message);
    });
  };

  const pickFolder = async () => {
    const picked = await open({ directory: true, title: "MLX model klasörünü seçin" });
    if (typeof picked === "string") void update((d) => void (d.transcription.mlx.modelDir = picked));
  };

  return (
    <>
      <Card
        title="Model"
        icon={Cpu}
        description="Hugging Face önbelleğinde bulunan MLX Whisper modelleri. İndirme gerekmez."
        actions={
          <Button icon={FolderOpen} onClick={() => void pickFolder()}>
            Klasör seç…
          </Button>
        }
      >
        {models === null ? (
          <LoaderCircle className="size-4 animate-spin text-ink-400" />
        ) : models.length === 0 ? (
          <p className="text-[13px] text-ink-400">Önbellekte MLX Whisper modeli bulunamadı. Bir klasör seçin ya da whisper.cpp motorunu kullanın.</p>
        ) : (
          <div className="space-y-2">
            {models.map((model) => (
              <ChoiceRow
                key={model.path}
                selected={mlx.modelDir === model.path}
                title={model.repo}
                meta={formatBytes(model.sizeBytes)}
                badge={model.repo.endsWith("large-v3-turbo") ? "Önerilen" : undefined}
                onSelect={() => void update((d) => void (d.transcription.mlx.modelDir = model.path))}
              />
            ))}
          </div>
        )}
        {mlx.modelDir && !models?.some((m) => m.path === mlx.modelDir) && (
          <p className="mt-3 truncate font-mono text-[11.5px] text-ink-400">{mlx.modelDir}</p>
        )}
      </Card>

      <Card
        title="Python çalışma ortamı"
        icon={Zap}
        description="MLX, Python üzerinden çalışır. Sedjem kendi yalıtılmış ortamını kurar (uv + mlx-whisper 0.4.3); başka projelere dokunmaz."
        actions={
          <Button variant={check?.ok ? "ghost" : "primary"} icon={Download} loading={installing} onClick={install}>
            {check?.ok ? "Onar" : "Ortamı kur"}
          </Button>
        }
      >
        <div className="flex items-center gap-2.5 text-[13px]">
          {check === null ? <LoaderCircle className="size-3.5 animate-spin text-ink-400" /> : <StatusDot tone={check.ok ? "ok" : "warn"} />}
          <span className="text-ink-200">{check?.message ?? "Kontrol ediliyor…"}</span>
        </div>
        {(installing || setupLog.length > 0) && (
          <pre className="scroll-quiet mt-3 max-h-40 overflow-auto rounded-lg bg-lapis-950/80 p-3 font-mono text-[11px] leading-relaxed text-ink-400">
            {setupLog.join("\n") || "Başlatılıyor…"}
          </pre>
        )}
        <div className="mt-4">
          <Row label="Özel Python yolu" hint="Boş bırakın: Sedjem'in kendi ortamı kullanılır.">
            <TextField
              label="Özel Python yolu"
              mono
              className="w-[300px]"
              value={mlx.pythonPath ?? ""}
              placeholder="/path/to/venv/bin/python"
              onCommit={(value) => void update((d) => void (d.transcription.mlx.pythonPath = value.trim() === "" ? null : value.trim()))}
            />
          </Row>
        </div>
      </Card>
    </>
  );
}

// ---------------------------------------------------------- whisper.cpp --

function WhisperCppConfig({ settings, update }: SectionProps) {
  const w = settings.transcription.whisperCpp;
  const { push } = useToasts();
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [progress, setProgress] = useState<Record<string, { downloaded: number; total: number }>>({});

  const refresh = useCallback(() => {
    api.listGgmlModels().then(setModels).catch(() => setModels([]));
  }, []);
  useEffect(refresh, [refresh]);

  useEffect(
    () =>
      subscribe("model:download", (event: DownloadEvent) => {
        if (event.state === "progress") {
          setProgress((p) => ({ ...p, [event.id]: { downloaded: event.downloaded, total: event.total } }));
          return;
        }
        setProgress(({ [event.id]: _finished, ...rest }) => rest);
        refresh();
        if (event.state === "done") {
          push("success", "Model indirildi ve doğrulandı.");
          if (!w.modelPath) void update((d) => void (d.transcription.whisperCpp.modelPath = event.path));
        } else if (event.state === "failed") {
          push("error", event.message);
        }
      }),
    [push, refresh, update, w.modelPath],
  );

  const download = (id: string) => {
    setProgress((p) => ({ ...p, [id]: { downloaded: 0, total: 1 } }));
    api.downloadGgmlModel(id).catch((err: unknown) => {
      setProgress(({ [id]: _failed, ...rest }) => rest);
      push("error", toAppError(err).message);
    });
  };
  const remove = (id: string) => {
    api
      .deleteGgmlModel(id)
      .then(refresh)
      .catch((err: unknown) => push("error", toAppError(err).message));
  };
  const pickFile = async () => {
    const picked = await open({ title: "ggml model dosyası seçin", filters: [{ name: "ggml model", extensions: ["bin"] }] });
    if (typeof picked === "string") void update((d) => void (d.transcription.whisperCpp.modelPath = picked));
  };

  return (
    <>
      <Card
        title="Modeller"
        icon={Download}
        description="Hugging Face'ten sabitlenmiş sürüm indirilir ve SHA-256 ile doğrulanır."
        actions={
          <Button icon={FolderOpen} onClick={() => void pickFile()}>
            Yerel dosya…
          </Button>
        }
      >
        <div className="space-y-2">
          {models.map((model) => {
            const job = progress[model.id];
            const inUse = w.modelPath === model.path;
            return (
              <div key={model.id} className="rounded-xl border border-white/[0.06] bg-lapis-950/40 p-3.5">
                <div className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-[13.5px] font-medium text-ink-50">
                      {model.label}
                      {model.recommended && <Badge>Önerilen</Badge>}
                      {inUse && <Badge tone="ok">Kullanılıyor</Badge>}
                    </div>
                    <div className="text-[12px] text-ink-400">
                      {model.description} · {formatBytes(model.sizeBytes)}
                    </div>
                  </div>
                  {job ? (
                    <Button variant="subtle" icon={X} onClick={() => void api.cancelGgmlDownload(model.id)}>
                      İptal
                    </Button>
                  ) : model.installed ? (
                    <div className="flex items-center gap-1">
                      {!inUse && (
                        <Button onClick={() => void update((d) => void (d.transcription.whisperCpp.modelPath = model.path))}>Kullan</Button>
                      )}
                      <Button variant="subtle" aria-label="Sil" icon={Trash} onClick={() => remove(model.id)} />
                    </div>
                  ) : (
                    <Button variant="primary" icon={Download} onClick={() => download(model.id)}>
                      İndir
                    </Button>
                  )}
                </div>
                {job && (
                  <div className="mt-3 flex items-center gap-3">
                    <ProgressBar value={job.total > 0 ? job.downloaded / job.total : 0} />
                    <span className="w-24 text-right font-mono text-[11.5px] text-gold-200 tabular-nums">
                      {formatPercent(job.downloaded, job.total)} · {formatBytes(job.downloaded)}
                    </span>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {w.modelPath && !models.some((m) => m.path === w.modelPath) && (
          <p className="mt-3 truncate font-mono text-[11.5px] text-ink-400">Seçili: {w.modelPath}</p>
        )}
      </Card>

      <Card title="Hızlandırma" icon={Zap}>
        <Row label="GPU (Metal)" hint="Apple Silicon GPU'sunda çalıştırır; kapatmak yalnızca hata ayıklama içindir.">
          <Toggle label="GPU (Metal)" checked={w.useGpu} onChange={(v) => void update((d) => void (d.transcription.whisperCpp.useGpu = v))} />
        </Row>
        <Row label="Flash attention" hint="Daha az bellek ve daha hızlı çözümleme.">
          <Toggle
            label="Flash attention"
            checked={w.flashAttention}
            onChange={(v) => void update((d) => void (d.transcription.whisperCpp.flashAttention = v))}
          />
        </Row>
      </Card>
    </>
  );
}

// -------------------------------------------------------------- Wyoming --

function WyomingConfig({ settings, update }: SectionProps) {
  const wy = settings.transcription.wyoming;
  const { push } = useToasts();
  const [testing, setTesting] = useState(false);

  const test = () => {
    setTesting(true);
    api
      .testWyoming(wy.host, wy.port)
      .then((model) => push("success", `Bağlantı başarılı: ${model}`))
      .catch((err: unknown) => push("error", toAppError(err).message))
      .finally(() => setTesting(false));
  };

  return (
    <Card
      title="Sunucu"
      icon={Server}
      description="Wyoming protokolüyle konuşan bir ASR sunucusu (ör. ağınızdaki faster-whisper). Ses yalnızca bu adrese gider."
      actions={
        <Button icon={Radio} loading={testing} disabled={wy.host.trim() === ""} onClick={test}>
          Bağlantıyı test et
        </Button>
      }
    >
      <Row label="Adres" hint="Ana makine adı veya IP.">
        <TextField
          label="Wyoming adresi"
          mono
          className="w-[240px]"
          value={wy.host}
          placeholder="ör. sunucu.local"
          onCommit={(value) => void update((d) => void (d.transcription.wyoming.host = value.trim()))}
        />
      </Row>
      <Row label="Port">
        <TextField
          label="Wyoming portu"
          type="number"
          mono
          className="w-[110px]"
          value={String(wy.port)}
          onCommit={(value) => {
            const port = Number.parseInt(value, 10);
            if (Number.isInteger(port) && port > 0 && port < 65_536) void update((d) => void (d.transcription.wyoming.port = port));
            else push("error", "Port 1–65535 arasında olmalı.");
          }}
        />
      </Row>
    </Card>
  );
}

// --------------------------------------------------------------- OpenAI --

function OpenAiConfig({ settings, update }: SectionProps) {
  const o = settings.transcription.openai;
  const { push } = useToasts();
  const [testing, setTesting] = useState(false);
  const [hasKey, setHasKey] = useState(false);
  const [keyDraft, setKeyDraft] = useState("");

  useEffect(() => {
    api.hasOpenaiApiKey().then(setHasKey).catch(() => setHasKey(false));
  }, []);

  const test = () => {
    setTesting(true);
    api
      .testOpenai(o.baseUrl)
      .then((message) => push("success", message))
      .catch((err: unknown) => push("error", toAppError(err).message))
      .finally(() => setTesting(false));
  };

  const saveKey = (key: string | null) => {
    api
      .setOpenaiApiKey(key)
      .then(() => {
        setHasKey(key !== null);
        setKeyDraft("");
        push("success", key === null ? "Anahtar silindi." : "Anahtar, Anahtar Zinciri'ne kaydedildi.");
      })
      .catch((err: unknown) => push("error", toAppError(err).message));
  };

  return (
    <Card
      title="Sunucu"
      icon={Globe}
      description="OpenAI uyumlu /audio/transcriptions uç noktası. Uzak adresler için https zorunludur."
      actions={
        <Button icon={Radio} loading={testing} disabled={o.baseUrl.trim() === ""} onClick={test}>
          Test et
        </Button>
      }
    >
      <Row label="Taban URL" hint="Sürüm kısmı dahil, ör. http://localhost:8000/v1">
        <TextField
          label="Taban URL"
          mono
          className="w-[300px]"
          value={o.baseUrl}
          placeholder="http://localhost:8000/v1"
          onCommit={(value) => void update((d) => void (d.transcription.openai.baseUrl = value.trim()))}
        />
      </Row>
      <Row label="Model adı">
        <TextField
          label="Model adı"
          mono
          className="w-[220px]"
          value={o.model}
          onCommit={(value) => void update((d) => void (d.transcription.openai.model = value.trim()))}
        />
      </Row>
      <Row label="API anahtarı" hint={hasKey ? "macOS Anahtar Zinciri'nde kayıtlı." : "İsteğe bağlı. Ayar dosyasına değil, Anahtar Zinciri'ne yazılır."}>
        <div className="flex items-center gap-2">
          <input
            aria-label="API anahtarı"
            type="password"
            autoComplete="off"
            value={keyDraft}
            placeholder={hasKey ? "••••••••" : "sk-…"}
            onChange={(event) => setKeyDraft(event.target.value)}
            className="h-8 w-[180px] rounded-lg border border-white/10 bg-lapis-950/70 px-3 font-mono text-[12.5px] text-ink-50 outline-none placeholder:text-ink-500 focus:border-gold-400/60"
          />
          <Button icon={KeyRound} disabled={keyDraft.trim() === ""} onClick={() => saveKey(keyDraft)}>
            Kaydet
          </Button>
          {hasKey && <Button variant="subtle" aria-label="Anahtarı sil" icon={Trash} onClick={() => saveKey(null)} />}
        </div>
      </Row>
    </Card>
  );
}

// -------------------------------------------------------------- helpers --

function Badge({ children, tone = "gold" }: { children: string; tone?: "gold" | "ok" }) {
  return (
    <span
      className={`rounded-full px-2 py-px text-[10.5px] font-semibold tracking-wide uppercase ${
        tone === "ok" ? "bg-jade/15 text-jade ring-1 ring-jade/30" : "bg-gold-400/15 text-gold-200 ring-1 ring-gold-400/30"
      }`}
    >
      {children}
    </span>
  );
}

function ChoiceRow({
  selected,
  title,
  meta,
  badge,
  onSelect,
}: {
  selected: boolean;
  title: string;
  meta: string;
  badge: string | undefined;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-colors ${
        selected ? "border-gold-400/45 bg-gold-400/[0.08]" : "border-white/[0.06] bg-lapis-950/40 hover:border-white/15"
      }`}
    >
      <span className={`grid size-4 place-items-center rounded-full ring-1 ${selected ? "bg-gold-400 ring-gold-300" : "ring-ink-500"}`}>
        {selected && <span className="size-1.5 rounded-full bg-lapis-950" />}
      </span>
      <span className="flex-1 truncate font-mono text-[12.5px] text-ink-50">{title}</span>
      {badge && <Badge>{badge}</Badge>}
      <span className="text-[12px] text-ink-400">{meta}</span>
    </button>
  );
}
