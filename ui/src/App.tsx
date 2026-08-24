/**
 * The overlay.
 *
 * One window, one input, one list, and two side panes that appear when there is room for them.
 * The niri constraints are requirements rather than preferences: the layout works from 400 px
 * to full width, there are no popups and no second toplevel, and every action has a binding.
 *
 * Layout, by available width — a tiling column is often narrow, so this is the primary axis:
 *
 * | width   | what is shown                        |
 * |---------|--------------------------------------|
 * | < 620   | results only; panes on a binding     |
 * | 620–980 | results plus one pane                |
 * | ≥ 980   | results, preview and context         |
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Ambient } from "./components/Ambient";
import { Breadcrumb } from "./components/Breadcrumb";
import { AgentInspector, type InspectorTab } from "./components/AgentInspector";
import { CommandPalette, type Command } from "./components/CommandPalette";
import { ConflictDialog } from "./components/ConflictDialog";
import { OpenWith } from "./components/OpenWith";
import { ContextSidebar } from "./components/ContextSidebar";
import { PlacesView, useFlatPlaces } from "./components/PlacesView";
import { usePlaces, PLACES } from "./hooks/usePlaces";
import { useVisibility } from "./hooks/useVisibility";
import { PreviewPane } from "./components/PreviewPane";
import { ResultList } from "./components/ResultList";
import { useBrowse } from "./hooks/useBrowse";
import { useOperations } from "./hooks/useOperations";
import { useSelection } from "./hooks/useSelection";
import { throttled, useDeferred } from "./hooks/useDeferred";
import { useSearch } from "./hooks/useSearch";
import { useViewport } from "./hooks/useViewport";
import {
  ipc,
  NEEDS_AUTHORISATION,
  type Application,
  type AppsForPath,
  type Place,
  type SearchHit,
  type SortBy,
} from "./ipc";
import { isArchive, rowFromEntry, rowFromHit, type Row } from "./row";
import { codeOf, messageOf } from "./format";

const ROW_HEIGHT_COMPACT = 28;
const ROW_HEIGHT_COMFORTABLE = 34;

/** Which side pane is showing when there is room for only one. */
type Pane = "preview" | "context";

/**
 * Put a name inside a directory.
 *
 * `/a/b` and `/a/b/` are the same directory and must produce the same path — and the root is
 * the case that catches a naive join, because `"/" + "/" + name` is two slashes and a daemon
 * that resolves it is being polite about a bug.
 */
function joinPath(directory: string, name: string): string {
  return directory.endsWith("/") ? `${directory}${name}` : `${directory}/${name}`;
}

function splitPathName(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut < 0 ? path : path.slice(cut + 1);
}

/** The daemon needs to know somebody is here, not how fast they type. */
const pingSometimes = throttled(5000);

const SORT_LABELS: Record<SortBy, string> = {
  name: "Name",
  size: "Größe",
  modified: "Datum",
  none: "unsortiert",
};

/** What an empty list says. Which of the two modes is empty changes what is worth saying. */
function emptyMessage(
  searching: boolean,
  loading: boolean,
  browse: { loading: boolean; error: string | null },
): string {
  if (loading || browse.loading) return "…";
  if (browse.error) return browse.error;
  return searching ? "Nichts gefunden" : "Dieser Ordner ist leer";
}

/// Where browsing starts. The home directory is the only defensible guess before the daemon
/// has been asked which roots it indexes, and it is what every file manager opens on.
/**
 * Where the overlay opens.
 *
 * Not `/`. The filesystem root is where a file manager opens when nobody decided where it should
 * open, and what it shows first is sixteen directories of which one is ever wanted. The places
 * screen shows the folders somebody actually uses, the drives, and one row for everything
 * technical — see `PlacesView` and D52.
 */
const START_PATH = PLACES;

export type AppProps = {
  /**
   * Where to open, for the browser tests.
   *
   * Most of them are about browsing a directory, and starting them at the places screen would
   * mean every one began by navigating out of it — which tests the navigation over and over and
   * the thing they are about once.
   */
  startPath?: string;
};

/** One array rather than a fresh one per render: the chooser resets its cursor when the list it
 * is given is a different list, and `[]` written twice is two different lists. */
const NO_APPLICATIONS: Application[] = [];

