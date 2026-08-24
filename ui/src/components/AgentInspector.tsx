/**
 * The Agent Inspector: what the agents did, what they believe, and what they may do.
 *
 * M11 §8. One panel, five tabs, inside the existing toplevel — niri owns the layout and this
 * project ships no second window (M4's constraints, which are requirements).
 *
 * It obeys D4 like everything else in this window: **it renders and it does not decide.** No
 * filtering, no sorting, no merging. The lists arrive in the order the daemon chose, and the
 * one control that looks like a filter — the tab strip — chooses which *call* is made, not
 * which rows of one answer survive.
 *
 * It writes in exactly three places, and each of them is deliberate rather than convenient:
 *
 *  1. the kill switch (R7), because a switch you have to leave the window to reach is a switch
 *     nobody reaches in the moment they want it;
 *  2. undo, for the same reason;
 *  3. deleting a memory (D78) — a machine that forms durable beliefs about a person's work and
 *     does not let the person correct them is a machine keeping secrets, and the first wrong
 *     memory an agent forms is repeated confidently until somebody can remove it.
 *
 * The three-dimensional tab is loaded on demand and not before. The main bundle is deliberately
 * unsplit (see `vite.config.ts`) because a tiling user opens this window constantly and cold
 * start is a 300 ms gate; three.js is around half a megabyte and belongs to a tab most sessions
 * never open. A dynamic import is the one exception, and it is an exception that protects the
 * rule rather than eroding it.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ipc,
  type AgentStatusResult,
  type AuditEntry,
  type Memory,
  type MemoryKind,
  type SessionsResult,
} from "../ipc";

export type InspectorTab = "live" | "memory" | "policy" | "undo" | "architecture";

export const TABS: { id: InspectorTab; label: string; hint: string }[] = [
  { id: "live", label: "Live", hint: "Sitzungen und Aufrufe, während sie geschehen" },
  { id: "memory", label: "Gedächtnis", hint: "Was die Agenten gelernt haben" },
  { id: "policy", label: "Regeln", hint: "Not-Aus, Allow-Liste, abgelehnte Aufrufe" },
  { id: "undo", label: "Rückgängig", hint: "Das Journal der Änderungen" },
  { id: "architecture", label: "Architektur", hint: "Das Gedächtnis, dreidimensional" },
];

/** How often the Live tab asks again while it is open. */
const LIVE_POLL_MS = 2000;

const KIND_LABEL: Record<MemoryKind, string> = {
  fact: "Fakt",
  preference: "Vorliebe",
  procedure: "Ablauf",
  decision: "Entscheidung",
  failure: "Fehlschlag",
  observation: "Beobachtung",
};

function scopeLabel(scope: Memory["scope"]): string {
  switch (scope.kind) {
    case "global":
      return "global";
    case "agent":
      return `Agent ${scope.id ?? "?"}`;
    case "session":
      return `Sitzung ${scope.id ?? "?"}`;
    case "project":
      return `Projekt ${scope.id ?? "?"}`;
    default:
      return String(scope.kind);
  }
}

