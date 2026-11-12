import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, type LocationKind, type StorageSettings } from "../api";
import { useToast } from "./Toasts";

const labels: Record<LocationKind, { title: string; hint: string }> = {
  Portable: { title: "Next to the app (portable)", hint: "Data folder beside the .exe" },
  LocalAppData: { title: "User AppData", hint: "%LOCALAPPDATA%\\CustomSeafile" },
  Custom: { title: "Custom folder", hint: "Any empty or new folder" },
};

export function Settings({ onClose }: { onClose: () => void }) {
  const toast = useToast();
  const [settings, setSettings] = useState<StorageSettings | null>(null);
  const [selected, setSelected] = useState<LocationKind | null>(null);
  const [customPath, setCustomPath] = useState("");
  const [moving, setMoving] = useState(false);

  const load = useCallback(async () => {
    try {
      const s = await api.getStorageSettings();
      setSettings(s);
      setSelected(s.current_kind);
      setCustomPath(s.options.find((o) => o.kind === "Custom")?.path ?? "");
    } catch (e) {
      toast("error", "Cannot load settings", errorMessage(e));
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  async function browse() {
    try {
      const p = await api.pickFolder();
      if (p) {
        setCustomPath(p);
        setSelected("Custom");
      }
    } catch (e) {
      toast("error", "Cannot open folder picker", errorMessage(e));
    }
  }

  async function move() {
    if (!settings || !selected) return;
    const target =
      selected === "Custom" ? customPath.trim() : settings.options.find((o) => o.kind === selected)?.path ?? "";
    const ok = confirm(
      `Move app data to:\n${target}\n\nSync is stopped during the move and restarted afterwards. ` +
        `Your synced folders are not moved.`,
    );
    if (!ok) return;

    setMoving(true);
    try {
      await api.moveDataLocation(selected, selected === "Custom" ? customPath.trim() : null);
      toast("success", "Data moved", target);
    } catch (e) {
      toast("error", "Move failed", errorMessage(e));
    } finally {
      setMoving(false);
      load();
    }
  }

  const customInvalid = selected === "Custom" && !customPath.trim();
  const unchanged =
    !!settings &&
    selected === settings.current_kind &&
    (selected !== "Custom" || customPath.trim() === settings.current_path);

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-slate-900/30 p-6">
      <div className="w-full max-w-xl rounded-xl bg-white shadow-lg ring-1 ring-slate-200">
        <div className="flex items-center justify-between border-b border-slate-100 px-5 py-3">
          <h2 className="text-sm font-semibold">Settings</h2>
          <button className="text-xs text-slate-500 hover:text-slate-800 disabled:opacity-50" onClick={onClose} disabled={moving}>
            Close
          </button>
        </div>

        <div className="space-y-3 px-5 py-4">
          <div>
            <h3 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">Data location</h3>
            <p className="mt-0.5 text-xs text-slate-500">
              Config database and sync engine data. Synced library folders stay where they are.
            </p>
          </div>

          {settings?.warning && (
            <div className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800">{settings.warning}</div>
          )}

          {settings && (
            <div className="rounded-md bg-slate-50 px-3 py-2 text-xs">
              <span className="text-slate-500">Current: </span>
              <span className="font-mono break-all">{settings.current_path}</span>
            </div>
          )}

          <div className="space-y-2">
            {settings?.options.map((opt) => {
              const isCurrent = opt.kind === settings.current_kind;
              return (
                <label
                  key={opt.kind}
                  className={`flex cursor-pointer gap-3 rounded-lg border px-3 py-2.5 ${
                    selected === opt.kind ? "border-sky-400 bg-sky-50/50" : "border-slate-200"
                  } ${!opt.available ? "cursor-not-allowed opacity-60" : ""}`}
                >
                  <input
                    type="radio"
                    name="location"
                    className="mt-0.5 accent-sky-600"
                    checked={selected === opt.kind}
                    disabled={!opt.available || moving}
                    onChange={() => setSelected(opt.kind)}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-sm font-medium">
                      {labels[opt.kind].title}
                      {isCurrent && (
                        <span className="rounded bg-emerald-100 px-1.5 py-0.5 text-[10px] text-emerald-700">current</span>
                      )}
                    </div>
                    {opt.kind === "Custom" ? (
                      <div className="mt-1 flex gap-2">
                        <input
                          className="min-w-0 flex-1 rounded-md border border-slate-300 bg-white px-2 py-1 font-mono text-xs outline-none focus:border-sky-500"
                          placeholder="D:\SeafileData"
                          value={customPath}
                          disabled={moving}
                          onChange={(e) => {
                            setCustomPath(e.target.value);
                            setSelected("Custom");
                          }}
                        />
                        <button
                          type="button"
                          className="rounded-md px-2.5 py-1 text-xs font-medium text-slate-700 ring-1 ring-slate-300 hover:bg-slate-50 disabled:opacity-50"
                          onClick={browse}
                          disabled={moving}
                        >
                          Browse…
                        </button>
                      </div>
                    ) : (
                      <div className="font-mono text-xs break-all text-slate-500">{opt.path || labels[opt.kind].hint}</div>
                    )}
                    {opt.note && <div className="mt-0.5 text-xs text-amber-700">{opt.note}</div>}
                  </div>
                </label>
              );
            })}
          </div>
        </div>

        <div className="flex justify-end gap-2 border-t border-slate-100 px-5 py-3">
          <button
            className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
            disabled={!settings || moving || unchanged || customInvalid}
            onClick={move}
          >
            {moving ? "Moving…" : "Move data here"}
          </button>
        </div>
      </div>
    </div>
  );
}
