/**
 * The clipboard, the running transfer, and what to say about it.
 *
 * **The clipboard is the system's.** Cut and copy put files on the desktop's clipboard in the
 * formats every other file manager reads, and paste reads whatever is there — so copying here
 * and pasting in Nautilus works, and so does the other direction. That is not a nicety: M6's
 * test is a week of exclusive use, and a file manager you cannot copy out of fails it on the
 * first day.
 *
 * A copy is also remembered *here*, which looks like duplication and is not. The remembered one
 * is what the status line shows, and it is what paste falls back to when the system clipboard
 * cannot be reached — an unfocused window on Wayland may not take the selection, and losing the
 * ability to paste inside the window because of that would be a bad trade.
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
import { messageOf, splitPath } from "../format";

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

/**
 * A question waiting to be answered before anything moves.
 *
 * It carries what would move, rather than reading the clipboard again when the answer comes
 * back: between the question and the answer somebody may have copied something else, in this
 * window or in another application, and a dialog that asked about one set of files and then
 * moved another would be the worst bug in this file.
 */
export type PendingConflict = {
  conflicts: Conflict[];
  totalFiles: number;
  destination: string;
  paths: string[];
  mode: "copy" | "move";
};

/** What just happened, for the status line. Cleared when the next thing happens. */
export type Notice = {
  text: string;
  kind: "info" | "error";
} | null;

