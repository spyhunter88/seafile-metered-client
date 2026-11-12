import { useEffect, useState } from "react";
import { api, events, formatRate, type LibrarySyncConfig, type TransferRate } from "../api";

interface Props {
  libraries: LibrarySyncConfig[];
  daemonError: string | null;
}

/** Bottom bar: sync engine state, library counts and current upload/download speed. */
export function StatusBar({ libraries, daemonError }: Props) {
  const [rate, setRate] = useState<TransferRate>({ upload: 0, download: 0 });

  useEffect(() => {
    api.getTransferRate().then(setRate).catch(console.error);
    const sub = events.onTransferRate(setRate);
    return () => {
      sub.then((un) => un());
    };
  }, []);

  const syncing = libraries.filter((l) => l.current_state === "Syncing").length;
  const errors = libraries.filter((l) => typeof l.current_state === "object").length;

  return (
    <footer className="flex items-center gap-4 border-t border-slate-200 bg-white px-4 py-1.5 text-xs text-slate-500">
      <span className="flex items-center gap-1.5" title={daemonError ?? "seaf-daemon is running"}>
        <span className={`h-2 w-2 rounded-full ${daemonError ? "bg-rose-500" : "bg-emerald-500"}`} />
        {daemonError ? "Sync engine stopped" : "Sync engine running"}
      </span>
      <span>
        {libraries.length} synced{syncing > 0 && ` · ${syncing} syncing`}
        {errors > 0 && <span className="text-rose-600"> · {errors} with errors</span>}
      </span>
      <div className="flex-1" />
      <span className="flex items-center gap-3 font-mono tabular-nums" title="Current transfer speed">
        <span className={rate.upload > 0 ? "text-sky-700" : ""}>↑ {formatRate(rate.upload)}</span>
        <span className={rate.download > 0 ? "text-emerald-700" : ""}>↓ {formatRate(rate.download)}</span>
      </span>
    </footer>
  );
}
