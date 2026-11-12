import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

type ToastKind = "info" | "success" | "warning" | "error";

interface Toast {
  id: number;
  kind: ToastKind;
  title: string;
  body?: string;
}

type PushToast = (kind: ToastKind, title: string, body?: string) => void;

const ToastContext = createContext<PushToast>(() => {});

const kindStyles: Record<ToastKind, string> = {
  info: "border-sky-300 bg-sky-50 text-sky-900",
  success: "border-emerald-300 bg-emerald-50 text-emerald-900",
  warning: "border-amber-300 bg-amber-50 text-amber-900",
  error: "border-rose-300 bg-rose-50 text-rose-900",
};

let nextId = 1;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const dismiss = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
  }, []);

  const push = useCallback<PushToast>(
    (kind, title, body) => {
      const id = nextId++;
      setToasts((t) => [...t.slice(-4), { id, kind, title, body }]);
      setTimeout(() => dismiss(id), kind === "error" ? 8000 : 5000);
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-80 flex-col gap-2">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`pointer-events-auto cursor-pointer rounded-lg border px-4 py-3 shadow-md ${kindStyles[t.kind]}`}
            onClick={() => dismiss(t.id)}
          >
            <div className="text-sm font-semibold">{t.title}</div>
            {t.body && <div className="mt-0.5 text-xs break-words opacity-80">{t.body}</div>}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast(): PushToast {
  return useContext(ToastContext);
}
