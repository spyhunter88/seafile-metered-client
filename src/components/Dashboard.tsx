import { getVersion } from "@tauri-apps/api/app";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  errorMessage,
  events,
  type LibrarySyncConfig,
  type NetworkCost,
  type RemoteRepo,
  type SessionInfo,
} from "../api";
import { NetworkBadge } from "./Badges";
import { HistoryPanel } from "./HistoryPanel";
import { LibraryRow } from "./LibraryRow";
import { Settings } from "./Settings";
import { StatusBar } from "./StatusBar";
import { useToast } from "./Toasts";

const POLL_MS = 3000;

export function Dashboard({ session, onLoggedOut }: { session: SessionInfo; onLoggedOut: () => void }) {
  const toast = useToast();
  const [network, setNetwork] = useState<NetworkCost>("Unmetered");
  const [remote, setRemote] = useState<RemoteRepo[]>([]);
  const [local, setLocal] = useState<LibrarySyncConfig[]>([]);
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [daemonError, setDaemonError] = useState<string | null>(null);
  const [loadingRemote, setLoadingRemote] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  const [storageWarning, setStorageWarning] = useState<string | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [tab, setTab] = useState<"libraries" | "history">("libraries");

  useEffect(() => {
    getVersion().then(setVersion).catch(console.error);
  }, []);

  const refreshStatus = useCallback(() => {
    api.getDaemonError().then(setDaemonError).catch(console.error);
    api
      .getStorageSettings()
      .then((s) => setStorageWarning(s.warning))
      .catch(console.error);
  }, []);

  const refreshLocal = useCallback(async () => {
    try {
      setLocal(await api.getLibraries());
    } catch (e) {
      console.error(e);
    }
  }, []);

  const refreshRemote = useCallback(async () => {
    setLoadingRemote(true);
    try {
      setRemote(await api.getCloudLibraries());
      setRemoteError(null);
    } catch (e) {
      setRemoteError(errorMessage(e));
    } finally {
      setLoadingRemote(false);
    }
  }, []);

  // Initial load + backend events.
  useEffect(() => {
    api.getNetworkStatus().then(setNetwork).catch(console.error);
    refreshStatus();
    refreshRemote();
    refreshLocal();

    const subs = [
      events.onNetworkChanged(setNetwork),
      events.onLibrariesUpdated(refreshLocal),
      events.onDaemonError(setDaemonError),
      events.onStorageChanged(() => {
        refreshStatus();
        refreshLocal();
      }),
      events.onAutoPaused((c) => toast("warning", "Paused on metered network", c.libraries.join(", "))),
      events.onAutoResumed((c) => toast("success", "Resumed on unmetered network", c.libraries.join(", "))),
    ];
    return () => {
      subs.forEach((p) => p.then((un) => un()));
    };
  }, [refreshLocal, refreshRemote, refreshStatus, toast]);

  // Poll live status only while the window is visible (hidden in tray = no polling).
  useEffect(() => {
    let timer: number | undefined;
    const start = () => {
      if (timer === undefined) timer = window.setInterval(refreshLocal, POLL_MS);
    };
    const stop = () => {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") {
        refreshLocal();
        start();
      } else {
        stop();
      }
    };
    onVisibility();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [refreshLocal]);

  const rows = useMemo(() => {
    const localById = new Map(local.map((l) => [l.repo_id, l]));
    const remoteIds = new Set(remote.map((r) => r.id));
    const synced = remote.filter((r) => localById.has(r.id));
    const others = remote.filter((r) => !localById.has(r.id));
    const orphanLocal = local.filter((l) => !remoteIds.has(l.repo_id));
    return [
      ...synced.map((r) => ({ key: r.id, remote: r, local: localById.get(r.id) })),
      ...orphanLocal.map((l) => ({ key: l.repo_id, remote: undefined, local: l })),
      ...others.map((r) => ({ key: r.id, remote: r, local: undefined })),
    ];
  }, [remote, local]);

  async function logout() {
    try {
      await api.logout();
      onLoggedOut();
    } catch (e) {
      toast("error", "Logout failed", errorMessage(e));
    }
  }

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-3 border-b border-slate-200 bg-white px-4 py-3">
        <img src="/app-icon.svg" alt="" className="h-6 w-6" />
        <h1 className="text-sm font-semibold">Seafile Metered Client</h1>
        {version && <span className="text-xs text-slate-400">v{version}</span>}
        <NetworkBadge cost={network} />
        <div className="flex-1" />
        <div className="text-right text-xs leading-tight text-slate-500">
          <div className="font-medium text-slate-700">{session.email}</div>
          <div>{session.server_url}</div>
        </div>
        <button
          className="rounded-md px-2.5 py-1 text-xs font-medium text-slate-600 ring-1 ring-slate-300 hover:bg-slate-50"
          onClick={() => setShowSettings(true)}
        >
          Settings
        </button>
        <button
          className="rounded-md px-2.5 py-1 text-xs font-medium text-slate-600 ring-1 ring-slate-300 hover:bg-slate-50"
          onClick={logout}
        >
          Log out
        </button>
      </header>

      {showSettings && (
        <Settings
          onClose={() => {
            setShowSettings(false);
            refreshStatus();
          }}
        />
      )}

      {storageWarning && (
        <div className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-800">{storageWarning}</div>
      )}

      {daemonError && (
        <div className="border-b border-rose-200 bg-rose-50 px-4 py-2 text-xs text-rose-700">
          <span className="font-semibold">Sync engine not running:</span> {daemonError}
        </div>
      )}
      {network === "Metered" && (
        <div className="border-b border-amber-200 bg-amber-50 px-4 py-2 text-xs text-amber-800">
          Metered network detected. Libraries without “Bypass Metered” are paused.
        </div>
      )}

      <main className="flex-1 overflow-auto p-4">
        <div className="mb-2 flex items-center gap-4">
          {(["libraries", "history"] as const).map((t) => (
            <button
              key={t}
              className={`border-b-2 pb-0.5 text-xs font-semibold tracking-wide uppercase ${
                tab === t ? "border-sky-600 text-slate-700" : "border-transparent text-slate-400 hover:text-slate-600"
              }`}
              onClick={() => setTab(t)}
            >
              {t === "libraries" ? "Libraries" : "Sync history"}
            </button>
          ))}
          <div className="flex-1" />
          {tab === "libraries" && (
            <>
              <span className="text-xs text-slate-400">Right-click a library for more actions</span>
              <button
                className="text-xs text-sky-700 hover:underline disabled:opacity-50"
                onClick={refreshRemote}
                disabled={loadingRemote}
              >
                {loadingRemote ? "Loading…" : "Refresh"}
              </button>
            </>
          )}
        </div>

        {tab === "history" ? (
          <HistoryPanel />
        ) : (
          <>
            {remoteError && (
              <div className="mb-3 rounded-md bg-rose-50 px-3 py-2 text-xs text-rose-700">
                Cannot load cloud libraries: {remoteError}
              </div>
            )}

            <div className="rounded-lg bg-white shadow-sm ring-1 ring-slate-200">
              {rows.length === 0 && !loadingRemote ? (
                <div className="px-4 py-8 text-center text-sm text-slate-500">No libraries found.</div>
              ) : (
                rows.map((r) => <LibraryRow key={r.key} remote={r.remote} local={r.local} onChanged={refreshLocal} />)
              )}
            </div>
          </>
        )}
      </main>

      <StatusBar libraries={local} daemonError={daemonError} />
    </div>
  );
}