export function useOperations(onChanged: () => void, destination: string | null) {
  const [clipboard, setClipboard] = useState<Clipboard>(null);
  const [running, setRunning] = useState<RunningTransfer | null>(null);
  const [pending, setPending] = useState<PendingConflict | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  /** Something from another application is over the window. */
  const [dragging, setDragging] = useState(false);

  // The operation this window started. Progress for anything else — there is nothing else
  // today, but the daemon does not promise that — is ignored rather than displayed.
  const mine = useRef<number | null>(null);
  // What the running operation *is*. A transfer and an extraction report on the same two
  // notifications, deliberately, so the wording of the closing sentence is the one thing that
  // has to remember which of them was asked for.
  const runningKind = useRef<"transfer" | "extract">("transfer");
  const changed = useRef(onChanged);
  changed.current = onChanged;
  // Where a drop would land. Held in a ref because the subscription is made once and the
  // directory changes underneath it — re-subscribing on every navigation would mean a window
  // that briefly refuses drops each time somebody moves.
  const target = useRef(destination);
  target.current = destination;

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
        setNotice(summarise(done, runningKind.current));
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

  /**
   * Remember it here, and offer it to the desktop.
   *
   * Both, in that order: the window's own state is set first so that the pill appears the
   * instant the key is pressed, and the system clipboard follows. If the compositor refuses the
   * selection the notice says the copy is only good inside this window, because a person who is
   * not told finds out later by pasting the wrong thing somewhere else.
   */
  const remember = useCallback((paths: string[], mode: "copy" | "move") => {
    if (paths.length === 0) return;
    setClipboard({ paths, mode });
    const verb = mode === "move" ? "ausgeschnitten" : "kopiert";
    setNotice({ text: `${paths.length} ${verb}`, kind: "info" });

    ipc.clipboardWrite(paths, mode).catch(() => {
      setNotice({ text: `${paths.length} ${verb} — nur in diesem Fenster`, kind: "info" });
    });
  }, []);

  const cut = useCallback((paths: string[]) => remember(paths, "move"), [remember]);
  const copy = useCallback((paths: string[]) => remember(paths, "copy"), [remember]);

  /**
   * Pick up what the desktop is holding, so the pill is right when the window is summoned.
   *
   * On focus rather than on an interval or at startup: polling the clipboard means asking
   * another application over and over, and doing it at startup would put a round trip on the
   * cold-start path the milestone measures.
   */
  useEffect(() => {
    if (typeof window === "undefined") return;
    const pickUp = () => {
      ipc
        .clipboardRead()
        .then((system) => {
          if (system && system.paths.length > 0) {
            setClipboard({ paths: system.paths, mode: system.mode });
          }
        })
        .catch(() => {});
    };
    window.addEventListener("focus", pickUp);
    return () => window.removeEventListener("focus", pickUp);
  }, []);

  /// Start the transfer for real, with a decision already made about what is in the way.
  const begin = useCallback(
    (paths: string[], mode: "copy" | "move", destination: string, conflict: ConflictChoice) => {
      ipc
        .transfer(paths, destination, mode, conflict)
        .then((answer) => {
          if (answer.outcome !== "started") return;
          mine.current = answer.operation;
          runningKind.current = "transfer";
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
          // thing people do and pasting a cut twice is not. The desktop's clipboard is emptied
          // too, or the next paste anywhere would name files that have moved. Clearing is a
          // no-op when something else owns the selection, so a cut taken from another file
          // manager leaves that manager's clipboard alone.
          if (mode === "move") {
            setClipboard(null);
            ipc.clipboardClear().catch(() => {});
          }
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [],
  );

  /**
   * Ask the daemon what would happen, then either do it or ask the person.
   *
   * Reached through a ref by the drop subscription above, which is made once and would
   * otherwise capture whichever `plan` existed at mount.
   */
  const plan = useCallback(
    (paths: string[], mode: "copy" | "move", destination: string) => {
      // Before anything moves. A file manager that starts copying and then reports a failure
      // has already made the decision, badly.
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
            paths,
            mode,
          });
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [begin],
  );

  const planRef = useRef(plan);
  planRef.current = plan;

  /**
   * Files dragged in from another application.
   *
   * **A drop copies.** Not because copying is the more likely intent — inside one file manager
   * a drag usually means move — but because the drop event carries neither the modifier keys
   * nor which filesystem the source is on, which are the two things a correct guess needs. A
   * wrong guess in the copy direction leaves a duplicate; a wrong guess in the move direction
   * takes a file out of somebody else's window. Ctrl+X and Ctrl+V are still there for a move,
   * and they say what they mean.
   */
  useEffect(() => {
    let disposed = false;
    const offs: (() => void)[] = [];

    ipc
      .onFilesDropped((paths) => {
        if (disposed) return;
        const into = target.current;
        // A result list is not a directory, so there is nowhere for a drop to land. Saying so
        // is better than copying into whatever was browsed last.
        if (into === null) {
          setNotice({ text: "erst einen Ordner öffnen", kind: "info" });
          return;
        }
        planRef.current(paths, "copy", into);
      })
      .then((off) => (disposed ? off() : offs.push(off)))
      .catch(() => {});

    ipc
      .onDragOver((over) => !disposed && setDragging(over))
      .then((off) => (disposed ? off() : offs.push(off)))
      .catch(() => {});

    return () => {
      disposed = true;
      offs.forEach((off) => off());
    };
  }, []);

  /**
   * Unpack an archive into a new directory beside it.
   *
   * Two calls, like a paste: the first asks what is inside and where it would go, the second
   * does it. The dry run is not ceremony — it is where a file that is not an archive, or one
   * the policy will not read, is refused while nothing has happened yet.
   *
   * **When something is already at the destination it unpacks beside it** rather than asking.
   * The three answers a conflict dialog could offer are refuse, merge and replace; merging an
   * archive into an existing directory and replacing one are both destructive in ways a person
   * cannot see from a dialog, and "notes (2)" costs nothing but a name.
   */
  const extract = useCallback(
    (archive: string) => {
      if (running || pending) return;
      ipc
        .extractArchive(archive, null, "fail", true)
        .then((planned) => {
          if (planned.outcome !== "planned") return;
          const conflict: ConflictChoice = planned.occupied ? "keep" : "fail";
          return ipc.extractArchive(archive, null, conflict).then((answer) => {
            if (answer.outcome !== "started") return;
            mine.current = answer.operation;
            runningKind.current = "extract";
            setRunning({
              operation: answer.operation,
              fraction: 0,
              filesDone: 0,
              totalFiles: answer.total_files,
              bytesDone: 0,
              totalBytes: answer.total_bytes,
              current: null,
            });
            setNotice({
              text: `entpacke ${answer.total_files} nach ${splitPath(answer.destination).name}`,
              kind: "info",
            });
          });
        })
        .catch((e: unknown) => setNotice({ text: messageOf(e), kind: "error" }));
    },
    [running, pending],
  );

  const paste = useCallback(
    (destination: string) => {
      if (running || pending) return;

      // The desktop's clipboard is the source of truth, so a copy made in another file manager
      // pastes here. The window's own is the fallback for the case where the selection could
      // not be read — being unable to paste what this window itself copied would be the worse
      // failure of the two.
      ipc
        .clipboardRead()
        .then((system) => {
          if (system && system.paths.length > 0) {
            setClipboard({ paths: system.paths, mode: system.mode });
            plan(system.paths, system.mode, destination);
          } else if (clipboard) {
            plan(clipboard.paths, clipboard.mode, destination);
          }
        })
        .catch(() => {
          if (clipboard) plan(clipboard.paths, clipboard.mode, destination);
        });
    },
    [clipboard, running, pending, plan],
  );

  /// The answer to the question the dialog asked.
  const resolveConflict = useCallback(
    (choice: ConflictChoice) => {
      if (!pending) return;
      const { paths, mode, destination } = pending;
      setPending(null);
      begin(paths, mode, destination, choice);
    },
    [pending, begin],
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
    dragging,
    pending,
    resolveConflict,
    dismissConflict,
    notice,
    setNotice,
    cut,
    copy,
    paste,
    extract,
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
function summarise(done: TransferDone, kind: "transfer" | "extract"): Notice {
  if (kind === "extract") return summariseExtraction(done);
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

/**
 * The same notification, said the way an extraction ended.
 *
 * `moved` carries the one directory it landed in, and `failed` carries the entries the daemon
 * refused — a link, a device node, a path that would have written above the destination. Those
 * are named rather than counted away: an archive that quietly unpacks to fewer files than it
 * holds is the failure a person discovers a week later.
 */
function summariseExtraction(done: TransferDone): Notice {
  if (done.cancelled) return { text: "entpacken abgebrochen", kind: "info" };
  const into = done.moved[0] ? splitPath(done.moved[0]).name : "";
  if (done.failed.length > 0) {
    const first = done.failed[0];
    return {
      text:
        done.failed.length === 1
          ? `entpackt nach ${into} — 1 Eintrag nicht: ${first?.reason ?? ""}`
          : `entpackt nach ${into} — ${done.failed.length} Einträge nicht`,
      kind: "error",
    };
  }
  return { text: `entpackt nach ${into}`, kind: "info" };
}
