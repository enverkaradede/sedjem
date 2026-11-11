import { getCurrentWindow } from "@tauri-apps/api/window";
import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";

// Each window loads only its own UI: the panel stays tiny and fast.
const PanelApp = lazy(() => import("./panel/PanelApp").then((m) => ({ default: m.PanelApp })));
const SettingsApp = lazy(() => import("./settings/SettingsApp").then((m) => ({ default: m.SettingsApp })));

const isPanel = getCurrentWindow().label === "panel";
document.documentElement.dataset.window = isPanel ? "panel" : "settings";

const container = document.getElementById("root");
if (!container) {
  throw new Error("root element missing from index.html");
}

createRoot(container).render(
  <StrictMode>
    <Suspense fallback={null}>{isPanel ? <PanelApp /> : <SettingsApp />}</Suspense>
  </StrictMode>,
);
