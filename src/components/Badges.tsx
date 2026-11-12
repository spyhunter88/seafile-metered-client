import type { NetworkCost, SyncStatus } from "../api";

export function NetworkBadge({ cost }: { cost: NetworkCost }) {
  const metered = cost === "Metered";
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium ${
        metered ? "bg-amber-100 text-amber-800" : "bg-emerald-100 text-emerald-800"
      }`}
      title="Windows network connection cost"
    >
      <span className={`h-2 w-2 rounded-full ${metered ? "bg-amber-500" : "bg-emerald-500"}`} />
      {metered ? "Metered Hotspot" : "Unmetered"}
    </span>
  );
}

export function StatusBadge({ status }: { status: SyncStatus | null }) {
  let label: string;
  let cls: string;
  let title: string | undefined;

  if (status === null) {
    label = "Not synced";
    cls = "bg-slate-100 text-slate-500";
  } else if (typeof status === "object") {
    label = "Error";
    cls = "bg-rose-100 text-rose-700";
    title = status.Error;
  } else {
    switch (status) {
      case "Synced":
        label = "Synced";
        cls = "bg-emerald-100 text-emerald-700";
        break;
      case "Syncing":
        label = "Syncing…";
        cls = "bg-sky-100 text-sky-700 animate-pulse";
        break;
      case "PausedMetered":
        label = "Paused (Metered)";
        cls = "bg-amber-100 text-amber-800";
        break;
      case "PausedManual":
        label = "Paused (Manual)";
        cls = "bg-slate-200 text-slate-700";
        break;
    }
  }

  return (
    <span className={`inline-block rounded px-2 py-0.5 text-xs font-medium whitespace-nowrap ${cls}`} title={title}>
      {label}
    </span>
  );
}
