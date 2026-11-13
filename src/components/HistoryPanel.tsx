import { useCallback, useEffect, useState } from "react";
import {
  api,
  errorMessage,
  events,
  formatRelative,
  formatTime,
  type HistoryEntry,
  type HistorySettings,
} from "../api";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { useToast } from "./Toasts";
import { useReveal } from "./useReveal";

const AGE_OPTIONS = [1, 3, 7, 14, 30, 90, 365];
const RECORD_OPTIONS = [50, 100, 200, 500, 1000, 5000];
const PAGE_SIZE = 20;

const selectCls =
  "rounded-md border border-slate-300 bg-white px-1.5 py-0.5 text-xs outline-none focus:border-sky-500 disabled:opacity-50";

/** Recent sync activity (completed syncs and errors) recorded from seaf-daemon. */
export function HistoryPanel() {
  const toast = useToast();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [settings, setSettings] = useState<HistorySettings | null>(null);
  const [saving, setSaving] = useState(false);
  // Click any time to switch the whole list between "5 minutes ago" and full date/time.
  const [absolute, setAbsolute] = useState(false);
  const now = useNow(30_000);
  const reveal = useReveal();
  const [page, setPage] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number; entry: HistoryEntry } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const pageCount = Math.max(1, Math.ceil(entries.length / PAGE_SIZE));
  // Clearing or pruning can shrink the list below the current page.
  const current = Math.min(page, pageCount - 1);
  const visible = entries.slice(current * PAGE_SIZE, (current + 1) * PAGE_SIZE);

  function menuItems(e: HistoryEntry): MenuItem[] {
    return [
      {
        label: e.path ? "Show in Explorer" : "Open library folder",
        hint: e.path ? "Select the item in its folder; opens the folder if the item was deleted" : undefined,
        onSelect: () => reveal(e.repo_id, e.path),
      },
      ...(e.path ? [{ label: "Open library folder", onSelect: () => reveal(e.repo_id, null) }] : []),
      { separator: true },
      {
        label: "Copy path",
        disabled: !e.path,
        onSelect: () => navigator.clipboard.writeText(e.path ?? "").catch(console.error),
      },
    ];
  }

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
            {visible.map((e) => (
              <li
                key={e.id}
                className={`flex items-start gap-3 px-4 py-2 text-xs ${menu?.entry.id === e.id ? "bg-slate-50" : ""}`}
                onContextMenu={(ev) => {
                  ev.preventDefault();
                  setMenu({ x: ev.clientX, y: ev.clientY, entry: e });
                }}
              >
                <span
                  className={`mt-1 h-2 w-2 shrink-0 rounded-full ${e.kind === "Error" ? "bg-rose-500" : "bg-emerald-500"}`}
                  title={e.kind === "Error" ? "Error" : "Synced"}
                />
                <div className="min-w-0 flex-1">
                  <div className="flex gap-2">
                    <span className="truncate font-medium text-slate-800">{e.repo_name || e.repo_id}</span>
                    <TimeCell time={e.time} now={now} absolute={absolute} onToggle={() => setAbsolute((v) => !v)} />
                  </div>
                  <div className={e.kind === "Error" ? "text-rose-700" : "text-slate-600"}>{e.detail}</div>
                  {e.path && <div className="truncate font-mono text-slate-500" title={e.path}>{e.path}</div>}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      {pageCount > 1 && (
        <div className="mt-2 flex items-center justify-end gap-3 text-xs text-slate-500">
          <span>
            {current * PAGE_SIZE + 1}–{Math.min((current + 1) * PAGE_SIZE, entries.length)} of {entries.length}
          </span>
          <button
            className="rounded px-2 py-0.5 ring-1 ring-slate-300 hover:bg-slate-50 disabled:opacity-40"
            disabled={current === 0}
            onClick={() => setPage(current - 1)}
          >
            ‹ Prev
          </button>
          <span>
            Page {current + 1} / {pageCount}
          </span>
          <button
            className="rounded px-2 py-0.5 ring-1 ring-slate-300 hover:bg-slate-50 disabled:opacity-40"
            disabled={current >= pageCount - 1}
            onClick={() => setPage(current + 1)}
          >
            Next ›
          </button>
        </div>
      )}

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.entry)} onClose={closeMenu} />}
    </div>
  );
}

/** Relative time; hover shows the exact date/time, click toggles the display. */
function TimeCell({
  time,
  now,
  absolute,
  onToggle,
}: {
  time: number;
  now: number;
  absolute: boolean;
  onToggle: () => void;
}) {
  const exact = formatTime(time);
  return (
    <button
      type="button"
      className="ml-auto whitespace-nowrap text-slate-400 hover:text-slate-600"
      title={absolute ? formatRelative(time, now) : exact}
      onClick={onToggle}
    >
      {absolute ? exact : formatRelative(time, now)}
    </button>
  );
}

/** Current time in seconds, refreshed periodically so relative labels stay correct. */
function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now() / 1000), intervalMs);
    return () => window.clearInterval(id);
  }, [intervalMs]);
  return now;
}

/** Keeps a stored value selectable even if it is not one of the presets. */
function withCurrent(options: number[], current: number): number[] {
  return options.includes(current) ? options : [...options, current].sort((a, b) => a - b);
}
