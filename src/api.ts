import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type NetworkCost = "Unmetered" | "Metered";

export type SyncStatus =
  | "Synced"
  | "Syncing"
  | "PausedMetered"
  | "PausedManual"
  | { Error: string };

export interface LibrarySyncConfig {
  repo_id: string;
  repo_name: string;
  local_path: string;
  is_user_paused: boolean;
  bypass_metered: boolean;
  current_state: SyncStatus;
}

export interface RemoteRepo {
  id: string;
  name: string;
  size: number;
  encrypted: boolean;
  permission: string;
  mtime: number;
  owner: string;
}

export interface SessionInfo {
  server_url: string;
  email: string;
}

export interface AutoChange {
  libraries: string[];
  network: NetworkCost;
}

export type LocationKind = "Portable" | "LocalAppData" | "Custom";

export interface LocationOption {
  kind: LocationKind;
  path: string;
  available: boolean;
  note: string | null;
}

export interface StorageSettings {
  current_kind: LocationKind;
  current_path: string;
  options: LocationOption[];
  warning: string | null;
}

export interface TransferRate {
  upload: number;
  download: number;
}

export interface HistoryEntry {
  id: number;
  time: number;
  repo_id: string;
  repo_name: string;
  kind: "Synced" | "Error";
  detail: string;
  path: string | null;
}

export interface HistorySettings {
  max_age_days: number;
  max_records: number;
}

export interface LibraryError {
  time: number | null;
  path: string | null;
  message: string;
}

export interface GeneralSettings {
  autostart: boolean;
  start_minimized: boolean;
  ignore_temp_files: boolean;
}

export interface PatternGroup {
  name: string;
  patterns: string[];
}

export const api = {
  getGeneralSettings: () => invoke<GeneralSettings>("get_general_settings"),
  setGeneralSettings: (settings: GeneralSettings) => invoke<void>("set_general_settings", { settings }),
  getTempIgnorePatterns: () => invoke<PatternGroup[]>("get_temp_ignore_patterns"),
  openSettingsWindow: () => invoke<void>("open_settings_window"),
  openLibraryFolder: (repoId: string) => invoke<void>("open_library_folder", { repoId }),
  openLibraryOnCloud: (repoId: string, repoName: string) =>
    invoke<void>("open_library_on_cloud", { repoId, repoName }),
  getLibraryErrors: (repoId: string) => invoke<LibraryError[]>("get_library_errors", { repoId }),
  getTransferRate: () => invoke<TransferRate>("get_transfer_rate"),
  getSyncHistory: () => invoke<HistoryEntry[]>("get_sync_history"),
  getHistorySettings: () => invoke<HistorySettings>("get_history_settings"),
  setHistorySettings: (settings: HistorySettings) => invoke<void>("set_history_settings", { settings }),
  clearSyncHistory: () => invoke<void>("clear_sync_history"),
  getStorageSettings: () => invoke<StorageSettings>("get_storage_settings"),
  moveDataLocation: (kind: LocationKind, customPath: string | null) =>
    invoke<void>("move_data_location", { kind, customPath }),
  login: (serverUrl: string, email: string, password: string) =>
    invoke<string>("login", { serverUrl, email, password }),
  logout: () => invoke<void>("logout"),
  getSession: () => invoke<SessionInfo | null>("get_session"),
  getCloudLibraries: () => invoke<RemoteRepo[]>("get_cloud_libraries"),
  getLibraries: () => invoke<LibrarySyncConfig[]>("get_libraries"),
  setupLibrarySync: (repoId: string, localPath: string, bypassMetered: boolean) =>
    invoke<void>("setup_library_sync", { repoId, localPath, bypassMetered }),
  toggleLibraryPause: (repoId: string, pauseState: boolean) =>
    invoke<void>("toggle_library_pause", { repoId, pauseState }),
  toggleBypassMetered: (repoId: string, bypass: boolean) =>
    invoke<void>("toggle_bypass_metered", { repoId, bypass }),
  forceSync: (repoId: string) => invoke<void>("force_sync", { repoId }),
  unsyncLibrary: (repoId: string) => invoke<void>("unsync_library", { repoId }),
  getNetworkStatus: () => invoke<NetworkCost>("get_network_status"),
  getDaemonError: () => invoke<string | null>("get_daemon_error"),
  pickFolder: () => invoke<string | null>("pick_folder"),
};

export const events = {
  onNetworkChanged: (cb: (c: NetworkCost) => void): Promise<UnlistenFn> =>
    listen<NetworkCost>("network-changed", (e) => cb(e.payload)),
  onLibrariesUpdated: (cb: () => void): Promise<UnlistenFn> =>
    listen("libraries-updated", () => cb()),
  onAutoPaused: (cb: (c: AutoChange) => void): Promise<UnlistenFn> =>
    listen<AutoChange>("auto-paused", (e) => cb(e.payload)),
  onAutoResumed: (cb: (c: AutoChange) => void): Promise<UnlistenFn> =>
    listen<AutoChange>("auto-resumed", (e) => cb(e.payload)),
  onDaemonError: (cb: (msg: string) => void): Promise<UnlistenFn> =>
    listen<string>("daemon-error", (e) => cb(e.payload)),
  onStorageChanged: (cb: () => void): Promise<UnlistenFn> => listen("storage-changed", () => cb()),
  onTransferRate: (cb: (r: TransferRate) => void): Promise<UnlistenFn> =>
    listen<TransferRate>("transfer-rate", (e) => cb(e.payload)),
  onHistoryUpdated: (cb: () => void): Promise<UnlistenFn> => listen("history-updated", () => cb()),
};

export function formatRate(bytesPerSec: number): string {
  return `${formatSize(bytesPerSec)}/s`;
}

export function formatTime(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString();
}

const relativeFmt = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
const RELATIVE_STEPS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 86400],
  ["month", 30 * 86400],
  ["week", 7 * 86400],
  ["day", 86400],
  ["hour", 3600],
  ["minute", 60],
];

/** "just now", "5 minutes ago", "yesterday", ... */
export function formatRelative(unixSecs: number, nowSecs = Date.now() / 1000): string {
  const diff = unixSecs - nowSecs;
  if (Math.abs(diff) < 45) return "just now";
  for (const [unit, secs] of RELATIVE_STEPS) {
    if (Math.abs(diff) >= secs || unit === "minute") {
      return relativeFmt.format(Math.round(diff / secs), unit);
    }
  }
  return "just now";
}

export function errorMessage(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function isPaused(s: SyncStatus): boolean {
  return s === "PausedManual" || s === "PausedMetered";
}

export function formatSize(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}
