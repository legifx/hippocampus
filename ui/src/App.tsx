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
import { CommandPalette, type Command } from "./components/CommandPalette";
import { ContextSidebar } from "./components/ContextSidebar";
import { PreviewPane } from "./components/PreviewPane";
import { ResultList } from "./components/ResultList";
import { throttled, useDeferred } from "./hooks/useDeferred";
import { useSearch } from "./hooks/useSearch";
import { useViewport } from "./hooks/useViewport";
import { ipc, type SearchHit } from "./ipc";

const ROW_HEIGHT_COMPACT = 28;
const ROW_HEIGHT_COMFORTABLE = 34;

/** Which side pane is showing when there is room for only one. */
type Pane = "preview" | "context";

/** The daemon needs to know somebody is here, not how fast they type. */
const pingSometimes = throttled(5000);

export function App() {
  const { state, run } = useSearch();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [pane, setPane] = useState<Pane>("preview");
  const [paletteOpen, setPaletteOpen] = useState(false);
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
  const hit: SearchHit | null = state.hits[selected] ?? null;

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
        setSelected(0);
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
      setSelected(0);
      run(next);
      // A person typing here is the clearest evidence a human is present, which is what the
      // daemon's duty cycle keys off (D22) — but the first keystroke establishes that, and the
      // next forty repeat it at the cost of forty round trips.
      pingSometimes(() => void ipc.activityPing().catch(() => {}));
    },
    [run],
  );

  const openSelected = useCallback(() => {
    if (!hit) return;
    ipc.openPath(hit.path).catch(() => {});
  }, [hit]);

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
      { id: "dismiss", label: "Schließen", binding: "Esc", run: () => ipc.dismiss().catch(() => {}) },
    ],
    [openSelected, hit, pane, showBoth, comfortable],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        if (paletteOpen) setPaletteOpen(false);
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

      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          setSelected((i) => Math.min(i + 1, Math.max(state.hits.length - 1, 0)));
          break;
        case "ArrowUp":
          event.preventDefault();
          setSelected((i) => Math.max(i - 1, 0));
          break;
        case "PageDown":
          event.preventDefault();
          setSelected((i) => Math.min(i + 10, Math.max(state.hits.length - 1, 0)));
          break;
        case "PageUp":
          event.preventDefault();
          setSelected((i) => Math.max(i - 10, 0));
          break;
        case "Home":
          if (event.ctrlKey) {
            event.preventDefault();
            setSelected(0);
          }
          break;
        case "End":
          if (event.ctrlKey) {
            event.preventDefault();
            setSelected(Math.max(state.hits.length - 1, 0));
          }
          break;
        case "Enter":
          event.preventDefault();
          openSelected();
          break;
        case "Tab":
          event.preventDefault();
          setPane((p) => (p === "preview" ? "context" : "preview"));
          break;
        case "c":
          if (event.ctrlKey && hit) {
            event.preventDefault();
            navigator.clipboard?.writeText(hit.path).catch(() => {});
          }
          break;
        case "d":
          if (event.ctrlKey) {
            event.preventDefault();
            setComfortable((c) => !c);
          }
          break;
      }
    },
    [paletteOpen, query, onQuery, state.hits.length, openSelected, hit],
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
          {state.error ? (
            <span className="hc-error">{state.error}</span>
          ) : (
            <>
              <span className="hc-num">{state.hits.length}</span>
              <span className="hc-num">{state.elapsedMs} ms</span>
              <span>{state.mode}</span>
              {state.refining && <span className="hc-faint">…</span>}
            </>
          )}
        </span>
      </header>

      {daemonError && (
        <p className="hc-banner" role="status">
          {daemonError}
        </p>
      )}

      <main className="hc-body">
        <ResultList
          hits={state.hits}
          selected={selected}
          onSelect={setSelected}
          onOpen={(h) => ipc.openPath(h.path).catch(() => {})}
          rowHeight={rowHeight}
          zoneHues={zoneHues}
          emptyMessage={
            state.loading
              ? "…"
              : query.trim() === ""
                ? "Tippen, um zu suchen"
                : "Nichts gefunden"
          }
        />

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
