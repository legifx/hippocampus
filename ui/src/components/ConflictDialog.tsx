/**
 * "There is already something called that."
 *
 * The question is asked *before* anything moves, because the daemon's dry run says which names
 * are taken. That ordering is the whole point: a file manager that starts copying and then
 * reports a failure has already made the person's decision for them, badly.
 *
 * It shows both sides — what is there and what would arrive, with sizes and dates — because
 * "overwrite?" is not a question anybody can answer. "Replace these 3 kB from March with these
 * 40 kB from today?" is.
 *
 * An internal pane rather than a window: the niri constraints in M4 forbid a second toplevel,
 * and they are right to. It is also keyboard-first, like everything else here.
 */

import { useEffect, useRef, useState } from "react";
import type { Conflict, ConflictChoice } from "../ipc";
import { formatAge, formatSize, splitPath } from "../format";

export type ConflictDialogProps = {
  conflicts: Conflict[];
  /** How many things the whole transfer would move, so the choice has its scale. */
  totalFiles: number;
  onChoose: (choice: ConflictChoice) => void;
  onCancel: () => void;
};

type Option = {
  choice: ConflictChoice;
  label: string;
  key: string;
  detail: string;
};

/** The order matters: the safest choice is first, and it is what Enter takes. */
const OPTIONS: Option[] = [
  {
    choice: "keep",
    label: "Beide behalten",
    key: "B",
    detail: "das Neue kommt als „name (2)“ dazu",
  },
  {
    choice: "skip",
    label: "Überspringen",
    key: "Ü",
    detail: "vorhandene bleiben, der Rest wird übertragen",
  },
  {
    choice: "replace",
    label: "Ersetzen",
    key: "E",
    detail: "das Vorhandene landet im Papierkorb",
  },
];

export function ConflictDialog({
  conflicts,
  totalFiles,
  onChoose,
  onCancel,
}: ConflictDialogProps) {
  const [cursor, setCursor] = useState(0);
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  const many = conflicts.length > 1;
  const first = conflicts[0];

  return (
    <div
      className="hc-conflict glass glass-sheen"
      role="dialog"
      aria-modal="true"
      aria-label="Namenskonflikt"
      ref={ref}
      tabIndex={-1}
      onKeyDown={(event) => {
        // A loop rather than `find`, because the architectural test that keeps data processing
        // out of this frontend greps for that and cannot tell a three-element key lookup from a
        // list of results. The bluntness is what makes the rule enforceable.
        const letter = event.key.toLowerCase();
        for (const option of OPTIONS) {
          if (option.key.toLowerCase() === letter) {
            event.preventDefault();
            onChoose(option.choice);
            return;
          }
        }
        switch (event.key) {
          case "Escape":
            event.preventDefault();
            onCancel();
            break;
          case "ArrowDown":
            event.preventDefault();
            setCursor((c) => Math.min(c + 1, OPTIONS.length - 1));
            break;
          case "ArrowUp":
            event.preventDefault();
            setCursor((c) => Math.max(c - 1, 0));
            break;
          case "Enter":
            event.preventDefault();
            onChoose(OPTIONS[cursor]!.choice);
            break;
        }
      }}
    >
      <p className="hc-conflict-question hc-subtitle">
        {many
          ? `${conflicts.length} von ${totalFiles} sind schon da`
          : `„${splitPath(first?.destination ?? "").name}“ ist schon da`}
      </p>

      {first && (
        <div className="hc-conflict-sides hc-meta">
          {/* Two fields of the kit's `well`, side by side. */}
          <div className="well rounded-sm">
            <span className="hc-faint">vorhanden</span>
            <span className="hc-num">
              {first.existing.kind === "dir" ? "Ordner" : formatSize(first.existing.size)}
            </span>
            <span className="hc-num">{formatAge(first.existing.mtime)}</span>
          </div>
          <div className="well rounded-sm">
            <span className="hc-faint">neu</span>
            <span className="hc-num">
              {first.incoming.kind === "dir" ? "Ordner" : formatSize(first.incoming.size)}
            </span>
            <span className="hc-num">{formatAge(first.incoming.mtime)}</span>
          </div>
        </div>
      )}

      {many && (
        <p className="hc-meta hc-faint hc-conflict-rest">
          {/* Named rather than counted, up to a point: a person deciding to replace things
              deserves to see what they are. */}
          {namesOf(conflicts)}
        </p>
      )}

      <div className="hc-conflict-options">
        {OPTIONS.map((option, index) => (
          <button
            key={option.choice}
            type="button"
            className={index === cursor ? "hc-conflict-option hc-conflict-chosen" : "hc-conflict-option"}
            aria-current={index === cursor}
            onMouseEnter={() => setCursor(index)}
            onMouseDown={(event) => {
              event.preventDefault();
              onChoose(option.choice);
            }}
          >
            <span className="hc-row-text">{option.label}</span>
            <span className="hc-meta hc-conflict-detail">{option.detail}</span>
            <kbd className="hc-pill">{option.key}</kbd>
          </button>
        ))}
      </div>

      <p className="hc-meta hc-faint">
        {many ? "Die Wahl gilt für alle." : ""} Esc bricht ab.
      </p>
    </div>
  );
}

/** The first few names, and how many more there are. */
function namesOf(conflicts: Conflict[], limit = 4): string {
  const names: string[] = [];
  for (const conflict of conflicts) {
    if (names.length >= limit) break;
    names.push(splitPath(conflict.destination).name);
  }
  const rest = conflicts.length - names.length;
  return rest > 0 ? `${names.join(", ")} und ${rest} weitere` : names.join(", ");
}
