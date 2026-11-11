/** Typed wrappers around Tauri commands and events. */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AppError,
  AppInfo,
  DictationResult,
  DictationState,
  DownloadEvent,
  EngineStatus,
  HistoryEntry,
  MicrophoneStatus,
  MlxModel,
  MlxSetupEvent,
  ModelStatus,
  Permissions,
  PythonCheck,
  Settings,
  UpdateInfo,
} from "./types";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  takeStartupNotices: () => invoke<string[]>("take_startup_notices"),
  getAppInfo: () => invoke<AppInfo>("get_app_info"),

  panelPress: () => invoke<void>("panel_press"),
  panelRelease: () => invoke<void>("panel_release"),
  cancelDictation: () => invoke<void>("cancel_dictation"),
  getDictationState: () => invoke<DictationState>("get_dictation_state"),
  getHistory: () => invoke<HistoryEntry[]>("get_history"),
  clearHistory: () => invoke<void>("clear_history"),
  copyToClipboard: (text: string) => invoke<void>("copy_to_clipboard", { text }),

  getEngineStatus: () => invoke<EngineStatus>("get_engine_status"),
  reloadEngine: () => invoke<void>("reload_engine"),
  listGgmlModels: () => invoke<ModelStatus[]>("list_ggml_models"),
  downloadGgmlModel: (id: string) => invoke<void>("download_ggml_model", { id }),
  cancelGgmlDownload: (id: string) => invoke<void>("cancel_ggml_download", { id }),
  deleteGgmlModel: (id: string) => invoke<void>("delete_ggml_model", { id }),
  discoverMlxModels: () => invoke<MlxModel[]>("discover_mlx_models"),
  checkMlxPython: (pythonPath: string | null) => invoke<PythonCheck>("check_mlx_python", { pythonPath }),
  setupMlxEnv: () => invoke<void>("setup_mlx_env"),
  testWyoming: (host: string, port: number) => invoke<string>("test_wyoming", { host, port }),
  testOpenai: (baseUrl: string) => invoke<string>("test_openai", { baseUrl }),
  hasOpenaiApiKey: () => invoke<boolean>("has_openai_api_key"),
  setOpenaiApiKey: (key: string | null) => invoke<void>("set_openai_api_key", { key }),

  getPermissions: () => invoke<Permissions>("get_permissions"),
  openPrivacySettings: (pane: "accessibility" | "microphone") => invoke<void>("open_privacy_settings", { pane }),
  requestMicrophone: () => invoke<MicrophoneStatus>("request_microphone"),
  listInputDevices: () => invoke<string[]>("list_input_devices"),

  getUpdateInfo: () => invoke<UpdateInfo>("get_update_info"),
  checkForUpdate: () => invoke<UpdateInfo>("check_for_update"),
  installUpdate: () => invoke<void>("install_update"),

  showSettings: () => invoke<void>("show_settings"),
  quitApp: () => invoke<void>("quit_app"),
};

interface EventMap {
  "dictation:state": DictationState;
  "dictation:level": number;
  "dictation:partial": string;
  "dictation:result": DictationResult;
  "dictation:no-speech": null;
  "dictation:busy": null;
  "dictation:error": AppError;
  "engine:status": EngineStatus;
  "model:download": DownloadEvent;
  "mlx:setup": MlxSetupEvent;
  "settings:changed": Settings;
  "update:state": UpdateInfo;
  "ui:navigate": string;
  "panel:hover": boolean;
}

export type EventName = keyof EventMap;

export function on<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): Promise<UnlistenFn> {
  return listen<EventMap[E]>(event, (e) => handler(e.payload));
}

/**
 * Subscribes from a React effect. Handles the race where the component
 * unmounts before `listen` resolves, so no listener ever leaks.
 */
export function subscribe<E extends EventName>(event: E, handler: (payload: EventMap[E]) => void): () => void {
  let disposed = false;
  let unlisten: UnlistenFn | undefined;
  on(event, handler)
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((err: unknown) => console.error(`subscribe(${event}) failed`, err));
  return () => {
    disposed = true;
    unlisten?.();
  };
}
