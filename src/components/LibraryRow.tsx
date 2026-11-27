import { useCallback, useState } from "react";
import { api, errorMessage, formatSize, isPaused, type LibrarySyncConfig, type RemoteRepo } from "../api";
import { StatusBadge } from "./Badges";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { ErrorsDialog } from "./ErrorsDialog";
import { useToast } from "./Toasts";

interface Props {
  /** Cloud info; missing when the library is configured locally but not listed by the server. */
  remote?: RemoteRepo;
  /** Local sync config; missing when the library is not synced. */
  local?: LibrarySyncConfig;
  onChanged: () => void;
}

const btn = "rounded-md px-2.5 py-1 text-xs font-medium disabled:opacity-50 whitespace-nowrap";
const btnPrimary = `${btn} bg-sky-600 text-white hover:bg-sky-700`;
const btnSecondary = `${btn} bg-white text-slate-700 ring-1 ring-slate-300 hover:bg-slate-50`;

const iconBtn = "inline-flex h-7 w-7 items-center justify-center rounded-md disabled:opacity-50 transition-colors";
const iconBtnPrimary = `${iconBtn} bg-sky-600 text-white hover:bg-sky-700`;
const iconBtnSecondary = `${iconBtn} bg-white text-slate-600 ring-1 ring-slate-300 hover:bg-slate-50 hover:text-slate-800`;
const iconBtnDanger = `${iconBtn} bg-white text-rose-600 ring-1 ring-rose-200 hover:bg-rose-50`;

function PlayIcon() {
  return (
    <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M8 5.14v14l11-7-11-7z" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg className="h-3.5 w-3.5" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M6 5h4v14H6V5zm8 0h4v14h-4V5z" />
    </svg>
  );
}

function SyncIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
      <path d="M16 16h5v5" />
    </svg>
  );
}

function UnlinkIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m18.84 12.25 1.72-1.71h-.02a5.004 5.004 0 0 0-.12-7.07 5.006 5.006 0 0 0-6.95 0l-1.72 1.71" />
      <path d="m5.17 11.75-1.71 1.71a5.004 5.004 0 0 0 .12 7.07 5.006 5.006 0 0 0 6.95 0l1.71-1.71" />
      <line x1="8" y1="2" x2="8" y2="5" />
      <line x1="2" y1="8" x2="5" y2="8" />
      <line x1="16" y1="19" x2="16" y2="22" />
      <line x1="19" y1="16" x2="22" y2="16" />
    </svg>
  );
}

function CloudDownloadIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M12 13v8l-4-4" />
      <path d="m12 21 4-4" />
      <path d="M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg
      className="h-3.5 w-3.5"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M18 6 6 18" />
      <path d="m6 6 12 12" />
    </svg>
  );
}

