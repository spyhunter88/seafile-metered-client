import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type IntegrationState, type IntegrationStatus } from "../api";
import { useToast } from "./Toasts";

const stateLabel: Record<IntegrationState, { text: string; cls: string }> = {
  Installed: { text: "Installed", cls: "bg-emerald-100 text-emerald-700" },
  NotInstalled: { text: "Not installed", cls: "bg-slate-100 text-slate-600" },
  Broken: { text: "Incomplete", cls: "bg-amber-100 text-amber-800" },
  OtherOwner: { text: "Registered by another app", cls: "bg-amber-100 text-amber-800" },
};

const btn = "rounded-md px-2.5 py-1 text-xs font-medium disabled:opacity-50";

export function ExplorerIntegration() {
  const toast = useToast();
  const [status, setStatus] = useState<IntegrationStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [needsRestart, setNeedsRestart] = useState(false);

  const load = useCallback(async () => {
    try {
      setStatus(await api.getExplorerIntegration());
    } catch (e) {
      toast("error", "Cannot read Explorer integration status", errorMessage(e));
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  async function run(action: () => Promise<void>, success: string) {
    setBusy(true);
    try {
      await action();
      toast("success", success, "Restart Explorer to apply the change.");
      setNeedsRestart(true);
    } catch (e) {
      toast("error", "Explorer integration", errorMessage(e));
    } finally {
      setBusy(false);
      load();
    }
  }

  async function restart() {
    if (!confirm("Restart Windows Explorer now? Open folder windows will be closed.")) return;
    setBusy(true);
    try {
      await api.restartExplorer();
      setNeedsRestart(false);
    } catch (e) {
      toast("error", "Cannot restart Explorer", errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  const label = status ? stateLabel[status.state] : null;
  const canInstall = !!status?.bundled_available && status.state !== "Installed";
  const canRemove = status?.state === "Installed" || status?.state === "Broken";
  const off = busy;

  return (
    <div className="space-y-2">
      <div>
        <h3 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">Explorer integration</h3>
        <p className="mt-0.5 text-xs text-slate-500">
          Sync status icons on files and folders. Needs administrator permission once.
        </p>
      </div>

      <div className="flex items-center gap-2 rounded-md bg-slate-50 px-3 py-2 text-xs">
        {label && <span className={`rounded px-1.5 py-0.5 text-[11px] font-medium ${label.cls}`}>{label.text}</span>}
        <span className="min-w-0 flex-1 truncate font-mono text-slate-500" title={status?.dll_path ?? undefined}>
          {status?.dll_path ?? ""}
        </span>
      </div>

      {status?.state === "OtherOwner" && (
        <p className="text-xs text-amber-700">
          The icons are registered by another program (probably Seafile Desktop). Installing replaces that registration.
        </p>
      )}
      {status && !status.bundled_available && (
        <p className="text-xs text-rose-700">seafile_ext64.dll is missing from the app's binaries folder.</p>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          className={`${btn} bg-sky-600 text-white hover:bg-sky-700`}
          disabled={off || !canInstall}
          onClick={() => run(api.installExplorerIntegration, "Explorer integration installed")}
        >
          {status?.state === "Broken" ? "Repair" : "Install"}
        </button>
        <button
          className={`${btn} bg-white text-rose-600 ring-1 ring-rose-200 hover:bg-rose-50`}
          disabled={off || !canRemove}
          onClick={() => run(api.uninstallExplorerIntegration, "Explorer integration removed")}
        >
          Remove
        </button>
        <button
          className={`${btn} bg-white text-slate-700 ring-1 ring-slate-300 hover:bg-slate-50 ${needsRestart ? "ring-amber-400" : ""}`}
          disabled={off}
          onClick={restart}
        >
          Restart Explorer
        </button>
      </div>
    </div>
  );
}
