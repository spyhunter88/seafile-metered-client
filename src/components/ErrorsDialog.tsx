import { useEffect, useState } from "react";
import { api, errorMessage, formatTime, type LibraryError } from "../api";

/** Shows the latest error of a library, plus earlier ones recorded by the sync engine. */
export function ErrorsDialog({ repoId, name, onClose }: { repoId: string; name: string; onClose: () => void }) {
  const [errors, setErrors] = useState<LibraryError[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    api.getLibraryErrors(repoId).then(setErrors, (e) => setLoadError(errorMessage(e)));
  }, [repoId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const [latest, ...earlier] = errors ?? [];

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-slate-900/30 p-6"
      onMouseDown={onClose}
      // Keep right-clicks inside the dialog from reopening the row's menu.
      onContextMenu={(e) => e.stopPropagation()}
    >
      <div
        className="flex max-h-full w-full max-w-lg flex-col rounded-xl bg-white shadow-lg ring-1 ring-slate-200"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-slate-100 px-5 py-3">
          <h2 className="truncate text-sm font-semibold" title={name}>
            Last error · {name}
          </h2>
          <button className="text-xs text-slate-500 hover:text-slate-800" onClick={onClose}>
            Close
          </button>
        </div>

        <div className="overflow-auto px-5 py-4 text-xs">
          {loadError && <div className="rounded-md bg-rose-50 px-3 py-2 text-rose-700">{loadError}</div>}
          {!errors && !loadError && <div className="text-slate-500">Loading…</div>}
          {errors && !latest && <div className="text-slate-500">No errors recorded for this library.</div>}

          {latest && (
            <div className="rounded-md bg-rose-50 px-3 py-2 text-rose-800">
              <div className="font-semibold">{latest.message}</div>
              {latest.path && <div className="mt-0.5 font-mono break-all">{latest.path}</div>}
              <div className="mt-0.5 text-rose-600/80">{latest.time ? formatTime(latest.time) : "Current state"}</div>
            </div>
          )}

          {earlier.length > 0 && (
            <>
              <h3 className="mt-4 mb-1 font-semibold tracking-wide text-slate-500 uppercase">Earlier</h3>
              <ul className="divide-y divide-slate-100">
                {earlier.map((e, i) => (
                  <li key={i} className="py-1.5">
                    <div className="flex gap-2">
                      <span className="flex-1 text-slate-700">{e.message}</span>
                      <span className="whitespace-nowrap text-slate-400">{e.time ? formatTime(e.time) : "now"}</span>
                    </div>
                    {e.path && <div className="font-mono break-all text-slate-500">{e.path}</div>}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
