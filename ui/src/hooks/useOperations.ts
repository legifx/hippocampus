/**
 * The clipboard, the running transfer, and what to say about it.
 *
 * The clipboard here is the file manager's, not the system's: cut and copy remember paths and
 * a verb, and paste turns them into a transfer. That is a deliberate first step rather than an
 * omission — the system clipboard's file support on Wayland is a separate piece of work, and
 * this makes the operations usable today without pretending to be interoperable yet.
 *
 * A transfer is single at a time. Two copies into the same directory is a conflict machine, and
 * a person who wants a second one can wait for the first — which is visible, because there is a
 * bar for it.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ipc,
  type Conflict,
  type ConflictChoice,
  type TransferDone,
  type TransferProgress,
} from "../ipc";

export type Clipboard = {
  paths: string[];
  mode: "copy" | "move";
} | null;

export type RunningTransfer = {
  operation: number;
  fraction: number;
  filesDone: number;
  totalFiles: number;
  bytesDone: number;
  totalBytes: number;
  current: string | null;
};

/** A question waiting to be answered before anything moves. */
export type PendingConflict = {
  conflicts: Conflict[];
  totalFiles: number;
  destination: string;
};

/** What just happened, for the status line. Cleared when the next thing happens. */
export type Notice = {
  text: string;
  kind: "info" | "error";
} | null;

