/**
 * Gehirn — the brain as the agents use it, live.
 *
 * The window's second destination beside the files (the rail's third entry). What it shows, and
 * where each number comes from:
 *
 * - **Status line** — `brain.services`: whether the curator's decision model is loaded or
 *   asleep, when the store was last backed up, whether federation to the Main Brain is syncing.
 *   Asked every thirty seconds while this view is showing, never while it is not.
 * - **The brain** — `brain.graph` once, drawn by `ui-agent/src/brain3d.ts`, which renders on
 *   demand only: a spark per call, a glow per hot region, nothing while nothing happens.
 * - **Heatmap, live log, agents** — `brain.activity`, asked every 1.5 s with the last sequence
 *   number while the view is showing. The daemon keeps the ring and decays the heat (D4: this
 *   window counts nothing); the bars are its numbers.
 * - **Fragen** — `brain.ask`, the same curator an agent reaches through `brain_ask`, so a person
 *   can see exactly what an agent would be handed.
 *
 * Nothing here polls, animates or holds a frame while the window is hidden or another section is
 * showing: `active` gates every effect.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ipc,
  type AskResult,
  type BrainActivity,
  type BrainServices,
  type PulseEvent,
  type ZoneHeat,
} from "../ipc";

const ACTIVITY_MS = 1500;
const SERVICES_MS = 30_000;
const LOG_KEEP = 120;

interface SceneHandle {
  setHeat(zones: { zone: string; heat: number; reads: number; writes: number }[]): void;
  pulse(event: { zone: string | null; kind: string; paths: string[] }): void;
  setVisible(visible: boolean): void;
  setAmbient(on: boolean): void;
  readonly frames: number;
  dispose(): void;
}

function ago(seconds: number | null | undefined): string {
  if (seconds === null || seconds === undefined) return "nie";
  if (seconds < 60) return "gerade eben";
  const m = Math.floor(seconds / 60);
  if (m < 60) return `vor ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `vor ${h} h`;
  return `vor ${Math.floor(h / 24)} Tagen`;
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function who(actor: string): string {
  if (actor === "human") return "du";
  return actor.replace(/^agent:/, "");
}

const KIND_LABEL: Record<string, string> = {
  read: "liest",
  search: "sucht",
  write: "schreibt",
  refused: "abgelehnt",
};

function methodLabel(method: string): string {
  return method.replace(/^brain\./, "").replace(/^fs\./, "fs ");
}

export function BrainView({ active }: { active: boolean }) {
  const host = useRef<HTMLDivElement | null>(null);
  const scene = useRef<SceneHandle | null>(null);
  const [sceneState, setSceneState] = useState<"loading" | "ready" | "failed">("loading");
  const [sceneNote, setSceneNote] = useState("");
  const [ambient, setAmbient] = useState(false);
  const [activity, setActivity] = useState<BrainActivity | null>(null);
  const [log, setLog] = useState<PulseEvent[]>([]);
  const [services, setServices] = useState<BrainServices | null>(null);
  const [question, setQuestion] = useState("");
  const [asking, setAsking] = useState(false);
  const [answer, setAnswer] = useState<AskResult | null>(null);
  const [askError, setAskError] = useState<string | null>(null);
  const [backingUp, setBackingUp] = useState(false);
  const cursor = useRef(0);
  const activeRef = useRef(active);
  activeRef.current = active;

  // The scene: built once, on first showing, and kept — rebuilding would throw away the camera.
  const [wanted, setWanted] = useState(active);
  useEffect(() => {
    if (active) setWanted(true);
  }, [active]);
  useEffect(() => {
    if (!wanted) return undefined;
    let disposed = false;
    (async () => {
      try {
        const [{ Brain3D }, { assertSnapshot }, answer] = await Promise.all([
          import("@agent-view/brain3d"),
          import("@agent-view/snapshot"),
          ipc.brainGraph(4000),
        ]);
        if (disposed || !host.current) return;
        const snapshot = assertSnapshot(answer) as { nodes?: unknown[] };
        if ((snapshot.nodes?.length ?? 0) === 0) {
          setSceneState("failed");
          setSceneNote("Der Index hat noch keine Knoten zum Zeichnen — einmal `cortex consolidate` laufen lassen.");
          return;
        }
        scene.current = new Brain3D({ container: host.current, snapshot: snapshot as never }) as SceneHandle;
        scene.current.setVisible(activeRef.current);
        setSceneState("ready");
      } catch (e) {
        if (disposed) return;
        setSceneState("failed");
        setSceneNote(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      disposed = true;
      scene.current?.dispose();
      scene.current = null;
    };
  }, [wanted]);

  useEffect(() => {
    scene.current?.setVisible(active);
  }, [active, sceneState]);

  useEffect(() => {
    scene.current?.setAmbient(ambient && active);
  }, [ambient, active, sceneState]);

  // The live pulse.
  useEffect(() => {
    if (!active) return undefined;
    let stopped = false;
    let timer: number | null = null;
    const tick = async () => {
      try {
        const next = await ipc.brainActivity(cursor.current, 120);
        if (stopped) return;
        // The daemon answers only what is newer than the cursor.
        const fresh = next.events;
        // After a daemon restart the sequence starts again from one.
        if (next.seq < cursor.current) cursor.current = 0;
        else cursor.current = next.seq;
        setActivity(next);
        if (fresh.length > 0) {
          setLog((previous) => [...fresh.reverse(), ...previous].slice(0, LOG_KEEP));
          // A backlog after a long absence is not replayed as a firework: the last dozen only.
          for (const e of fresh.slice(0, 12)) scene.current?.pulse(e);
        }
        scene.current?.setHeat(next.zones);
      } catch {
        // The daemon being away is the status line's to say; the log just stops growing.
      }
      if (!stopped) timer = window.setTimeout(tick, ACTIVITY_MS);
    };
    tick();
    return () => {
      stopped = true;
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [active]);

  // The services line.
  useEffect(() => {
    if (!active) return undefined;
    let stopped = false;
    const load = () =>
      ipc
        .brainServices()
        .then((s) => !stopped && setServices(s))
        .catch(() => {});
    load();
    const id = window.setInterval(load, SERVICES_MS);
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [active]);

  const ask = useCallback(async () => {
    const q = question.trim();
    if (!q || asking) return;
    setAsking(true);
    setAskError(null);
    try {
      setAnswer(await ipc.brainAsk(q, 6, true));
    } catch (e) {
      setAskError(e instanceof Error ? e.message : String(e));
    } finally {
      setAsking(false);
    }
  }, [question, asking]);

  const backup = useCallback(async () => {
    setBackingUp(true);
    try {
      await ipc.storeBackup(false);
      setServices(await ipc.brainServices());
    } catch {
      /* the status line keeps the old age, which is the truth */
    } finally {
      setBackingUp(false);
    }
  }, []);

  const zones: ZoneHeat[] = activity?.zones ?? [];
  // Hottest first, as the daemon sorted them: the first is the scale.
  const maxHeat = Math.max(1, zones[0]?.heat ?? 1);
  const agents = activity?.agents ?? [];
  const calls = activity?.calls ?? 0;

  const curator = services?.curator;
  const curatorText = !curator
    ? "…"
    : !curator.enabled
      ? "aus"
      : !curator.reachable
        ? "nicht erreichbar"
        : curator.sleeping
          ? `schläft (${curator.model ?? "Modell"})`
          : `wach (${curator.model ?? "Modell"})`;
  const fed = services?.federation;
  const fedText = !fed
    ? "…"
    : !fed.enabled
      ? "aus"
      : fed.last_error
        ? "Fehler — erneuter Versuch"
        : fed.last_sync_at
          ? `synchron ${ago(Math.floor(Date.now() / 1000) - fed.last_sync_at)}${fed.pending ? `, ${fed.pending} wartend` : ""}`
          : `${fed.pending} wartend`;
  const light = services?.backups.light;

  return (
    <section className="hc-brain hc-section-enter" aria-label="Gehirn">
      <header className="hc-brain-status glass glass-flat" style={{ ["--i" as string]: 0 }}>
        <span className="hc-brain-stat">
          <strong>{activity?.active_agents ?? 0}</strong> Agenten aktiv
        </span>
        <span className="hc-brain-stat">
          <strong>{calls}</strong> Zugriffe / 10 min
        </span>
        <span className="hc-brain-stat" title="Das Entscheidungsmodell hinter brain_ask. Es entlädt sich nach fünf Minuten ohne Frage.">
          Kurator: <strong>{curatorText}</strong>
        </span>
        <span className="hc-brain-stat" title={fed?.basis ?? ""}>
          Main Brain: <strong>{fedText}</strong>
        </span>
        <span className="hc-brain-stat" title={services?.backups.dir ?? ""}>
          Sicherung: <strong>{ago(light?.latest_age_secs)}</strong>
          <button type="button" className="hc-brain-mini" onClick={backup} disabled={backingUp}>
            {backingUp ? "sichert…" : "jetzt"}
          </button>
        </span>
      </header>

      <div className="hc-brain-main">
        <div className="hc-brain-stage glass glass-flat">
          <div className="hc-brain-canvas" ref={host} />
          {sceneState !== "ready" && (
            <p className="hc-brain-note hc-meta">
              {sceneState === "loading" ? "Das Gehirn wird aufgebaut…" : sceneNote}
            </p>
          )}
          <div className="hc-brain-legend hc-meta">
            <span><i className="hc-dot hc-dot-read" /> lesen/suchen</span>
            <span><i className="hc-dot hc-dot-write" /> schreiben</span>
            <span><i className="hc-dot hc-dot-core" /> Hippocampus (Eingang)</span>
            <span>innen Working · außen Cold</span>
            <label className="hc-brain-toggle">
              <input type="checkbox" checked={ambient} onChange={(e) => setAmbient(e.target.checked)} /> drehen
            </label>
          </div>
        </div>

        <aside className="hc-brain-side">
          <section className="hc-brain-card glass glass-flat" aria-label="Heatmap" style={{ ["--i" as string]: 1 }}>
            <h3>Heatmap <span className="hc-meta">— Themenbereiche jetzt</span></h3>
            {zones.length === 0 ? (
              <p className="hc-meta">Gerade fragt niemand etwas. Jeder Zugriff eines Agenten erscheint hier.</p>
            ) : (
              <ol className="hc-heat">
                {zones.slice(0, 10).map((z) => (
                  <li key={z.zone}>
                    <span className="hc-heat-name">{z.zone}</span>
                    <span className="hc-heat-bar">
                      <span
                        className="hc-heat-fill"
                        style={{
                          width: `${Math.max(4, (100 * z.heat) / maxHeat)}%`,
                          ["--heat-write" as string]: `${Math.round((100 * 2 * z.writes) / Math.max(0.001, z.reads + 2 * z.writes))}%`,
                        }}
                      />
                    </span>
                    <span className="hc-heat-num hc-meta">
                      {z.reads.toFixed(0)}⌕ {z.writes.toFixed(0)}✎
                    </span>
                  </li>
                ))}
              </ol>
            )}
          </section>

          <section className="hc-brain-card glass glass-flat" aria-label="Agenten" style={{ ["--i" as string]: 2 }}>
            <h3>Agenten</h3>
            {agents.length === 0 ? (
              <p className="hc-meta">Keiner in den letzten zehn Minuten.</p>
            ) : (
              <ul className="hc-agents">
                {agents.map((a) => (
                  <li key={a.actor}>
                    <span className="hc-agent-name">{who(a.actor)}</span>
                    <span className="hc-meta">
                      {a.calls}× · {methodLabel(a.last_method)} ·{" "}
                      {ago(Math.floor(((activity?.now_ms ?? Date.now()) - a.last_ms) / 1000))}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="hc-brain-card hc-brain-log glass glass-flat" aria-label="Live-Log" style={{ ["--i" as string]: 3 }}>
            <h3>Live-Log</h3>
            {log.length === 0 ? (
              <p className="hc-meta">Noch nichts seit dem Öffnen.</p>
            ) : (
              <ol className="hc-log">
                {log.map((e) => (
                  <li key={e.seq} className={`hc-log-${e.kind}`}>
                    <time className="hc-meta">{clock(e.ts_ms)}</time>
                    <span className="hc-log-who">{who(e.actor)}</span>
                    <span className="hc-log-what">
                      {KIND_LABEL[e.kind] ?? e.kind} <code>{methodLabel(e.method)}</code>
                      {e.label ? <> „{e.label}"</> : null}
                    </span>
                    {e.zone && <span className="hc-log-zone">{e.zone}</span>}
                  </li>
                ))}
              </ol>
            )}
          </section>
        </aside>
      </div>

      <section className="hc-brain-ask glass glass-flat" aria-label="Das Gehirn fragen" style={{ ["--i" as string]: 4 }}>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void ask();
          }}
        >
          <input
            className="hc-brain-question"
            placeholder="Das Gehirn fragen — z. B. „Wo ist entschieden, wie takt ans Server-Gehirn zustellt?“"
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            aria-label="Frage an den Kurator"
          />
          <button type="submit" className="hc-brain-mini" disabled={asking || question.trim() === ""}>
            {asking ? "liest…" : "fragen"}
          </button>
        </form>
        {askError && <p className="hc-meta">{askError}</p>}
        {answer && (
          <div className="hc-answer">
            {answer.answer && <p className="hc-answer-text">{answer.answer}</p>}
            <ol className="hc-answer-items">
              {answer.items.map((item, i) => (
                <li key={`${item.path ?? item.memory_id}-${i}`}>
                  <button
                    type="button"
                    className="hc-answer-path"
                    title="Im Dateimanager öffnen"
                    onClick={() => item.path && ipc.openPath(item.path)}
                    disabled={!item.path}
                  >
                    {item.path ? item.path.replace(/^\/home\/[^/]+\//, "~/") : `Erinnerung #${item.memory_id}`}
                  </button>
                  {item.why && <span className="hc-meta"> — {item.why}</span>}
                </li>
              ))}
            </ol>
            <p className="hc-meta">
              {answer.curator.mode === "model"
                ? `ausgewählt von ${answer.curator.model ?? "dem Kurator"} aus ${answer.curator.candidates} Kandidaten in ${answer.curator.retrieval_ms + answer.curator.model_ms} ms`
                : `Rangfolge von ${answer.curator.candidates} Kandidaten in ${answer.curator.retrieval_ms} ms${answer.curator.note ? ` (${answer.curator.note})` : ""}`}
            </p>
          </div>
        )}
      </section>
    </section>
  );
}