export function LibraryRow({ remote, local, onChanged }: Props) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [setupOpen, setSetupOpen] = useState(false);
  const [path, setPath] = useState("");
  const [bypass, setBypass] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [errorsOpen, setErrorsOpen] = useState(false);
  const closeMenu = useCallback(() => setMenu(null), []);

  const repoId = local?.repo_id ?? remote!.id;
  const name = local?.repo_name ?? remote!.name;

  async function run(action: () => Promise<void>, success?: string) {
    setBusy(true);
    try {
      await action();
      if (success) toast("success", success, name);
      onChanged();
    } catch (e) {
      toast("error", `${name}: action failed`, errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function browse() {
    try {
      const picked = await api.pickFolder();
      if (picked) setPath(picked);
    } catch (e) {
      toast("error", "Cannot open folder picker", errorMessage(e));
    }
  }

  async function attempt(action: () => Promise<void>, failure: string) {
    try {
      await action();
    } catch (e) {
      toast("error", failure, errorMessage(e));
    }
  }

  function confirmUnsync() {
    if (confirm(`Stop syncing "${name}"? Local files are kept.`)) {
      run(() => api.unsyncLibrary(repoId), "Library unsynced");
    }
  }

  const status = local?.current_state ?? null;
  const paused = local ? isPaused(local.current_state) || local.is_user_paused : false;

  const menuItems: MenuItem[] = local
    ? [
        {
          label: paused ? "Sync now (one-time)" : "Sync now",
          disabled: busy,
          onSelect: () => run(() => api.forceSync(repoId), "Sync started"),
        },
        { label: "Open local folder", onSelect: () => attempt(() => api.openLibraryFolder(repoId), "Cannot open folder") },
        {
          label: "View on cloud",
          onSelect: () => attempt(() => api.openLibraryOnCloud(repoId, name), "Cannot open browser"),
        },
        { separator: true },
        { label: "Last error…", onSelect: () => setErrorsOpen(true) },
        { separator: true },
        { label: "Unsync", danger: true, disabled: busy, onSelect: confirmUnsync },
      ]
    : [
        {
          label: "Sync…",
          disabled: busy || remote?.encrypted,
          hint: remote?.encrypted ? "Encrypted libraries are not supported yet" : undefined,
          onSelect: () => setSetupOpen(true),
        },
        { label: "Open local folder", disabled: true, onSelect: () => {} },
        {
          label: "View on cloud",
          onSelect: () => attempt(() => api.openLibraryOnCloud(repoId, name), "Cannot open browser"),
        },
        { separator: true },
        { label: "Last error…", disabled: true, onSelect: () => {} },
        { separator: true },
        { label: "Unsync", danger: true, disabled: true, onSelect: () => {} },
      ];

  const sizeLabel = remote ? formatSize(remote.size) : null;

  return (
    <div
      className={`border-b border-slate-100 px-4 py-3 last:border-b-0 ${menu ? "bg-slate-50" : ""}`}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
    >
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems} onClose={closeMenu} />}
      {errorsOpen && <ErrorsDialog repoId={repoId} name={name} onClose={() => setErrorsOpen(false)} />}

      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium" title={name}>
              {name}
            </span>
            {remote?.encrypted && <span className="text-[10px] text-slate-400 uppercase">encrypted</span>}
            {remote?.permission === "r" && <span className="text-[10px] text-slate-400 uppercase">read-only</span>}
          </div>
          <div className="flex min-w-0 gap-1.5 text-xs text-slate-500">
            {sizeLabel && <span className="shrink-0">{sizeLabel}</span>}
            {sizeLabel && local && <span className="text-slate-300">·</span>}
            {local && (
              <span className="truncate" title={local.local_path}>
                {local.local_path}
              </span>
            )}
          </div>
        </div>

        <StatusBadge status={status} />

        {local ? (
          <div className="flex items-center gap-2">
            <label
              className="flex cursor-pointer items-center gap-1 text-xs text-slate-600 select-none"
              title="Keep syncing this library on metered networks"
            >
              <input
                type="checkbox"
                className="accent-sky-600"
                checked={local.bypass_metered}
                disabled={busy}
                onChange={(e) => run(() => api.toggleBypassMetered(repoId, e.target.checked))}
              />
              Bypass Metered
            </label>
            {local.is_user_paused ? (
              <button
                className={iconBtnSecondary}
                disabled={busy}
                title="Resume"
                aria-label="Resume"
                onClick={() => run(() => api.toggleLibraryPause(repoId, false))}
              >
                <PlayIcon />
              </button>
            ) : (
              <button
                className={iconBtnSecondary}
                disabled={busy}
                title="Pause"
                aria-label="Pause"
                onClick={() => run(() => api.toggleLibraryPause(repoId, true))}
              >
                <PauseIcon />
              </button>
            )}
            {paused && (
              <button
                className={iconBtnPrimary}
                disabled={busy}
                title="Force sync (run one sync now, then stay paused)"
                aria-label="Force sync"
                onClick={() => run(() => api.forceSync(repoId), "Force sync started")}
              >
                <SyncIcon />
              </button>
            )}
            <button
              className={iconBtnDanger}
              disabled={busy}
              title="Unsync (keep local files)"
              aria-label="Unsync"
              onClick={confirmUnsync}
            >
              <UnlinkIcon />
            </button>
          </div>
        ) : (
          <button
            className={setupOpen ? iconBtnSecondary : iconBtnPrimary}
            disabled={busy || remote?.encrypted}
            title={
              remote?.encrypted
                ? "Encrypted libraries are not supported yet"
                : setupOpen
                  ? "Cancel"
                  : "Sync this library"
            }
            aria-label={setupOpen ? "Cancel" : "Sync this library"}
            onClick={() => setSetupOpen((v) => !v)}
          >
            {setupOpen ? <CloseIcon /> : <CloudDownloadIcon />}
          </button>
        )}
      </div>

      {!local && setupOpen && (
        <div className="mt-3 flex flex-wrap items-center gap-2 rounded-md bg-slate-50 p-3">
          <input
            className="min-w-0 flex-1 rounded-md border border-slate-300 bg-white px-2 py-1 text-xs outline-none focus:border-sky-500"
            placeholder="New or existing folder, e.g. D:\Seafile\Documents"
            title="Existing files with identical content are not downloaded again"
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
          <button className={btnSecondary} onClick={browse} disabled={busy}>
            Browse…
          </button>
          <label className="flex items-center gap-1 text-xs text-slate-600 select-none">
            <input type="checkbox" className="accent-sky-600" checked={bypass} onChange={(e) => setBypass(e.target.checked)} />
            Bypass Metered
          </label>
          <button
            className={btnPrimary}
            disabled={busy || !path.trim()}
            onClick={() =>
              run(async () => {
                await api.setupLibrarySync(repoId, path.trim(), bypass);
                setSetupOpen(false);
              }, "Sync started")
            }
          >
            Start sync
          </button>
        </div>
      )}
    </div>
  );
}
