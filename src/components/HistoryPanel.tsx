import { useCallback, useEffect, useState } from "react";
import { api, errorMessage, events, formatTime, type HistoryEntry, type HistorySettings } from "../api";
import { useToast } from "./Toasts";

const AGE_OPTIONS = [1, 3, 7, 14, 30, 90, 365];
const RECORD_OPTIONS = [50, 100, 200, 500, 1000, 5000];

const selectCls =
  "rounded-md border border-slate-300 bg-white px-1.5 py-0.5 text-xs outline-none focus:border-sky-500 disabled:opacity-50";

/** Recent sync activity (completed syncs and errors) recorded from seaf-daemon. */
export function HistoryPanel() {
  const toast = useToast();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [settings, setSettings] = useState<HistorySettings | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(() => {
    api.getSyncHistory().then(setEntries).catch(console.error);
  }, []);

  useEffect(() => {
    load();
    api.getHistorySettings().then(setSettings).catch(console.error);
    const sub = events.onHistoryUpdated(load);
    return () => {
      sub.then((un) => un());
    };
  }, [load]);

  async function save(next: HistorySettings) {
    const prev = settings;
    setSettings(next);
    setSaving(true);
    try {
      await api.setHistorySettings(next);
    } catch (e) {
      setSettings(prev);
      toast("error", "Cannot save history settings", errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  async function clear() {
    if (!confirm("Clear the sync history?")) return;
    try {
      await api.clearSyncHistory();
    } catch (e) {
      toast("error", "Cannot clear history", errorMessage(e));
    }
  }

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center gap-3 text-xs text-slate-600">
        {settings && (
          <>
            <label className="flex items-center gap-1.5">
              Keep last
              <select
                className={selectCls}
                value={settings.max_age_days}
                disabled={saving}
                onChange={(e) => save({ ...settings, max_age_days: Number(e.target.value) })}
              >
                {withCurrent(AGE_OPTIONS, settings.max_age_days).map((d) => (
                  <option key={d} value={d}>
                    {d === 1 ? "1 day" : `${d} days`}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex items-center gap-1.5">
              Max records
              <select
                className={selectCls}
                value={settings.max_records}
                disabled={saving}
                onChange={(e) => save({ ...settings, max_records: Number(e.target.value) })}
              >
                {withCurrent(RECORD_OPTIONS, settings.max_records).map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
          </>
        )}
        <div className="flex-1" />
        <span className="text-slate-400">{entries.length} records</span>
        <button className="text-sky-700 hover:underline disabled:opacity-50" onClick={clear} disabled={entries.length === 0}>
          Clear
        </button>
      </div>

      <div className="rounded-lg bg-white shadow-sm ring-1 ring-slate-200">
        {entries.length === 0 ? (
          <div className="px-4 py-8 text-center text-sm text-slate-500">No sync activity recorded yet.</div>
        ) : (
          <ul className="divide-y divide-slate-100">
            {entries.map((e) => (
              <li key={e.id} className="flex items-start gap-3 px-4 py-2 text-xs">
                <span
                  className={`mt-1 h-2 w-2 shrink-0 rounded-full ${e.kind === "Error" ? "bg-rose-500" : "bg-emerald-500"}`}
                  title={e.kind === "Error" ? "Error" : "Synced"}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex gap-2">
                    <span className="truncate font-medium text-slate-800">{e.repo_name || e.repo_id}</span>
                    <span className="ml-auto whitespace-nowrap text-slate-400">{formatTime(e.time)}</span>
                  </div>
                  <div className={e.kind === "Error" ? "text-rose-700" : "text-slate-600"}>{e.detail}</div>
                  {e.path && <div className="truncate font-mono text-slate-500" title={e.path}>{e.path}</div>}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/** Keeps a stored value selectable even if it is not one of the presets. */
function withCurrent(options: number[], current: number): number[] {
  return options.includes(current) ? options : [...options, current].sort((a, b) => a - b);
}
