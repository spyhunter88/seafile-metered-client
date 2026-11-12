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
const btnDanger = `${btn} bg-white text-rose-600 ring-1 ring-rose-200 hover:bg-rose-50`;

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
              <button className={btnSecondary} disabled={busy} onClick={() => run(() => api.toggleLibraryPause(repoId, false))}>
                Resume
              </button>
            ) : (
              <button className={btnSecondary} disabled={busy} onClick={() => run(() => api.toggleLibraryPause(repoId, true))}>
                Pause
              </button>
            )}
            {paused && (
              <button
                className={btnPrimary}
                disabled={busy}
                title="Run one sync now, then stay paused"
                onClick={() => run(() => api.forceSync(repoId), "Force sync started")}
              >
                Force sync
              </button>
            )}
            <button className={btnDanger} disabled={busy} onClick={confirmUnsync}>
              Unsync
            </button>
          </div>
        ) : (
          <button
            className={btnPrimary}
            disabled={busy || remote?.encrypted}
            title={remote?.encrypted ? "Encrypted libraries are not supported yet" : undefined}
            onClick={() => setSetupOpen((v) => !v)}
          >
            {setupOpen ? "Cancel" : "Sync…"}
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
