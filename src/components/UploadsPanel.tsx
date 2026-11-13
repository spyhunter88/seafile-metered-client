import { useCallback, useState } from "react";
import { formatRate, type UploadFile, type UploadTask } from "../api";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { useReveal } from "./useReveal";

const STATUS_LABEL: Record<string, { text: string; cls: string }> = {
  add: { text: "New", cls: "bg-emerald-100 text-emerald-700" },
  newdir: { text: "New folder", cls: "bg-emerald-100 text-emerald-700" },
  mod: { text: "Modified", cls: "bg-sky-100 text-sky-700" },
  mov: { text: "Renamed", cls: "bg-violet-100 text-violet-700" },
  del: { text: "Deleted", cls: "bg-rose-100 text-rose-700" },
  deldir: { text: "Deleted folder", cls: "bg-rose-100 text-rose-700" },
};

const STEP_LABEL: Record<string, string> = {
  data: "Uploading file data",
  fs: "Uploading file list",
  check: "Checking server",
  commit: "Finishing",
  "update-branch": "Finishing",
};

/** Libraries preparing or sending an upload, with progress, speed and the files involved. */
export function UploadsPanel({ tasks }: { tasks: UploadTask[] }) {
  const reveal = useReveal();
  const [menu, setMenu] = useState<{ x: number; y: number; repoId: string; file: UploadFile } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  if (tasks.length === 0) {
    return (
      <div className="rounded-lg bg-white px-4 py-8 text-center text-sm text-slate-500 shadow-sm ring-1 ring-slate-200">
        Nothing to upload. New and changed files show up here while they are being uploaded.
      </div>
    );
  }

  const menuItems = (repoId: string, f: UploadFile): MenuItem[] => [
    {
      label: "Show in Explorer",
      hint: "Select the item in its folder; opens the folder if the item was deleted",
      onSelect: () => reveal(repoId, f.path),
    },
    { label: "Open library folder", onSelect: () => reveal(repoId, null) },
    { separator: true },
    { label: "Copy path", onSelect: () => navigator.clipboard.writeText(f.path).catch(console.error) },
  ];

  return (
    <div className="space-y-3">
      {tasks.map((t) => (
        <TaskCard
          key={t.repo_id}
          task={t}
          selected={menu?.repoId === t.repo_id ? menu.file.path : null}
          onFileMenu={(x, y, file) => setMenu({ x, y, repoId: t.repo_id, file })}
        />
      ))}
      <p className="text-xs text-slate-400">
        Progress and speed are reported per library by the sync engine, not per file.
      </p>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.repoId, menu.file)} onClose={closeMenu} />}
    </div>
  );
}

function TaskCard({
  task: t,
  selected,
  onFileMenu,
}: {
  task: UploadTask;
  selected: string | null;
  onFileMenu: (x: number, y: number, file: UploadFile) => void;
}) {
  const preparing = t.phase === "Preparing";
  const percent = t.total > 0 ? Math.min(100, Math.floor((t.done / t.total) * 100)) : null;
  const status = preparing ? "Waiting: scanning local changes" : STEP_LABEL[t.step] ?? "Uploading";

  return (
    <div className="rounded-lg bg-white shadow-sm ring-1 ring-slate-200">
      <div className="px-4 pt-3 pb-2">
        <div className="flex items-baseline gap-2 text-xs">
          <span className="truncate text-sm font-medium text-slate-800">{t.repo_name || t.repo_id}</span>
          <span className={preparing ? "text-amber-700" : "text-sky-700"}>{status}</span>
          <div className="flex-1" />
          {!preparing && t.rate > 0 && <span className="whitespace-nowrap text-slate-600">{formatRate(t.rate)}</span>}
          {percent !== null && <span className="w-9 text-right font-medium text-slate-700">{percent}%</span>}
        </div>
        <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-slate-100">
          {percent !== null ? (
            <div className="h-full rounded-full bg-sky-500 transition-[width]" style={{ width: `${percent}%` }} />
          ) : (
            <div className={`h-full w-full animate-pulse rounded-full ${preparing ? "bg-amber-300" : "bg-sky-300"}`} />
          )}
        </div>
        {t.file_count > 0 && (
          <div className="mt-1 text-xs text-slate-500">
            {t.file_count === 1 ? "1 item" : `${t.file_count} items`} in this upload
          </div>
        )}
      </div>

      {t.files.length > 0 && (
        <ul className="max-h-64 divide-y divide-slate-100 overflow-auto border-t border-slate-100">
          {t.files.map((f) => {
            const label = STATUS_LABEL[f.status] ?? { text: f.status, cls: "bg-slate-100 text-slate-600" };
            return (
              <li
                key={`${f.status}:${f.path}`}
                className={`flex items-center gap-2 px-4 py-1 text-xs ${selected === f.path ? "bg-slate-50" : ""}`}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onFileMenu(e.clientX, e.clientY, f);
                }}
              >
                <span className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] ${label.cls}`}>{label.text}</span>
                <span className="truncate font-mono text-slate-600" title={f.path}>
                  {f.path}
                </span>
              </li>
            );
          })}
          {t.file_count > t.files.length && (
            <li className="px-4 py-1 text-xs text-slate-400">…and {t.file_count - t.files.length} more</li>
          )}
        </ul>
      )}
    </div>
  );
}
