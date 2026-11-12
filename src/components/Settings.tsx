import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  errorMessage,
  type GeneralSettings,
  type LocationKind,
  type PatternGroup,
  type StorageSettings,
} from "../api";
import { useToast } from "./Toasts";

const labels: Record<LocationKind, { title: string; hint: string }> = {
  Portable: { title: "Next to the app (portable)", hint: "Data folder beside the .exe" },
  LocalAppData: { title: "User AppData", hint: "%LOCALAPPDATA%\\CustomSeafile" },
  Custom: { title: "Custom folder", hint: "Any empty or new folder" },
};

const TABS = [
  { id: "general", label: "General" },
  { id: "storage", label: "Data location" },
] as const;
type TabId = (typeof TABS)[number]["id"];

/** Content of the separate "settings" window. */
export function SettingsWindow() {
  const [tab, setTab] = useState<TabId>("general");
  // A data move must finish before the window can close or switch tabs.
  const [moving, setMoving] = useState(false);
  const movingRef = useRef(false);
  movingRef.current = moving;

  useEffect(() => {
    const win = getCurrentWindow();
    const unlisten = win.onCloseRequested((e) => {
      if (movingRef.current) e.preventDefault();
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !movingRef.current) win.close();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      unlisten.then((un) => un());
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <div className="flex h-full flex-col bg-white">
      <div className="flex gap-4 border-b border-slate-200 px-5 pt-2" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={tab === t.id}
            disabled={moving && tab !== t.id}
            className={`-mb-px border-b-2 py-2 text-xs font-medium disabled:opacity-50 ${
              tab === t.id ? "border-sky-600 text-slate-800" : "border-transparent text-slate-500 hover:text-slate-700"
            }`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-auto">
        {tab === "general" ? <GeneralTab /> : <StorageTab moving={moving} setMoving={setMoving} />}
      </div>
    </div>
  );
}

// ---- General ----------------------------------------------------------------

function GeneralTab() {
  const toast = useToast();
  const [settings, setSettings] = useState<GeneralSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [patterns, setPatterns] = useState<PatternGroup[] | null>(null);
  const [showPatterns, setShowPatterns] = useState(false);

  useEffect(() => {
    api.getGeneralSettings().then(setSettings, (e) => toast("error", "Cannot load settings", errorMessage(e)));
    api.getTempIgnorePatterns().then(setPatterns).catch(console.error);
  }, [toast]);

  async function save(next: GeneralSettings) {
    const prev = settings;
    setSettings(next);
    setSaving(true);
    try {
      await api.setGeneralSettings(next);
    } catch (e) {
      setSettings(prev);
      toast("error", "Cannot save settings", errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="space-y-3 px-5 py-4">
      <h3 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">Startup</h3>
      {!settings ? (
        <div className="text-xs text-slate-500">Loading…</div>
      ) : (
        <div className="space-y-2">
          <label className="flex items-start gap-3 rounded-lg border border-slate-200 px-3 py-2.5">
            <input
              type="checkbox"
              className="mt-0.5 accent-sky-600"
              checked={settings.autostart}
              disabled={saving}
              onChange={(e) => save({ ...settings, autostart: e.target.checked })}
            />
            <div>
              <div className="text-sm font-medium">Start automatically with Windows</div>
              <div className="text-xs text-slate-500">Launch the app when you sign in to Windows.</div>
            </div>
          </label>

          <label
            className={`flex items-start gap-3 rounded-lg border border-slate-200 px-3 py-2.5 ${
              settings.autostart ? "" : "opacity-60"
            }`}
          >
            <input
              type="checkbox"
              className="mt-0.5 accent-sky-600"
              checked={settings.start_minimized}
              disabled={saving || !settings.autostart}
              onChange={(e) => save({ ...settings, start_minimized: e.target.checked })}
            />
            <div>
              <div className="text-sm font-medium">Start minimized to the system tray</div>
              <div className="text-xs text-slate-500">
                When started with Windows, sync in the background without opening the window.
              </div>
            </div>
          </label>
        </div>
      )}

      <h3 className="pt-2 text-xs font-semibold tracking-wide text-slate-500 uppercase">Sync</h3>
      {settings && (
        <div className="rounded-lg border border-slate-200 px-3 py-2.5">
          <label className="flex items-start gap-3">
            <input
              type="checkbox"
              className="mt-0.5 accent-sky-600"
              checked={settings.ignore_temp_files}
              disabled={saving}
              onChange={(e) => save({ ...settings, ignore_temp_files: e.target.checked })}
            />
            <div>
              <div className="text-sm font-medium">Ignore temporary files</div>
              <div className="text-xs text-slate-500">
                Don't upload lock and temp files from Word, Excel, draw.io, LibreOffice, AutoCAD, unfinished downloads, etc.
                Rules are written into <span className="font-mono">seafile-ignore.txt</span> in each library; your own lines
                in that file are kept.
              </div>
              <div className="mt-1 text-xs text-amber-700">
                The ignore file is synced like any other file, so the rules also apply on other computers syncing the
                library. Files already uploaded stay on the server.
              </div>
            </div>
          </label>
          <button
            type="button"
            className="mt-2 ml-7 text-xs text-sky-700 hover:underline"
            onClick={() => setShowPatterns((v) => !v)}
          >
            {showPatterns ? "Hide patterns" : "Show patterns"}
          </button>
          {showPatterns && patterns && (
            <dl className="mt-1 ml-7 space-y-1 text-xs">
              {patterns.map((g) => (
                <div key={g.name}>
                  <dt className="text-slate-600">{g.name}</dt>
                  <dd className="font-mono text-slate-500">{g.patterns.join("   ")}</dd>
                </div>
              ))}
            </dl>
          )}
        </div>
      )}
    </div>
  );
}

// ---- Data location ----------------------------------------------------------

function StorageTab({ moving, setMoving }: { moving: boolean; setMoving: (v: boolean) => void }) {
  const toast = useToast();
  const [settings, setSettings] = useState<StorageSettings | null>(null);
  const [selected, setSelected] = useState<LocationKind | null>(null);
  const [customPath, setCustomPath] = useState("");

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
    <>
      <div className="space-y-3 px-5 py-4">
        <div>
          <h3 className="text-xs font-semibold tracking-wide text-slate-500 uppercase">Data location</h3>
          <p className="mt-0.5 text-xs text-slate-500">
            Config database and sync engine data. Synced library folders stay where they are.
          </p>
        </div>

        {settings?.warning && <div className="rounded-md bg-amber-50 px-3 py-2 text-xs text-amber-800">{settings.warning}</div>}

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

      <div className="flex-1" />
      <div className="flex justify-end gap-2 border-t border-slate-100 px-5 py-3">
        <button
          className="rounded-md bg-sky-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-700 disabled:opacity-50"
          disabled={!settings || moving || unchanged || customInvalid}
          onClick={move}
        >
          {moving ? "Moving…" : "Move data here"}
        </button>
      </div>
    </>
  );
}
