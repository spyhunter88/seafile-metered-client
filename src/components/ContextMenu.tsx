import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuItem =
  | { separator: true }
  | {
      separator?: false;
      label: string;
      onSelect: () => void;
      disabled?: boolean;
      danger?: boolean;
      hint?: string;
    };

interface Props {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

/** Right-click menu rendered at viewport coordinates, kept inside the window. */
export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - height - 4)),
    });
  }, [x, y]);

  useEffect(() => {
    const onPointer = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    // Capture so scrolling any container closes the menu.
    window.addEventListener("scroll", onClose, true);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("scroll", onClose, true);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 min-w-48 rounded-md bg-white py-1 text-xs shadow-lg ring-1 ring-slate-200"
      style={pos}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
    >
      {items.map((item, i) =>
        item.separator ? (
          <div key={i} className="my-1 border-t border-slate-100" />
        ) : (
          <button
            key={i}
            role="menuitem"
            type="button"
            title={item.hint}
            disabled={item.disabled}
            className={`block w-full px-3 py-1.5 text-left disabled:text-slate-300 ${
              item.danger ? "text-rose-600 hover:bg-rose-50" : "text-slate-700 hover:bg-slate-100"
            } disabled:hover:bg-transparent`}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}