export function App({ startPath = START_PATH }: AppProps = {}) {
  const { state, run } = useSearch();
  const browse = useBrowse(startPath);
  const [query, setQuery] = useState("");
  const [pane, setPane] = useState<Pane>("preview");
  const [paletteOpen, setPaletteOpen] = useState(false);
  // The Agent Inspector. Which tab is held here rather than inside it, so that closing the
  // panel and opening it again lands where it was left — a person checking on a running agent
  // does that several times a minute.
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("live");
  /// The row being renamed, and the name of a folder being created. Both are one small input
  /// in the header rather than a dialog: M4's niri constraints forbid a second toplevel, and a
  /// modal over the list would hide the thing being named.
  const [renaming, setRenaming] = useState<Row | null>(null);
  const [creating, setCreating] = useState<string | null>(null);
  const [comfortable, setComfortable] = useState(false);
  const [zoneHues, setZoneHues] = useState<Map<number, number>>(new Map());
  const [load, setLoad] = useState(0);
  const [daemonError, setDaemonError] = useState<string | null>(null);

  const input = useRef<HTMLInputElement | null>(null);
  const width = useViewport();

  const showPreview = width >= 620;
  const showContext = width >= 980 || (width >= 620 && pane === "context");
  const showBoth = width >= 980;

  const rowHeight = comfortable ? ROW_HEIGHT_COMFORTABLE : ROW_HEIGHT_COMPACT;

  // Two modes, one list: a directory when nothing is typed, results when something is (M6).
  const searching = query.trim() !== "";
  // Nothing periodic runs while nobody is looking. See `useVisibility` for what that was
  // costing before it was measured.
  const visible = useVisibility();

  // Three screens, one window: the places, a directory, or results. Typing always wins — that
  // is M6's shape, "search as an overlay, not a separate page".
  const atPlaces = !searching && browse.state.path === PLACES;
  const places = usePlaces(atPlaces);
  const flatPlaces = useFlatPlaces(places.sections);
  const rows: Row[] = useMemo(
    () => (searching ? state.hits.map(rowFromHit) : browse.state.entries.map(rowFromEntry)),
    [searching, state.hits, browse.state.entries],
  );
  // The selection resets whenever the list underneath it changes: a different directory, or a
  // different query. Otherwise "delete the three selected" would delete whatever landed in
  // those three positions.
  const selection = useSelection(rows.length, `${searching}:${browse.state.path}:${query}`);
  const selected = selection.state.cursor;
  const current: Row | null = rows[selected] ?? null;

  /** What an action acts on: everything selected, in the order the list shows it. */
  const targets = useMemo(() => {
    const out: string[] = [];
    for (let i = 0; i < rows.length; i++) {
      if (selection.state.indices.has(i)) out.push(rows[i]!.path);
    }
    return out;
  }, [rows, selection.state.indices]);

  // The directory a drop lands in. Null while searching: a result list is not a place.
  const operations = useOperations(browse.reload, searching ? null : browse.state.path);

  // The panes want a `SearchHit`, which a directory entry is not. Only a search result carries
  // a score and a tier, so a browsed row is given the shape with those left honestly empty.
  const hit: SearchHit | null = useMemo(() => {
    if (!current) return null;
    const fromSearch = searching ? state.hits[selected] : undefined;
    if (fromSearch) return fromSearch;
    return {
      node_id: -1,
      path: current.path,
      kind: current.isDir ? "dir" : "file",
      size: current.size,
      mtime: current.mtime,
      zone_id: current.zoneId,
      tier: current.tier ?? "stm",
      score: {
        path_rank: null,
        lexical_rank: null,
        vector_rank: null,
        fused: 0,
        salience_boost: 1,
        rerank: null,
        final_score: 0,
      },
      snippet: null,
    };
  }, [current, searching, state.hits, selected]);

  // The list follows the arrow key; the panes follow the list once it stops. Holding a key
  // otherwise fires one `brain.neighbors` and one preview render per row passed over, and
  // every one of them describes a row nobody looked at.
  // The hit and the row it came from settle together, as one value: they describe the same
  // file, and two timers could let the sidebar's title and its permissions disagree for a
  // frame while an arrow key is held down.
  const focus = useMemo(() => ({ hit, row: current }), [hit, current]);
  const settledFocus = useDeferred(focus);
  const settled = settledFocus.hit;

  // The first frame a person can act on. M4 §1's timer stops here.
  useEffect(() => {
    ipc.markInteractive().catch(() => {});
    input.current?.focus();
  }, []);

  // Summoned again: empty input, no selection, caret ready. The window is resident, so without
  // this the overlay would come back holding whatever was searched for last time.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    ipc
      .onWindowShown(() => {
        if (disposed) return;
        setQuery("");
        setPaletteOpen(false);
        run("");
        input.current?.focus();
      })
      .then((off) => (disposed ? off() : (unlisten = off)))
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [run]);

  useEffect(() => {
    let disposed = false;
    ipc
      .onDaemonUnavailable((message) => !disposed && setDaemonError(message))
      .then((off) => disposed && off())
      .catch(() => {});

    ipc
      .zones()
      .then((result) => {
        if (disposed) return;
        // The hue is the daemon's, derived from the zone's identity (D15). Deriving it again
        // here would be a second implementation of a value that has to be stable across clients.
        setZoneHues(new Map(result.zones.map((zone) => [zone.id, zone.hue])));
      })
      .catch(() => {});

    return () => {
      disposed = true;
    };
  }, []);



  // The indexing heartbeat, on its own timer, alive only while the window is.
  //
  // This used to run every four seconds, round the clock. It is two full table scans in the
  // daemon — over a hundred thousand nodes and as many chunks — and the overlay is resident,
  // so it was running for a hidden window, to set the period of an animation nobody could see.
  // The journal has it at up to 988 ms per call while the indexer was writing, blocking every
  // other call behind it on the same socket.
  useEffect(() => {
    if (!visible) return;
    let disposed = false;

    const poll = () => {
      ipc
        .extractStatus()
        .then((status) => {
          if (disposed) return;
          const outstanding = status.pending;
          setLoad(outstanding === 0 ? 0 : Math.min(1, Math.log10(outstanding + 1) / 4));
        })
        .catch(() => {});
    };
    poll();
    const timer = setInterval(poll, 8000);

    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [visible]);

  const onQuery = useCallback(
    (next: string) => {
      setQuery(next);
      run(next);
      // A person typing here is the clearest evidence a human is present, which is what the
      // daemon's duty cycle keys off (D22) — but the first keystroke establishes that, and the
      // next forty repeat it at the cost of forty round trips.
      pingSometimes(() => void ipc.activityPing().catch(() => {}));
    },
    [run],
  );

  /// Enter, and a double click: descend into a directory, hand a file to the desktop.
  const activate = useCallback(
    (row: Row) => {
      if (row.isDir) {
        setQuery("");
        run("");
        browse.goTo(row.path);
        return;
      }
      ipc.openPath(row.path).catch(() => {});
    },
    [browse, run],
  );

  const openSelected = useCallback(() => {
    if (current) activate(current);
  }, [current, activate]);

  // "Öffnen mit …": what the desktop offers for this type, so a choice made here means the same
  // thing as a choice made anywhere else on the desktop (D58). The list comes from the daemon;
  // this only remembers which files the answer applies to.
  const [chooser, setChooser] = useState<{ paths: string[]; label: string } | null>(null);
  const [choices, setChoices] = useState<AppsForPath | null>(null);
  const [choicesAll, setChoicesAll] = useState(false);
  const [choicesLoading, setChoicesLoading] = useState(false);

  const askWhichApplication = useCallback(
    (paths: string[], label: string, all = false) => {
      if (paths.length === 0) return;
      setChooser({ paths, label });
      setChoicesAll(all);
      setChoicesLoading(true);
      // Widening the list keeps the narrow one on screen until the wider one arrives, so there
      // is no frame in which the chooser is empty and Enter chooses nothing. A different
      // *file*, though, means the old list is about something else and has to go.
      if (!all) setChoices(null);
      // The type is the *first* file's. Opening a mixed selection with one application is a
      // thing people do — five images and a stray note — and the alternative, refusing until
      // the selection is uniform, is a dialog explaining a rule instead of opening the files.
      ipc
        .appsForPath(paths[0]!, all)
        .then((found) => setChoices(found))
        .catch(() => setChoices(null))
        .then(() => setChoicesLoading(false));
    },
    [],
  );

  const closeChooser = useCallback(() => {
    setChooser(null);
    setChoices(null);
    setChoicesAll(false);
    setChoicesLoading(false);
    input.current?.focus();
  }, []);

  const openWithChoice = useCallback(
    (application: Application) => {
      if (!chooser) return;
      ipc.openWith(application, chooser.paths).catch(() => {});
      closeChooser();
    },
    [chooser, closeChooser],
  );

  /** The selection, or the row under the cursor when nothing is selected. */
  const openWithSelected = useCallback(() => {
    const paths = targets.length > 0 ? targets : current ? [current.path] : [];
    const label = targets.length > 1 ? `${targets.length} Dateien` : current?.name ?? "";
    askWhichApplication(paths, label);
  }, [targets, current, askWhichApplication]);

  /**
   * Unpack what the cursor is on. Silent when it is not an archive, because the binding is
   * reached by pressing a key rather than by choosing a menu entry — a notice saying "that is
   * not an archive" for every stray Ctrl+E would be noise for a mistake nobody makes twice.
   */
  const extractSelected = useCallback(() => {
    if (current && isArchive(current.name)) operations.extract(current.path);
  }, [current, operations]);

  const commands: Command[] = useMemo(
    () => [
      { id: "open", label: "Öffnen", binding: "Enter", run: openSelected },
      {
        id: "agents",
        label: "Agenten-Inspektor",
        binding: "Ctrl+Shift+A",
        detail: "Sitzungen, Gedächtnis, Regeln, Rückgängig, Architektur",
        run: () => setInspectorOpen(true),
      },
      {
        id: "open-with",
        label: "Öffnen mit …",
        binding: "Ctrl+Enter",
        detail: "eine andere Anwendung, oder eine neue Standardwahl",
        run: () => openWithSelected(),
      },
      {
        id: "copy",
        label: "Pfad kopieren",
        binding: "Ctrl+C",
        run: () => hit && navigator.clipboard?.writeText(hit.path).catch(() => {}),
      },
      {
        id: "pane",
        label: pane === "preview" ? "Kontext zeigen" : "Vorschau zeigen",
        binding: "Tab",
        detail: showBoth ? "beide sind sichtbar" : undefined,
        run: () => setPane((p) => (p === "preview" ? "context" : "preview")),
      },
      {
        id: "density",
        label: comfortable ? "Kompakte Zeilen" : "Bequeme Zeilen",
        binding: "Ctrl+D",
        run: () => setComfortable((c) => !c),
      },
      {
        id: "startup",
        label: "Startzeit anzeigen",
        binding: "—",
        detail: "misst den Kaltstart dieses Fensters",
        run: () => {
          ipc
            .startupReport()
            .then((report) =>
              setDaemonError(
                report.waited_for_daemon_ms > 0
                  ? `Kaltstart ${report.interactive_ms} ms — davon ` +
                      `${report.waited_for_daemon_ms} ms Warten auf den Dienst, also nicht ` +
                      `mit dem Budget von ${report.budget_ms} ms vergleichbar`
                  : `Kaltstart ${report.interactive_ms} ms (Budget ${report.budget_ms} ms) — ` +
                      `Verbindung nach ${report.connected_ms} ms`,
              ),
            )
            .catch(() => {});
        },
      },
      ...(searching
        ? []
        : (["name", "size", "modified"] as SortBy[]).map((order) => ({
            id: `sort-${order}`,
            label: `Sortieren: ${SORT_LABELS[order]}`,
            binding: "—",
            detail: browse.state.sort === order ? "nochmal wählen kehrt um" : undefined,
            run: () => browse.orderBy(order),
          }))),
      ...(current && isArchive(current.name)
        ? [
            {
              id: "extract",
              label: "Entpacken",
              binding: "Ctrl+E",
              detail: "in einen neuen Ordner daneben",
              run: extractSelected,
            },
          ]
        : []),
      { id: "cut", label: "Ausschneiden", binding: "Ctrl+X", run: () => operations.cut(targets) },
      { id: "copyfiles", label: "Kopieren", binding: "Ctrl+C", run: () => operations.copy(targets) },
      {
        id: "paste",
        label: "Einfügen",
        binding: "Ctrl+V",
        detail: operations.clipboard ? undefined : "die Zwischenablage ist leer",
        run: () => !searching && operations.paste(browse.state.path),
      },
      { id: "trash", label: "In den Papierkorb", binding: "Entf", run: () => operations.trash(targets) },
      {
        id: "rename",
        label: "Umbenennen",
        binding: "F2",
        run: () => current && setRenaming(current),
      },
      {
        id: "mkdir",
        label: "Neuer Ordner",
        binding: "Ctrl+Shift+N",
        run: () => !searching && setCreating(""),
      },
      { id: "undo", label: "Rückgängig", binding: "Ctrl+Z", run: () => operations.undo() },
      { id: "dismiss", label: "Schließen", binding: "Esc", run: () => ipc.dismiss().catch(() => {}) },
    ],
    [
      openSelected,
      hit,
      pane,
      showBoth,
      comfortable,
      searching,
      browse,
      operations,
      targets,
      current,
      extractSelected,
    ],
  );

  const openPlace = useCallback(
    (place: Place | undefined) => {
      if (!place) return;
      // A drive that is not mounted has no directory to go to: its path is a device node. So
      // opening it is two steps, and the second one uses the mount point the daemon answers
      // with rather than a path guessed from the label — udisks2 chooses where it lands, and
      // guessing would send the window somewhere empty.
      if (place.volume && !place.volume.mounted) {
        const arrived = (mounted: { mount_point: string }) => {
          setDaemonError(null);
          places.reload();
          browse.goTo(mounted.mount_point);
        };

        setDaemonError(`${place.name} wird eingehängt …`);
        ipc
          .mount(place.path)
          .then(arrived)
          .catch((refusal: unknown) => {
            // An internal disk needs an administrator, and the daemon never waits on a person.
            // So it says so with a code, and this asks a second time — which is the moment
            // polkit puts a password dialog in front of somebody who just pressed Enter and is
            // therefore still there to answer it (D61).
            if (codeOf(refusal) !== NEEDS_AUTHORISATION) {
              setDaemonError(messageOf(refusal));
              return;
            }
            setDaemonError(`${place.name} braucht eine Berechtigung — bitte bestätigen …`);
            ipc
              .mount(place.path, true)
              .then(arrived)
              .catch((e: unknown) => setDaemonError(messageOf(e)));
          });
        return;
      }
      browse.goTo(place.path);
    },
    [browse, places],
  );

  /** Unmount the drive under the cursor, and power it off when it can be unplugged. */
  const ejectPlace = useCallback(
    (place: Place | undefined) => {
      if (!place?.volume?.mounted || !place.device) return;
      const removable = place.volume.removable;
      ipc
        .unmount(place.device, removable)
        .then(() => {
          setDaemonError(
            removable ? `${place.name} kann abgezogen werden` : `${place.name} ist ausgehängt`,
          );
          places.reload();
        })
        .catch((e: unknown) => setDaemonError(messageOf(e)));
    },
    [places],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (paletteOpen) setPaletteOpen(false);
        else if (inspectorOpen) setInspectorOpen(false);
        else if (renaming || creating !== null) {
          setRenaming(null);
          setCreating(null);
        } else if (operations.running) operations.cancel();
        else if (selection.state.indices.size > 1) selection.clear();
        else if (query !== "") onQuery("");
        else ipc.dismiss().catch(() => {});
        return;
      }
      if (event.key === "k" && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        setPaletteOpen((open) => !open);
        return;
      }
      // Ctrl+Shift+A: the Agent Inspector. Checked on the lower-case key as well, because a
      // shifted `a` arrives as `A` on some layouts and as `a` on others, and a binding that
      // works on one keyboard is a binding that does not work.
      if (
        (event.key === "A" || event.key === "a") &&
        event.shiftKey &&
        (event.ctrlKey || event.metaKey)
      ) {
        event.preventDefault();
        setInspectorOpen((open) => !open);
        return;
      }
      if (paletteOpen) return;
      // The panel owns the keyboard while it is open, except for the two bindings above. It is
      // a dialog inside the one toplevel, not a mode: arrow keys inside it must not also move
      // the cursor in the result list behind it.
      if (inspectorOpen) return;

      // While the conflict question is open, nothing else may act: the answer decides what
      // happens to files, and an arrow key that moved the selection underneath it would change
      // what the answer applies to.
      if (operations.pending) return;

      // And the same while the application chooser is open, for the same reason: the answer is
      // "open *these* with it", and a selection that moved underneath would open others.
      if (chooser) return;

      // While something is being named, the keyboard belongs to that field. Without this the
      // overlay's Enter opens the selected row and prevents the form from submitting, so the
      // rename silently never happens — which is how two browser tests found it.
      if (renaming || creating !== null) return;

      // The places screen has no selection and no operations — you do not cut a drive — so it
      // takes the movement keys and nothing else, before anything below can act on a list that
      // is not on screen. Escape, Ctrl+K and typing are handled above and still work.
      if (atPlaces) {
        switch (event.key) {
          case "ArrowDown":
            event.preventDefault();
            places.move(1, flatPlaces.length);
            return;
          case "ArrowUp":
            event.preventDefault();
            places.move(-1, flatPlaces.length);
            return;
          case "Home":
          case "PageUp":
            event.preventDefault();
            places.move(-flatPlaces.length, flatPlaces.length);
            return;
          case "End":
          case "PageDown":
            event.preventDefault();
            places.move(flatPlaces.length, flatPlaces.length);
            return;
          case "Enter":
          case "ArrowRight":
            event.preventDefault();
            openPlace(flatPlaces[places.cursor]);
            return;
          case "e":
          case "E":
            // Ctrl+E on a drive: unmount it, and cut the power when it is something that can be
            // unplugged. Only here, because the places screen is the only place a drive is a row.
            if (!event.ctrlKey && !event.metaKey) break;
            event.preventDefault();
            ejectPlace(flatPlaces[places.cursor]);
            return;
          default:
            // Everything else falls through: Ctrl+Z still undoes, Tab still switches panes.
            break;
        }
      }

      // Ctrl+A, and the operations, before the navigation keys: a file manager's bindings are
      // mostly modified keys, and checking them first keeps the switch below about movement.
      if (event.ctrlKey || event.metaKey) {
        switch (event.key.toLowerCase()) {
          case "a":
            // Ctrl+A belongs to the text field while there is text in it. Taking it always
            // means somebody trying to replace their query selects every file in the directory
            // instead — which is also how a browser test caught this, by failing to clear the
            // input it had just typed into.
            if (query !== "") return;
            event.preventDefault();
            selection.selectAll();
            return;
          case "c":
            event.preventDefault();
            operations.copy(targets);
            return;
          case "x":
            event.preventDefault();
            operations.cut(targets);
            return;
          case "v":
            event.preventDefault();
            if (!searching) operations.paste(browse.state.path);
            return;
          case "z":
            event.preventDefault();
            operations.undo();
            return;
          case "n":
            if (event.shiftKey && !searching) {
              event.preventDefault();
              setCreating("");
            }
            return;
          case "d":
            event.preventDefault();
            setComfortable((c) => !c);
            return;
          case "e":
            // Ctrl+E on an archive: unpack it. The places pane takes this key first and ejects
            // a drive with it, which is the same idea addressed to the only kind of row that
            // has one — nothing here can reach a drive, and nothing there can reach a file.
            event.preventDefault();
            extractSelected();
            return;
        }
      }

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          selection.move("down", event.shiftKey);
          break;
        case "ArrowUp":
          event.preventDefault();
          selection.move("up", event.shiftKey);
          break;
        case "PageDown":
          event.preventDefault();
          selection.move("pageDown", event.shiftKey);
          break;
        case "PageUp":
          event.preventDefault();
          selection.move("pageUp", event.shiftKey);
          break;
        case "Home":
          if (event.ctrlKey) {
            event.preventDefault();
            selection.move("home", event.shiftKey);
          }
          break;
        case "End":
          if (event.ctrlKey) {
            event.preventDefault();
            selection.move("end", event.shiftKey);
          }
          break;
        case "Enter":
          event.preventDefault();
          // Ctrl is "with something else". Plain Enter stays the fast path: one key, the
          // desktop's own choice, no dialog in front of the thing somebody asked for.
          if (event.ctrlKey || event.metaKey) openWithSelected();
          else openSelected();
          break;
        case "F2":
          event.preventDefault();
          if (current) setRenaming(current);
          break;
        case "Delete":
          event.preventDefault();
          operations.trash(targets);
          break;
        case "Tab":
          event.preventDefault();
          setPane((p) => (p === "preview" ? "context" : "preview"));
          break;
        case "Backspace":
          // Only when there is nothing to delete, so it never eats a character somebody meant
          // to remove. With an empty input it is the obvious "up one level".
          if (query === "" && !searching) {
            event.preventDefault();
            browse.goUp();
          }
          break;
        case "ArrowLeft":
          if (event.altKey) {
            event.preventDefault();
            browse.goBack();
          }
          break;
        case "ArrowRight":
          if (event.altKey && current?.isDir) {
            event.preventDefault();
            activate(current);
          }
          break;
      }
    },
    [
      paletteOpen,
      query,
      onQuery,
      openSelected,
      current,
      activate,
      browse,
      searching,
      selection,
      operations,
      targets,
      renaming,
      creating,
      atPlaces,
      places,
      flatPlaces,
      openPlace,
      extractSelected,
    ],
  );

  return (
    <div
      className="hc-overlay"
      data-density={comfortable ? "comfortable" : "compact"}
      data-width={showBoth ? "wide" : showPreview ? "medium" : "narrow"}
      // A border rather than an overlay: something is being dragged over the window and can be
      // let go of. An overlay would cover the breadcrumb, which is the one thing a person needs
      // to read at that moment — where it would land.
      data-dropping={operations.dragging ? "yes" : undefined}
      // Nothing moves while nobody is looking. A hidden GTK window keeps its page running, so
      // an infinite animation is an infinite animation — the CSS below stops them all rather
      // than trusting each one to know.
      data-visible={visible ? "yes" : "no"}
      onKeyDown={onKeyDown}
    >
      <Ambient load={load} />

      <header className="hc-head">
        <input
          ref={input}
          className="hc-input hc-title"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder="Wonach suchst du?"
          spellCheck={false}
          autoComplete="off"
          aria-label="Suche"
        />
        <span className="hc-meta hc-status">
          {state.error || browse.state.error ? (
            <span className="hc-error">{state.error ?? browse.state.error}</span>
          ) : searching ? (
            <>
              <span className="hc-num">{rows.length}</span>
              <span className="hc-num">{state.elapsedMs} ms</span>
              <span>{state.mode}</span>
              {state.refining && <span className="hc-faint">…</span>}
            </>
          ) : (
            <>
              <span className="hc-num">
                {selection.state.indices.size > 1
                  ? `${selection.state.indices.size}/${rows.length}`
                  : rows.length}
              </span>
              <span>{browse.state.sort}</span>
              {operations.clipboard && (
                <span className="hc-pill" title="Ctrl+V fügt hier ein">
                  {operations.clipboard.mode === "move" ? "ausgeschnitten" : "kopiert"}{" "}
                  {operations.clipboard.paths.length}
                </span>
              )}
              {browse.state.hidden > 0 && (
                <span className="hc-faint" title="Von der Deny-Liste verdeckt">
                  +{browse.state.hidden}
                </span>
              )}
            </>
          )}
          <button
            type="button"
            className={inspectorOpen ? "hc-pill hc-pill-on" : "hc-pill"}
            title="Agenten-Inspektor (Ctrl+Shift+A)"
            aria-label="Agenten-Inspektor"
            aria-pressed={inspectorOpen}
            onMouseDown={(event) => {
              // The search field must keep the caret: a person clicks this, looks, closes it,
              // and goes on typing. Taking focus would cost them the next keystroke.
              event.preventDefault();
              setInspectorOpen((open) => !open);
            }}
          >
            Agenten
          </button>
        </span>
      </header>

      {(renaming || creating !== null) && (
        <form
          className="hc-naming"
          onSubmit={(event) => {
            event.preventDefault();
            const value = new FormData(event.currentTarget).get("name");
            const name = typeof value === "string" ? value.trim() : "";
            if (name !== "") {
              if (renaming) operations.rename(renaming.path, name);
              else operations.mkdir(joinPath(browse.state.path, name));
            }
            setRenaming(null);
            setCreating(null);
            input.current?.focus();
          }}
        >
          <label className="hc-meta" htmlFor="hc-name">
            {renaming ? "Umbenennen" : "Neuer Ordner"}
          </label>
          <input
            id="hc-name"
            name="name"
            className="hc-name-input"
            defaultValue={renaming?.name ?? ""}
            autoFocus
            spellCheck={false}
            autoComplete="off"
            onKeyDown={(event) => {
              // Escape belongs to the naming field while it is open, and must not reach the
              // overlay's handler and close the window.
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                setRenaming(null);
                setCreating(null);
                input.current?.focus();
              }
            }}
          />
        </form>
      )}

      {operations.running && (
        <div className="hc-progress" role="status">
          <div
            className="hc-progress-bar"
            style={{ transform: `scaleX(${operations.running.fraction})` }}
          />
          <span className="hc-meta hc-progress-text">
            <span className="hc-num">
              {operations.running.filesDone}/{operations.running.totalFiles}
            </span>
            <span className="hc-num">{Math.round(operations.running.fraction * 100)} %</span>
            <span className="hc-progress-current">
              {operations.running.current
                ? splitPathName(operations.running.current)
                : "…"}
            </span>
            <button type="button" className="hc-pill" onMouseDown={() => operations.cancel()}>
              Abbrechen (Esc)
            </button>
          </span>
        </div>
      )}

      {(daemonError || operations.notice) && (
        <p
          className={operations.notice?.kind === "error" ? "hc-banner hc-error" : "hc-banner"}
          role="status"
        >
          {operations.notice?.text ?? daemonError}
        </p>
      )}

      <main className="hc-body">
        <div className="hc-column">
          {!searching && !atPlaces && (
            <Breadcrumb
              path={browse.state.path}
              onNavigate={(next) => browse.goTo(next)}
              onPlaces={() => browse.goTo(PLACES)}
              hue={null}
            />
          )}
          {atPlaces ? (
            <PlacesView
              sections={places.sections}
              cursor={places.cursor}
              onSelect={places.select}
              onOpen={openPlace}
              loading={places.loading}
              error={places.error}
            />
          ) : (
          <ResultList
            rows={rows}
            cursor={selected}
            selected={selection.state.indices}
            onSelect={(index, modifiers) => {
              if (modifiers.shift) selection.extendTo(index);
              else if (modifiers.ctrl) selection.toggle(index);
              else selection.select(index);
            }}
            onActivate={activate}
            rowHeight={rowHeight}
            visible={visible}
            zoneHues={zoneHues}
            emptyMessage={emptyMessage(searching, state.loading, browse.state)}
            selectedPaths={targets}
          />
          )}
        </div>

        {showPreview && (showBoth || pane === "preview") && <PreviewPane hit={settled} />}
        {showContext && <ContextSidebar hit={settled} row={settledFocus.row} />}
      </main>

      {operations.pending && (
        <ConflictDialog
          conflicts={operations.pending.conflicts}
          totalFiles={operations.pending.totalFiles}
          onChoose={operations.resolveConflict}
          onCancel={() => {
            operations.dismissConflict();
            input.current?.focus();
          }}
        />
      )}

      {chooser && (
        <OpenWith
          label={chooser.label}
          count={chooser.paths.length}
          mime={choices?.mime ?? "…"}
          applications={choices?.applications ?? NO_APPLICATIONS}
          showingAll={choicesAll}
          loading={choicesLoading}
          onOpen={openWithChoice}
          onSetDefault={(application) => {
            // Recorded first, so that a failure to write the choice is a failure to make it —
            // opening the file anyway would look like it had been remembered.
            const mime = choices?.mime;
            if (!mime) return;
            ipc
              .appsSetDefault(mime, application.id)
              .catch(() => {})
              .then(() => openWithChoice(application));
          }}
          onShowAll={() => chooser && askWhichApplication(chooser.paths, chooser.label, true)}
          onCancel={closeChooser}
        />
      )}

      <AgentInspector
        open={inspectorOpen}
        visible={visible}
        tab={inspectorTab}
        onTab={setInspectorTab}
        onClose={() => {
          setInspectorOpen(false);
          input.current?.focus();
        }}
      />

      <CommandPalette
        open={paletteOpen}
        commands={commands}
        onClose={() => {
          setPaletteOpen(false);
          input.current?.focus();
        }}
      />
    </div>
  );
}
