/** Mirrors of the Rust IPC types (serde camelCase). Keep in sync with src-tauri. */

export type EngineKind = "whisperCpp" | "mlx" | "wyoming" | "openai";
export type SnippetMatch = "exact" | "inline";

export interface PanelPosition {
  x: number;
  y: number;
}

export type InsertMethod = "paste" | "type";

export interface GeneralSettings {
  launchAtLogin: boolean;
  showInDock: boolean;
  panelVisible: boolean;
  panelPosition: PanelPosition | null;
  insertMethod: InsertMethod;
  autoEnter: boolean;
  restoreClipboard: boolean;
  keepHistory: boolean;
}

export interface ShortcutSettings {
  enabled: boolean;
  accelerator: string;
}

export interface RecordingSettings {
  tapThresholdMs: number;
  silenceAutoStop: boolean;
  silenceTimeoutMs: number;
  maxDurationS: number;
  inputDevice: string | null;
}

export interface TranscriptionSettings {
  engine: EngineKind;
  language: string;
  initialPrompt: string;
  whisperCpp: { modelPath: string | null; useGpu: boolean; flashAttention: boolean };
  mlx: { pythonPath: string | null; modelDir: string | null };
  wyoming: { host: string; port: number };
  openai: { baseUrl: string; model: string };
}

export interface Snippet {
  id: string;
  trigger: string;
  expansion: string;
  matchMode: SnippetMatch;
  enabled: boolean;
}

export interface Settings {
  version: number;
  general: GeneralSettings;
  shortcut: ShortcutSettings;
  recording: RecordingSettings;
  transcription: TranscriptionSettings;
  text: { filterHallucinations: boolean; snippets: Snippet[] };
  updates: { autoCheck: boolean; autoInstall: boolean };
}

export type ErrorCode =
  | "invalid_settings"
  | "io"
  | "shortcut"
  | "autostart"
  | "audio"
  | "engine"
  | "engine_not_ready"
  | "no_speech"
  | "inject"
  | "download"
  | "keychain"
  | "permission"
  | "internal";

export interface AppError {
  code: ErrorCode;
  message: string;
}

export type Phase = "idle" | "recording" | "processing";
export type Latch = "pending" | "toggle";

export interface DictationState {
  phase: Phase;
  latch: Latch | null;
  source: "panel" | "hotkey" | null;
}

export interface DictationResult {
  text: string;
  audioMs: number;
  latencyMs: number;
  language: string | null;
  inserted: boolean;
}

export interface HistoryEntry {
  id: number;
  text: string;
  createdAtMs: number;
  language: string | null;
}

export type EngineStatus =
  | { state: "loading"; kind: EngineKind }
  | { state: "ready"; kind: EngineKind; description: string }
  | { state: "failed"; kind: EngineKind; message: string };

export interface ModelStatus {
  id: string;
  fileName: string;
  label: string;
  description: string;
  sizeBytes: number;
  sha256: string;
  recommended: boolean;
  installed: boolean;
  path: string;
}

export type DownloadEvent =
  | { state: "progress"; id: string; downloaded: number; total: number }
  | { state: "done"; id: string; path: string }
  | { state: "failed"; id: string; message: string }
  | { state: "cancelled"; id: string };

export interface MlxModel {
  repo: string;
  path: string;
  sizeBytes: number;
}

export interface PythonCheck {
  ok: boolean;
  mlxWhisperVersion: string | null;
  message: string;
}

export type MlxSetupEvent =
  | { state: "log"; line: string }
  | { state: "done"; pythonPath: string }
  | { state: "failed"; message: string };

export type MicrophoneStatus = "granted" | "denied" | "restricted" | "notDetermined" | "unknown";

export interface Permissions {
  accessibility: boolean;
  microphone: MicrophoneStatus;
}

export type UpdateState =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "upToDate"; checkedAtMs: number }
  | { state: "available"; version: string; notes: string | null; date: string | null }
  | { state: "downloading"; version: string; downloaded: number; total: number | null }
  | { state: "installing"; version: string }
  | { state: "failed"; message: string };

export type UpdateInfo = UpdateState & { currentVersion: string };

export interface AppInfo {
  version: string;
  appDataDir: string;
}

/** Narrows an unknown rejection from `invoke` into an AppError. */
export function toAppError(err: unknown): AppError {
  if (typeof err === "object" && err !== null && "code" in err && "message" in err) {
    const candidate = err as { code: unknown; message: unknown };
    if (typeof candidate.code === "string" && typeof candidate.message === "string") {
      return { code: candidate.code as ErrorCode, message: candidate.message };
    }
  }
  return { code: "internal", message: typeof err === "string" ? err : "Beklenmeyen bir hata oluştu." };
}
