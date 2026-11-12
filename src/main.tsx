import { getCurrentWindow } from "@tauri-apps/api/window";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { SettingsWindow } from "./components/Settings";
import { ToastProvider } from "./components/Toasts";
import "./index.css";

// Every window loads index.html; the window label decides what it shows.
const isSettings = getCurrentWindow().label === "settings";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ToastProvider>{isSettings ? <SettingsWindow /> : <App />}</ToastProvider>
  </React.StrictMode>,
);
