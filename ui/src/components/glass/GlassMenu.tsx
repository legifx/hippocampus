import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { cn } from "../../cn";
import { pop, prefersReducedMotion } from "../../motion";
import { CheckIcon } from "./icons";

/* =====================================================================
   GlassMenu (GlassMenuList + a portal dropdown)
   ---------------------------------------------------------------------
   Ported from `ui/inspo/.../src/glass/GlassMenu.tsx`. The list renderer is used on its
   own by the palette and by anything that draws its own popover; `GlassMenu` is the
   anchor-positioned version, clamped to the viewport and closed by Escape, an outside
   press or a scroll.
   ===================================================================== */

export type MenuItem =
  | {
      type?: "item";
      id: string;
      label: string;
      icon?: ReactNode;
      shortcut?: string;
      danger?: boolean;
      disabled?: boolean;
      checked?: boolean;
      onSelect?: () => void;
    }
  | { type: "separator"; id: string }
  | { type: "label"; id: string; label: string };

/** Static list renderer — used by `GlassMenu` and by callers with their own popover. */
export function GlassMenuList({
  items,
  onSelect,
  className,
}: {
  items: MenuItem[];
  onSelect?: () => void;
  className?: string;
}) {
  return (
    <div className={cn("gm", className)}>
      {items.map((item) => {
        if (item.type === "separator") {
          return <div key={item.id} className="gm-separator" role="separator" />;
        }
        if (item.type === "label") {
          return (
            <div key={item.id} className="gm-label">
              {item.label}
            </div>
          );
        }
        return (
          <button
            key={item.id}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              item.onSelect?.();
              onSelect?.();
            }}
            className={cn("gm-item", item.danger && "gm-item-danger")}
          >
            <span className="gm-item-icon">{item.icon}</span>
            <span className="gm-item-label">{item.label}</span>
            {item.checked ? <CheckIcon className="gm-check" /> : null}
            {item.shortcut ? <span className="gm-item-shortcut">{item.shortcut}</span> : null}
          </button>
        );
      })}
    </div>
  );
}

export interface GlassMenuProps {
  open: boolean;
  /** Anchor point in viewport px. With `align="end"`, x is the right edge. */
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
  align?: "start" | "end";
  minWidth?: number;
  /** Accessible name for the popover. */
  label?: string;
}

export function GlassMenu({
  open,
  x,
  y,
  items,
  onClose,
  align = "start",
  minWidth = 224,
  label,
}: GlassMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    if (!open) return;
    const el = ref.current;
    const w = el?.offsetWidth ?? minWidth;
    const h = el?.offsetHeight ?? 0;
    const left = align === "end" ? x - w : x;
    setPos({
      left: Math.max(8, Math.min(left, window.innerWidth - w - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - h - 8)),
    });
  }, [open, x, y, align, minWidth]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("resize", onClose);
    window.addEventListener("scroll", onClose, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("scroll", onClose, true);
    };
  }, [open, onClose]);

  if (!open) return null;

  // The portal keeps the popover out of any clipping ancestor: the header scrolls
  // horizontally at narrow widths, and a menu inside it would be cut off there.
  return createPortal(
    <AnimatePresence>
      <motion.div
        ref={ref}
        role="menu"
        aria-label={label}
        initial={prefersReducedMotion() ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: -4 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={prefersReducedMotion() ? { opacity: 0 } : { opacity: 0, scale: 0.97, y: -2 }}
        transition={pop()}
        style={{ position: "fixed", left: pos.left, top: pos.top, minWidth }}
        className="glass glass-strong glass-sheen glass-blur rounded-lg gm-popover"
      >
        <GlassMenuList items={items} onSelect={onClose} />
      </motion.div>
    </AnimatePresence>,
    document.body,
  );
}

/** Stop a press inside a menu row from reaching a click handler on the row behind it. */
export function swallow(e: ReactMouseEvent) {
  e.stopPropagation();
}
