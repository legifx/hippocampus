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
import { CommandPalette, type Command } from "./components/CommandPalette";
import { ContextSidebar } from "./components/ContextSidebar";
import { PreviewPane } from "./components/PreviewPane";
import { ResultList } from "./components/ResultList";
import { useBrowse } from "./hooks/useBrowse";
import { useOperations } from "./hooks/useOperations";
import { useSelection } from "./hooks/useSelection";
import { throttled, useDeferred } from "./hooks/useDeferred";
import { useSearch } from "./hooks/useSearch";
import { useViewport } from "./hooks/useViewport";
import { ipc, type SearchHit, type SortBy } from "./ipc";
import { rowFromEntry, rowFromHit, type Row } from "./row";

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
const START_PATH = "/";

export function App() {
  const { state, run } = useSearch();
  const browse = useBrowse(START_PATH);
  const [query, setQuery] = useState("");
  const [pane, setPane] = useState<Pane>("preview");
  const [paletteOpen, setPaletteOpen] = useState(false);
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

  const operations = useOperations(browse.reload);

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
  const settled = useDeferred(hit);

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

    // The breath layer's period is the indexing load (design system §6). Polling it once every
    // few seconds is enough for a heartbeat and costs nothing.
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
    const timer = setInterval(poll, 4000);

    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, []);

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

  const commands: Command[] = useMemo(
    () => [
      { id: "open", label: "Öffnen", binding: "Enter", run: openSelected },
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
                `Kaltstart ${report.interactive_ms} ms (Budget ${report.budget_ms} ms) — ` +
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
    [openSelected, hit, pane, showBoth, comfortable, searching, browse, operations, targets, current],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (paletteOpen) setPaletteOpen(false);
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
      if (paletteOpen) return;

      // While something is being named, the keyboard belongs to that field. Without this the
      // overlay's Enter opens the selected row and prevents the form from submitting, so the
      // rename silently never happens — which is how two browser tests found it.
      if (renaming || creating !== null) return;

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
          openSelected();
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
    ],
  );

  return (
    <div
      className="hc-overlay"
      data-density={comfortable ? "comfortable" : "compact"}
      data-width={showBoth ? "wide" : showPreview ? "medium" : "narrow"}
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
          {!searching && (
            <Breadcrumb
              path={browse.state.path}
              onNavigate={(next) => browse.goTo(next)}
              hue={null}
            />
          )}
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
            zoneHues={zoneHues}
            emptyMessage={emptyMessage(searching, state.loading, browse.state)}
          />
        </div>

        {showPreview && (showBoth || pane === "preview") && <PreviewPane hit={settled} />}
        {showContext && <ContextSidebar hit={settled} />}
      </main>

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
