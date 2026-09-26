/**
 * Ctrl+K — the primary entry point (M4 scope).
 *
 * It is a list of actions with their bindings, and it exists as much to *teach* the bindings as
 * to invoke them: keyboard-only operation is mandatory, and a binding nobody can discover is a
 * binding nobody uses.
 *
 * It is an internal pane, not a window. The niri constraints are requirements: no popups, no
 * additional toplevels, ever.
 */

import { AnimatePresence, motion } from "framer-motion";
import { Kbd } from "./glass";
import { useEffect, useRef, useState } from "react";
import { spring } from "../motion";

export type Command = {
  id: string;
  label: string;
  binding: string;
  detail?: string;
  run: () => void;
};

export function CommandPalette({
  open,
  commands,
  onClose,
}: {
  open: boolean;
  commands: Command[];
  onClose: () => void;
}) {
  const [cursor, setCursor] = useState(0);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (open) {
      setCursor(0);
      // The palette takes focus so that the same arrow keys drive it rather than the result
      // list underneath. Returning focus is the search input's job on close.
      ref.current?.focus();
    }
  }, [open]);

  if (!open) return null;

  return (
    <AnimatePresence>
      <motion.div
        className="hc-palette glass glass-sheen"
        role="dialog"
        aria-label="Befehle"
        ref={ref}
        tabIndex={-1}
        initial={{ opacity: 0, y: 8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 4, scale: 0.99 }}
        transition={spring("expand")}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            setCursor((c) => Math.min(c + 1, commands.length - 1));
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            setCursor((c) => Math.max(c - 1, 0));
          } else if (event.key === "Enter") {
            event.preventDefault();
            commands[cursor]?.run();
            onClose();
          }
        }}
      >
        {commands.map((command, index) => (
          <div
            key={command.id}
            className={index === cursor ? "hc-command hc-command-selected" : "hc-command"}
            role="button"
            tabIndex={-1}
            aria-current={index === cursor}
            onMouseDown={(e) => {
              e.preventDefault();
              command.run();
              onClose();
            }}
          >
            <span className="hc-row-text">{command.label}</span>
            {command.detail && <span className="hc-meta hc-command-detail">{command.detail}</span>}
            <Kbd>{command.binding}</Kbd>
          </div>
        ))}
      </motion.div>
    </AnimatePresence>
  );
}