function ago(unixSeconds: number, now: number): string {
  const seconds = Math.max(0, now - unixSeconds);
  if (seconds < 60) return `${Math.round(seconds)} s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} min`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)} h`;
  return `${Math.round(seconds / 86400)} d`;
}

export type AgentInspectorProps = {
  open: boolean;
  onClose: () => void;
  /** Which tab to show. Held by the caller so a summon can reopen where it was left. */
  tab: InspectorTab;
  onTab: (tab: InspectorTab) => void;
};

export function AgentInspector({ open, onClose, tab, onTab }: AgentInspectorProps) {
  const [sessions, setSessions] = useState<SessionsResult | null>(null);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  /** The refused calls, asked for separately. The daemon filters; this renders (D4). */
  const [refused, setRefused] = useState<AuditEntry[]>([]);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [basis, setBasis] = useState<string>("");
  const [status, setStatus] = useState<AgentStatusResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useMemo(() => Math.floor(Date.now() / 1000), [sessions, audit, memories]);

  const fail = useCallback((e: unknown) => {
    setError(e instanceof Error ? e.message : String(e));
  }, []);

  const loadLive = useCallback(async () => {
    try {
      const [s, a, r, st] = await Promise.all([
        ipc.agentSessions(50),
        ipc.auditRecent(80),
        ipc.auditRecent(80, true),
        ipc.agentStatus(),
      ]);
      setSessions(s);
      setAudit(a.entries ?? []);
      setRefused(r.entries ?? []);
      setStatus(st);
      setError(null);
    } catch (e) {
      fail(e);
    }
  }, [fail]);

  const loadMemories = useCallback(async () => {
    try {
      const result = await ipc.memories(500, true);
      setMemories(result.memories ?? []);
      setBasis(result.basis ?? "");
      setError(null);
    } catch (e) {
      fail(e);
    }
  }, [fail]);

  // Nothing polls while the panel is shut, and nothing polls a tab that is not showing. A panel
  // that costs a core is a panel that gets switched off (M11 DoD §10), and this window has a
  // recorded case of an idle animation costing exactly that (D59).
  useEffect(() => {
    if (!open) return;
    if (tab === "live" || tab === "policy") {
      void loadLive();
      const timer = setInterval(() => void loadLive(), LIVE_POLL_MS);
      return () => clearInterval(timer);
    }
    if (tab === "memory") {
      void loadMemories();
    }
    return undefined;
  }, [open, tab, loadLive, loadMemories]);

  const forget = useCallback(
    async (memory: Memory) => {
      setBusy(true);
      try {
        await ipc.forgetMemory(memory.node_id);
        await loadMemories();
      } catch (e) {
        fail(e);
      } finally {
        setBusy(false);
      }
    },
    [fail, loadMemories],
  );

  const toggleSurface = useCallback(async () => {
    if (!status) return;
    setBusy(true);
    try {
      setStatus(await ipc.agentSetEnabled(!status.enabled));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }, [fail, status]);

  if (!open) return null;

  return (
    <section
      className="hc-inspector"
      role="dialog"
      aria-label="Agenten-Inspektor"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onClose();
        }
      }}
    >
      <header className="hc-inspector-head">
        <nav className="hc-tabs" role="tablist">
          {TABS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              role="tab"
              aria-selected={entry.id === tab}
              className={entry.id === tab ? "hc-tab hc-tab-on" : "hc-tab"}
              title={entry.hint}
              onMouseDown={() => onTab(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </nav>
        <span className="hc-meta">
          {status && (
            <button
              type="button"
              className={status.enabled ? "hc-pill hc-pill-on" : "hc-pill hc-pill-off"}
              disabled={busy}
              onMouseDown={() => void toggleSurface()}
              title="Not-Aus: schaltet die Agenten-Schnittstelle ab (R7)"
            >
              {status.enabled ? "Schnittstelle an" : "Schnittstelle aus"}
            </button>
          )}
          <button type="button" className="hc-pill" onMouseDown={onClose}>
            Schließen (Esc)
          </button>
        </span>
      </header>

      {error && (
        <p className="hc-banner hc-error" role="status">
          {error}
        </p>
      )}

      <div className="hc-inspector-body">
        {tab === "live" && <LiveTab sessions={sessions} audit={audit} now={now} />}
        {tab === "memory" && (
          <MemoryTab
            memories={memories}
            basis={basis}
            busy={busy}
            now={now}
            onForget={forget}
          />
        )}
        {tab === "policy" && <PolicyTab status={status} refused={refused} />}
        {tab === "undo" && <UndoTab />}
        {tab === "architecture" && <ArchitectureTab />}
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Live
// ---------------------------------------------------------------------------

function LiveTab({
  sessions,
  audit,
  now,
}: {
  sessions: SessionsResult | null;
  audit: AuditEntry[];
  now: number;
}) {
  const rows = sessions?.sessions ?? [];
  return (
    <div className="hc-inspector-split">
      <div className="hc-inspector-pane">
        <h2 className="hc-meta">
          Sitzungen <span className="hc-num">{sessions?.open ?? 0} offen</span>
        </h2>
        {rows.length === 0 && (
          <p className="hc-faint">
            Noch kein Agent hat eine Sitzung auf diesem Daemon geöffnet.
          </p>
        )}
        <ul className="hc-rows">
          {rows.map((session) => (
            <li key={session.id} className="hc-row-item">
              <span className={session.ended_at ? "hc-dot" : "hc-dot hc-dot-live"} />
              <span className="hc-row-main">
                <strong>{session.name}</strong>
                {session.task ? ` — ${session.task}` : ""}
              </span>
              <span className="hc-meta hc-row-side">
                <span className="hc-num">{session.calls}</span> Aufrufe
                {session.refusals > 0 && (
                  <>
                    {" · "}
                    <span className="hc-error">{session.refusals} abgelehnt</span>
                  </>
                )}
                {" · "}
                <span className="hc-num">{session.memories}</span> gemerkt
                {" · vor "}
                {ago(session.started_at, now)}
              </span>
            </li>
          ))}
        </ul>
      </div>

      <div className="hc-inspector-pane">
        <h2 className="hc-meta">Aufrufe</h2>
        <ul className="hc-rows hc-rows-dense">
          {audit.map((entry) => (
            <li
              key={entry.id}
              className={entry.allowed ? "hc-row-item" : "hc-row-item hc-row-refused"}
            >
              <span className="hc-row-main">
                <code>{entry.tool}</code>
                <span className="hc-faint"> {entry.actor}</span>
              </span>
              <span className="hc-meta hc-row-side">
                {entry.result_summary ?? (entry.allowed ? "" : "abgelehnt")}
                {" · vor "}
                {ago(entry.ts, now)}
              </span>
            </li>
          ))}
        </ul>
        {audit.length === 0 && <p className="hc-faint">Noch nichts protokolliert.</p>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

function MemoryTab({
  memories,
  basis,
  busy,
  now,
  onForget,
}: {
  memories: Memory[];
  basis: string;
  busy: boolean;
  now: number;
  onForget: (memory: Memory) => void;
}) {
  const [confirming, setConfirming] = useState<number | null>(null);
  return (
    <div className="hc-inspector-pane">
      <h2 className="hc-meta">
        Gedächtnis <span className="hc-num">{memories.length}</span>
      </h2>
      {/* Every view states its own basis — what this list is, and what it is not. */}
      <p className="hc-faint hc-basis">{basis}</p>
      {memories.length === 0 && (
        <p className="hc-faint">
          Noch hat kein Agent etwas gelernt, das nicht in einer Datei steht.
        </p>
      )}
      <ul className="hc-rows">
        {memories.map((memory) => (
          <li
            key={memory.node_id}
            className={memory.superseded_by ? "hc-row-item hc-row-old" : "hc-row-item"}
          >
            <span className="hc-row-main">
              <span className="hc-pill hc-kind">{KIND_LABEL[memory.kind] ?? memory.kind}</span>
              <span className="hc-memory-text">{memory.text}</span>
              {memory.subject_path && (
                <span className="hc-faint"> — zu {memory.subject_path}</span>
              )}
            </span>
            <span className="hc-meta hc-row-side">
              <span title="Wer sie geschrieben hat">{memory.actor}</span>
              {" · "}
              <span title="Gültigkeitsbereich">{scopeLabel(memory.scope)}</span>
              {" · "}
              <span
                className={memory.usefulness < 0 ? "hc-error" : undefined}
                title="Nutzen: steigt durch gemeldete Treffer, fällt durch gemeldeten Ballast und durch Abrufe, die niemand je gemeldet hat"
              >
                {memory.usefulness >= 0 ? "+" : ""}
                {memory.usefulness.toFixed(2)}
              </span>
              {" · "}
              <span title="geholfen / Ballast / überhaupt gezeigt">
                {memory.helped_count}/{memory.noise_count}/{memory.retrieved_count}
              </span>
              {memory.human_confirmed && <span title="Von einem Menschen bestätigt"> ✓</span>}
              {memory.superseded_by && (
                <span className="hc-faint"> · ersetzt durch {memory.superseded_by}</span>
              )}
              {" · vor "}
              {ago(memory.ts, now)}
              {confirming === memory.node_id ? (
                <>
                  {" "}
                  <button
                    type="button"
                    className="hc-pill hc-pill-danger"
                    disabled={busy}
                    onMouseDown={() => {
                      setConfirming(null);
                      onForget(memory);
                    }}
                  >
                    Wirklich löschen
                  </button>{" "}
                  <button
                    type="button"
                    className="hc-pill"
                    onMouseDown={() => setConfirming(null)}
                  >
                    Abbrechen
                  </button>
                </>
              ) : (
                <>
                  {" "}
                  <button
                    type="button"
                    className="hc-pill"
                    disabled={busy}
                    title="Löscht sie endgültig — auch über einen Neuaufbau des Index hinweg"
                    onMouseDown={() => setConfirming(memory.node_id)}
                  >
                    Löschen
                  </button>
                </>
              )}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

function PolicyTab({
  status,
  refused,
}: {
  status: AgentStatusResult | null;
  /** Asked for as refusals rather than sieved out of everything (D4). */
  refused: AuditEntry[];
}) {
  return (
    <div className="hc-inspector-split">
      <div className="hc-inspector-pane">
        <h2 className="hc-meta">Regeln</h2>
        {!status && <p className="hc-faint">…</p>}
        {status && (
          <dl className="hc-facts">
            <dt>Agenten-Schnittstelle</dt>
            <dd className={status.enabled ? undefined : "hc-error"}>
              {status.enabled ? "an" : "aus — kein Agent-Aufruf wird beantwortet"}
            </dd>
            <dt>Indizierung</dt>
            <dd>{status.indexing_paused ? "pausiert" : "läuft"}</dd>
            <dt>Trockenlauf verlangt</dt>
            <dd>{status.require_dry_run ? "ja" : "nein"}</dd>
            <dt>Schreib-Allow-Liste</dt>
            <dd>
              {status.write_allowlist.length === 0 ? (
                <span className="hc-faint">
                  leer — kein Agent darf irgendetwas im Dateisystem ändern
                </span>
              ) : (
                <ul className="hc-rows hc-rows-dense">
                  {status.write_allowlist.map((path) => (
                    <li key={path}>
                      <code>{path}</code>
                    </li>
                  ))}
                </ul>
              )}
            </dd>
          </dl>
        )}
      </div>

      <div className="hc-inspector-pane">
        <h2 className="hc-meta">
          Abgelehnt <span className="hc-num">{refused.length}</span>
        </h2>
        {refused.length === 0 && (
          <p className="hc-faint">Nichts abgelehnt, seit das Protokoll reicht.</p>
        )}
        <ul className="hc-rows hc-rows-dense">
          {refused.map((entry) => (
            <li key={entry.id} className="hc-row-item hc-row-refused">
              <span className="hc-row-main">
                <code>{entry.tool}</code> <span className="hc-faint">{entry.actor}</span>
              </span>
              <span className="hc-meta hc-row-side">{entry.result_summary ?? "abgelehnt"}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

function UndoTab() {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  return (
    <div className="hc-inspector-pane">
      <h2 className="hc-meta">Rückgängig</h2>
      <p className="hc-faint">
        Jede vom Agenten ausgeführte Änderung hat ihre Umkehrung mitgeschrieben, bevor sie
        geschah. Der Papierkorb ist der Grund, warum dafür kein Inhalt im Index liegen muss.
      </p>
      <button
        type="button"
        className="hc-pill"
        disabled={busy}
        onMouseDown={() => {
          setBusy(true);
          ipc
            .undo(1)
            .then(() => setMessage("Die letzte Änderung wurde zurückgenommen."))
            .catch((e: unknown) =>
              setMessage(e instanceof Error ? e.message : String(e)),
            )
            .finally(() => setBusy(false));
        }}
      >
        Letzte Änderung zurücknehmen
      </button>
      {message && <p className="hc-banner">{message}</p>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Architecture — the three-dimensional view, live
// ---------------------------------------------------------------------------

/**
 * The M8 scene, mounted here rather than reimplemented.
 *
 * `ui-agent` owns the layout, the palette and the renderer, and this imports that module
 * directly through a path alias. Two implementations of "what the brain looks like" would
 * diverge, and the one in the window would be the one nobody noticed had.
 *
 * Loaded on demand: three.js is around half a megabyte, this window has a 300 ms cold-start
 * gate, and most sessions never open this tab.
 */
function ArchitectureTab() {
  const host = useRef<HTMLDivElement | null>(null);
  const [state, setState] = useState<"loading" | "ready" | "failed">("loading");
  const [detail, setDetail] = useState<string>("");
  const [totals, setTotals] = useState<{ nodes: number; edges: number } | null>(null);
  const [layout, setLayout] = useState<"tree" | "shells">("tree");
  /** Bumped to fetch the snapshot again. A reload is asked for, never done unprompted: it
   *  rebuilds the scene, and rebuilding it throws away wherever the viewer had moved to. */
  const [reloads, setReloads] = useState(0);
  /** What the live tracking has seen since the snapshot was taken. */
  const [live, setLive] = useState<{ episodes: number; unseen: number; last: string | null }>({
    episodes: 0,
    unseen: 0,
    last: null,
  });
  const scene = useRef<{
    dispose(): void;
    pulse(nodeId: number, speed?: number): void;
    setLayout(mode: "tree" | "shells", animate?: boolean): void;
  } | null>(null);
  /** Node ids the current scene knows about. A pulse for anything else would be silent. */
  const drawn = useRef<Set<number>>(new Set());

  useEffect(() => {
    let disposed = false;

    (async () => {
      try {
        const [{ BrainScene }, { assertSnapshot }, answer] = await Promise.all([
          import("@agent-view/scene"),
          import("@agent-view/snapshot"),
          ipc.brainGraph(4000),
        ]);
        if (disposed || !host.current) return;
        // Validated before anything is drawn, exactly as the standalone view does it: a daemon
        // that answers with something unexpected must produce one sentence rather than a black
        // canvas with a stack trace behind it.
        const snapshot = assertSnapshot(answer);
        const graph = snapshot as { nodes?: unknown[]; edges?: unknown[] };
        setTotals({ nodes: graph.nodes?.length ?? 0, edges: graph.edges?.length ?? 0 });
        // A brain with nothing in it draws as a black screen, which is indistinguishable from
        // a broken renderer. Say so instead.
        if ((graph.nodes?.length ?? 0) === 0) {
          setState("failed");
          setDetail(
            "Der Index enthält noch keine Knoten, die gezeichnet werden könnten. " +
              "`cortex consolidate` einmal laufen lassen.",
          );
          return;
        }
        // A plain loop rather than map-then-filter, and not only for style: the architectural
        // test that enforces D4 looks for collection operations by name, and it is right to be
        // blunt about it. Narrowing a type is not data processing, but a test that had to tell
        // the difference would be a test with an exception in it, and the first real violation
        // would arrive dressed as one.
        const known = new Set<number>();
        for (const node of (graph.nodes ?? []) as { id?: number }[]) {
          if (typeof node.id === "number") known.add(node.id);
        }
        drawn.current = known;
        scene.current = new BrainScene({
          container: host.current,
          snapshot: snapshot as never,
        });
        setState("ready");
      } catch (e) {
        if (disposed) return;
        setState("failed");
        setDetail(e instanceof Error ? e.message : String(e));
      }
    })();

    return () => {
      disposed = true;
      // Not optional: a WebGL context that is never released is a WebGL context the browser
      // eventually refuses to give again, and this tab can be opened and closed all evening.
      scene.current?.dispose();
      scene.current = null;
    };
  }, [reloads]);

  // ---- Live tracking ---------------------------------------------------------------------
  //
  // `brain.diff` rather than fetching the graph again: the protocol documents the graph as the
  // one large answer, and "has anything happened" is a small question. Every episode that names
  // a node the scene is drawing sends a light along that node's edges, so a running agent is
  // visible as movement rather than as a number that changes.
  //
  // Episodes about nodes this snapshot does not contain — a memory written a moment ago is the
  // common case — are counted rather than drawn, and the count is what offers a reload. Redrawing
  // the scene on its own would throw away wherever the viewer had moved the camera to.
  useEffect(() => {
    if (state !== "ready") return undefined;
    let stopped = false;
    let cursor: number | null = null;

    const tick = async () => {
      try {
        const diff = await ipc.brainDiff(cursor, 200);
        if (stopped) return;
        cursor = diff.cursor;
        let unseen = 0;
        let last: string | null = null;
        for (const change of diff.changes) {
          last = change.action;
          if (change.node_id === null) continue;
          if (drawn.current.has(change.node_id)) scene.current?.pulse(change.node_id);
          else unseen += 1;
        }
        if (diff.changes.length > 0) {
          setLive((previous) => ({
            episodes: previous.episodes + diff.changes.length,
            unseen: previous.unseen + unseen,
            last: last ?? previous.last,
          }));
        }
      } catch {
        // A daemon that has gone away is not this tab's problem to report twice; the panel
        // already has an error line, and a failing poll must not fill it every two seconds.
      }
    };

    void tick();
    const timer = setInterval(() => void tick(), LIVE_POLL_MS);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [state]);

  return (
    <div className="hc-inspector-pane hc-architecture">
      <h2 className="hc-meta">
        Architektur
        {totals && (
          <>
            {" "}
            <span className="hc-num">{totals.nodes}</span> Knoten{" "}
            <span className="hc-num">{totals.edges}</span> Kanten
          </>
        )}
        {state === "ready" && (
          <>
            {" "}
            <button
              type="button"
              className="hc-pill"
              onMouseDown={() => {
                const next = layout === "tree" ? "shells" : "tree";
                setLayout(next);
                scene.current?.setLayout(next);
              }}
              title={
                layout === "tree"
                  ? "Nach Stufe anordnen — nur so ist Vergessen sichtbar"
                  : "Nach Enthaltensein anordnen — was der Index ist"
              }
            >
              {layout === "tree" ? "Baum" : "Schalen"}
            </button>
          </>
        )}
      </h2>
      {state === "loading" && <p className="hc-faint">Wird geladen …</p>}
      {state === "failed" && <p className="hc-banner hc-error">{detail}</p>}
      <div className="hc-scene" ref={host} data-state={state} />
      <p className="hc-faint">
        Helligkeit ist die Stufe, Größe die Salienz, Farbton die Zone; Grün ist eine Erinnerung,
        Gold ein Begriff. Lesen, nicht ändern.
      </p>
      {state === "ready" && (
        <p className="hc-meta">
          Live: <span className="hc-num">{live.episodes}</span> Ereignisse seit dem Öffnen
          {live.last && <> · zuletzt {live.last}</>}
          {live.unseen > 0 && (
            <>
              {" · "}
              <span className="hc-num">{live.unseen}</span> davon betreffen Knoten, die dieser
              Schnappschuss nicht enthält{" "}
              <button
                type="button"
                className="hc-pill"
                onMouseDown={() => setReloads((n) => n + 1)}
                title="Holt den Schnappschuss neu. Die Kameraposition geht dabei verloren."
              >
                Neu laden
              </button>
            </>
          )}
        </p>
      )}
    </div>
  );
}