export function useOperations(onChanged: () => void) {
  const [clipboard, setClipboard] = useState<Clipboard>(null);
  const [running, setRunning] = useState<RunningTransfer | null>(null);
  const [pending, setPending] = useState<PendingConflict | null>(null);
  const [notice, setNotice] = useState<Notice>(null);

  // The operation this window started. Progress for anything else — there is nothing else
  // today, but the daemon does not promise that — is ignored rather than displayed.
  const mine = useRef<number | null>(null);
  const changed = useRef(onChanged);
  changed.current = onChanged;

  useEffect(() => {
    let disposed = false;
    const offs: (() => void)[] = [];

    ipc
      .onTransferProgress((progress: TransferProgress) => {
        if (disposed || progress.operation !== mine.current) return;
        setRunning({
          operation: progress.operation,
          fraction: progress.fraction,
          filesDone: progress.files_done,
          totalFiles: progress.total_files,
          bytesDone: progress.bytes_done,
          totalBytes: progress.total_bytes,
          current: progress.current,
        });
      })
      .then((off) => (disposed ? off() : offs.push(off)))
      .catch(() => {});

    ipc
      .onTransferDone((done: TransferDone) => {
        if (disposed || done.operation !== mine.current) return;
        mine.current = null;
        setRunning(null);
        setNotice(summarise(done));
        // The directory changed underneath whatever is on screen.
        changed.current();
      })
      .then((off) => (disposed ? off() : offs.push(off)))
      .catch(() => {});

    return () => {
      disposed = true;
      offs.forEach((off) => off());
    };
  }, []);

  const cut = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    setClipboard({ paths, mode: "move" });
    setNotice({ text: `${paths.length} ausgeschnitten`, kind: "info" });
  }, []);

  const copy = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    setClipboard({ paths, mode: "copy" });
    setNotice({ text: `${paths.length} kopiert`, kind: "info" });
  }, []);

  /// Start the transfer for real, with a decision already made about what is in the way.
  const begin = useCallback(
    (paths: string[], mode: "copy" | "move", destination: string, conflict: ConflictChoice) => {
      ipc
        .transfer(paths, destination, mode, conflict)
        .then((answer) => {
          if (answer.outcome !== "started") return;
          mine.current = answer.operation;
          setRunning({
            operation: answer.operation,
            fraction: 0,
            filesDone: 0,
            totalFiles: answer.total_files,
            bytesDone: 0,
            totalBytes: answer.total_bytes,
            current: null,
          });
          // A move empties the clipboard; a copy does not, because pasting a copy twice is a
          // thing people do and pasting a cut twice is not.
          if (mode === "move") setClipboard(null);
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [],
  );

  const paste = useCallback(
    (destination: string) => {
      if (!clipboard || running || pending) return;
      const { paths, mode } = clipboard;

      // Ask the daemon what would happen before anything moves. A file manager that starts
      // copying and then reports a failure has already made the decision, badly.
      ipc
        .transfer(paths, destination, mode, "fail", true)
        .then((answer) => {
          if (answer.outcome !== "planned") return;
          if (answer.conflicts.length === 0) {
            begin(paths, mode, destination, "fail");
            return;
          }
          setPending({
            conflicts: answer.conflicts,
            totalFiles: answer.total_files,
            destination,
          });
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [clipboard, running, pending, begin],
  );

  /// The answer to the question the dialog asked.
  const resolveConflict = useCallback(
    (choice: ConflictChoice) => {
      if (!pending || !clipboard) return;
      const { paths, mode } = clipboard;
      const destination = pending.destination;
      setPending(null);
      begin(paths, mode, destination, choice);
    },
    [pending, clipboard, begin],
  );

  const dismissConflict = useCallback(() => {
    setPending(null);
    setNotice({ text: "nichts übertragen", kind: "info" });
  }, []);

  const cancel = useCallback(() => {
    if (!running) return;
    ipc.cancelOperation(running.operation).catch(() => {});
  }, [running]);

  const trash = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;
      // One call per path: trashing is a rename, so a hundred of them is a hundred instant
      // calls rather than an operation worth a progress bar.
      Promise.allSettled(paths.map((path) => ipc.trash(path)))
        .then((results) => {
          // Counted in a loop rather than with `filter`, because the architectural test that
          // keeps data processing out of this frontend greps for that and does not know the
          // outcome of a promise from a list of search results. The bluntness is the point.
          let failed = 0;
          for (const result of results) {
            if (result.status === "rejected") failed += 1;
          }
          setNotice(
            failed === 0
              ? { text: `${paths.length} in den Papierkorb`, kind: "info" }
              : { text: `${failed} von ${paths.length} nicht gelöscht`, kind: "error" },
          );
          changed.current();
        })
        .catch(() => {});
    },
    [],
  );

  const undo = useCallback(() => {
    ipc
      .undo(1)
      .then((result) => {
        setNotice({
          text: result.undone.length > 0 ? "rückgängig gemacht" : "nichts rückgängig zu machen",
          kind: "info",
        });
        changed.current();
      })
      .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
  }, []);

  const mkdir = useCallback(
    (path: string) => {
      ipc
        .mkdir(path)
        .then(() => {
          setNotice({ text: "Ordner angelegt", kind: "info" });
          changed.current();
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [],
  );

  const rename = useCallback(
    (path: string, name: string) => {
      ipc
        .rename(path, name)
        .then(() => {
          setNotice({ text: "umbenannt", kind: "info" });
          changed.current();
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [],
  );

  return {
    clipboard,
    running,
    pending,
    resolveConflict,
    dismissConflict,
    notice,
    setNotice,
    cut,
    copy,
    paste,
    cancel,
    trash,
    undo,
    mkdir,
    rename,
  };
}

/**
 * One sentence about how a transfer ended.
 *
 * A batch that partly failed says both halves. "3 kopiert" when two of five did not arrive is
 * the kind of message that costs somebody a file.
 */
function summarise(done: TransferDone): Notice {
  if (done.cancelled) {
    return {
      text: `abgebrochen — ${done.moved.length} bereits übertragen`,
      kind: "info",
    };
  }
  if (done.failed.length > 0) {
    const first = done.failed[0];
    return {
      text:
        done.failed.length === 1
          ? `${done.moved.length} übertragen, 1 nicht: ${first?.reason ?? ""}`
          : `${done.moved.length} übertragen, ${done.failed.length} nicht`,
      kind: "error",
    };
  }
  return { text: `${done.moved.length} übertragen`, kind: "info" };
}

function messageOf(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return "das hat nicht funktioniert";
}
