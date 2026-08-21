/**
 * "Öffnen mit …" — which application, when the desktop's own answer is not the one wanted.
 *
 * Every row in it was read by the daemon out of the desktop's association files, in the same
 * order and with the same default as `gio` and `xdg-mime` report (D58). That is the point of
 * the screen: it offers what the rest of the session would offer, so choosing here and choosing
 * in another file manager cannot mean different things.
 *
 * Keyboard-first and an internal pane, like [`ConflictDialog`] and for the same two reasons —
 * the M4 constraints forbid a second toplevel window, and a hand is already on the keyboard.
 *
 * Two things it deliberately does *not* do:
 *
 * - It does not order the list. The daemon does, because the order is the desktop's own and a
 *   second opinion about it is D4's data processing in the frontend.
 * - It does not decide what "open" means for an entry. The `Exec` line and its field codes are
 *   expanded in Rust, once, for both the window and the CLI.
 */

import { useEffect, useRef, useState } from "react";
import type { Application } from "../ipc";

export type OpenWithProps = {
  /** What would be opened. Named rather than counted when there is one of it. */
  label: string;
  /** How many files the choice applies to, so "in fünf Fenstern" is not a surprise. */
  count: number;
  /** The type the daemon determined, shown because it is what a default is set *for*. */
  mime: string;
  applications: Application[];
  /** Whether this is the whole installed list already, so the offer to widen it can go away. */
  showingAll: boolean;
  loading: boolean;
  onOpen: (application: Application) => void;
  /** Make it the desktop's default for this type, and open with it. */
  onSetDefault: (application: Application) => void;
  onShowAll: () => void;
  onCancel: () => void;
};

export function OpenWith({
  label,
  count,
  mime,
  applications,
  showingAll,
  loading,
  onOpen,
  onSetDefault,
  onShowAll,
  onCancel,
}: OpenWithProps) {
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  /**
   * Where the cursor is, and which list it is a position *in*.
   *
   * The list can be replaced underneath it — "alle anzeigen" is a different, longer list — and
   * a cursor left where it was would then open whatever happens to occupy that position now.
   *
   * Derived during render rather than reset in an effect, and that is the same lesson D56 paid
   * for: an effect commits one frame with the stale value, and that frame is on screen. Here it
   * is worse than a visual glitch, because Enter in that frame opens an application nobody
   * pointed at. A browser test caught it by failing under load — with the rows already grown to
   * three, the highlight was still on the second row of the old list.
   */
  const [pick, setPick] = useState<{ list: Application[]; index: number }>({
    list: applications,
    index: 0,
  });
  const cursor = pick.list === applications ? pick.index : 0;
  const moveTo = (index: number) => setPick({ list: applications, index });

  const chosen = applications[cursor];

  return (
    <div
      className="hc-openwith"
      role="dialog"
      aria-modal="true"
      aria-label="Öffnen mit"
      ref={ref}
      tabIndex={-1}
      onKeyDown={(event) => {
        switch (event.key) {
          case "Escape":
            event.preventDefault();
            onCancel();
            break;
          case "ArrowDown":
            event.preventDefault();
            moveTo(Math.min(cursor + 1, applications.length - 1));
            break;
          case "ArrowUp":
            event.preventDefault();
            moveTo(Math.max(cursor - 1, 0));
            break;
          case "Home":
            event.preventDefault();
            moveTo(0);
            break;
          case "End":
            event.preventDefault();
            moveTo(Math.max(applications.length - 1, 0));
            break;
          case "Enter":
            event.preventDefault();
            if (!chosen) return;
            // Shift is "and from now on". Two ways to say it rather than a checkbox, because a
            // checkbox in a keyboard dialog is a thing to tab to.
            if (event.shiftKey) onSetDefault(chosen);
            else onOpen(chosen);
            break;
          case "a":
            if (event.ctrlKey || event.metaKey) return;
            if (showingAll) return;
            event.preventDefault();
            onShowAll();
            break;
        }
      }}
    >
      <p className="hc-openwith-question hc-subtitle">
        {count > 1 ? `${count} Dateien öffnen mit …` : `„${label}“ öffnen mit …`}
      </p>
      <p className="hc-meta hc-faint">{mime}</p>

      <div className="hc-openwith-list" role="listbox" aria-label="Anwendungen">
        {loading && <p className="hc-meta hc-faint">sucht …</p>}
        {!loading && applications.length === 0 && (
          <p className="hc-meta hc-faint">
            {showingAll
              ? "keine Anwendung installiert"
              : "nichts beansprucht diesen Typ — A zeigt alle"}
          </p>
        )}
        {applications.map((application, index) => (
          <button
            key={application.id}
            type="button"
            role="option"
            aria-selected={index === cursor}
            data-app={application.id}
            className={
              index === cursor ? "hc-openwith-app hc-openwith-chosen" : "hc-openwith-app"
            }
            onMouseEnter={() => moveTo(index)}
            onMouseDown={(event) => {
              event.preventDefault();
              if (event.shiftKey) onSetDefault(application);
              else onOpen(application);
            }}
          >
            <span className="hc-row-text">{application.name}</span>
            {application.comment && (
              <span className="hc-meta hc-openwith-detail">{application.comment}</span>
            )}
            {application.is_default && <kbd className="hc-pill">Standard</kbd>}
            {application.terminal && <kbd className="hc-pill">Terminal</kbd>}
          </button>
        ))}
      </div>

      <p className="hc-meta hc-faint">
        {count > 1 ? "Enter öffnet alle. " : "Enter öffnet. "}
        Umschalt+Enter merkt sich die Wahl für {mime}.
        {!showingAll && " A zeigt alle Anwendungen."} Esc bricht ab.
      </p>
    </div>
  );
}
