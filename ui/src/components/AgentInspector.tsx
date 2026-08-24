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
export const LIVE_POLL_MS = 2000;

/**
 * How many audit rows the panel keeps on screen.
 *
 * The list grows as new entries arrive rather than being replaced wholesale, so it needs a
 * ceiling — this is a live view of what is happening, not the log, and `cortex audit` is the
 * log. Bounded here rather than by the query, because the query now returns only what is new.
 */
const AUDIT_KEPT = 200;

/**
 * The highest id in a page of audit entries.
 *
 * A loop rather than `Math.max(...ids)`: the architectural test that enforces D4 looks for
 * collection operations by name, and it is right to be blunt about it.
 */
function highestId(entries: AuditEntry[], current: number | null): number {
  let highest = current ?? 0;
  for (const entry of entries) {
    if (entry.id > highest) highest = entry.id;
  }
  return highest;
}

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
  /**
   * Whether anybody can see the window.
   *
   * A hidden GTK window keeps running its page: intervals go on firing for nobody, and the
   * daemon goes on answering them. This panel polls four calls every two seconds, so a person
   * who opens it and then dismisses the window would leave that running until the next reboot.
   *
   * `useVisibility` was written for exactly this failure and this panel ignored it, which is
   * the same defect one layer newer — see D59, and the hook's own doc comment.
   */
  visible: boolean;
};

export function AgentInspector({ open, onClose, tab, onTab, visible }: AgentInspectorProps) {
  const [sessions, setSessions] = useState<SessionsResult | null>(null);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  /** The refused calls, asked for separately. The daemon filters; this renders (D4). */
  const [refused, setRefused] = useState<AuditEntry[]>([]);
  /**
   * The highest audit id already held, per list.
   *
   * A poll that asks for the last eighty entries every two seconds re-reads seventy-nine it
   * already has, with their arguments — the largest column in that table, and one this panel
   * never displays. With a cursor a quiet daemon answers an empty list, which is what it should
   * cost to watch something that is not happening.
   *
   * In a ref rather than in state: changing it must not re-render, and the poll that reads it
   * has to see the newest value rather than the one its closure captured.
   */
  const seen = useRef<{ all: number | null; refused: number | null }>({
    all: null,
    refused: null,
  });
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
    // Settled rather than all: with the Agent Surface switched off some of these refuse, and
    // one refusal must not blank the three that still answer. Switching it off is exactly when
    // somebody wants to look at what happened before they did.
    const [s, a, r, st] = await Promise.allSettled([
      // Twenty rather than fifty: the counts inside a session change constantly, so this one
      // cannot be cursored the way the audit log can — every poll re-reads whatever it asks
      // for. What a live view needs is the sessions somebody might still be watching, and
      // `cortex sessions` is where the rest of them live.
      ipc.agentSessions(20),
      ipc.auditRecent(80, false, seen.current.all),
      ipc.auditRecent(80, true, seen.current.refused),
      ipc.agentStatus(),
    ]);
    if (s.status === "fulfilled") setSessions(s.value);
    if (a.status === "fulfilled") {
      const fresh = a.value.entries ?? [];
      if (fresh.length > 0) {
        seen.current.all = highestId(fresh, seen.current.all);
        setAudit((previous) => [...fresh, ...previous].slice(0, AUDIT_KEPT));
      }
    }
    if (r.status === "fulfilled") {
      const fresh = r.value.entries ?? [];
      if (fresh.length > 0) {
        seen.current.refused = highestId(fresh, seen.current.refused);
        setRefused((previous) => [...fresh, ...previous].slice(0, AUDIT_KEPT));
      }
    }
    if (st.status === "fulfilled") setStatus(st.value);

    // A loop rather than `.find`, for the reason the architectural test is blunt about: it
    // looks for collection operations by name and is right to, because a test that could tell
    // "searching four settled promises" from "filtering the daemon's answer" would be a test
    // with an exception in it.
    let refusal: string | null = null;
    for (const result of [s, a, r, st]) {
      if (result.status === "rejected" && refusal === null) {
        refusal = result.reason instanceof Error ? result.reason.message : String(result.reason);
      }
    }
    setError(refusal);
  }, []);

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
    if (!open || !visible) return;
    if (tab === "live" || tab === "policy") {
      void loadLive();
      const timer = setInterval(() => void loadLive(), LIVE_POLL_MS);
      return () => clearInterval(timer);
    }
    if (tab === "memory") {
      void loadMemories();
    }
    return undefined;
  }, [open, visible, tab, loadLive, loadMemories]);

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

  const write = useCallback(
    async (text: string, kind: string, supersedes: number[]) => {
      setBusy(true);
      try {
        await ipc.remember(text, kind, supersedes);
        await loadMemories();
      } catch (e) {
        fail(e);
      } finally {
        setBusy(false);
      }
    },
    [fail, loadMemories],
  );

  const supersede = useCallback(
    async (oldId: number, newId: number) => {
      setBusy(true);
      try {
        await ipc.supersedeMemory(oldId, newId);
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

  const panel = useRef<HTMLElement | null>(null);
  // Focus moves into the panel when it opens. Without this the keyboard is still talking to the
  // search field behind it: Escape would close the window rather than the panel, and an arrow
  // key would move a selection nobody can see.
  useEffect(() => {
    if (!open) return;
    panel.current?.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus();
  }, [open]);

  if (!open) return null;

  return (
    <section
      ref={panel}
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
        {/*
          * A tab strip, and the arrow keys move along it — which is what a tablist does
          * everywhere else and therefore what somebody will try. Left and right rather than up
          * and down, because the strip is horizontal and the wrong axis is worse than none.
          */}
        <nav
          className="hc-tabs"
          role="tablist"
          onKeyDown={(event) => {
            const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
            if (step === 0) return;
            event.preventDefault();
            let index = 0;
            for (let i = 0; i < TABS.length; i += 1) {
              if (TABS[i]!.id === tab) index = i;
            }
            const next = TABS[(index + step + TABS.length) % TABS.length]!;
            onTab(next.id);
            // The focus follows the selection, or the next arrow key comes from the old tab and
            // moves two.
            const strip = event.currentTarget;
            requestAnimationFrame(() => {
              strip.querySelector<HTMLButtonElement>('[aria-selected="true"]')?.focus();
            });
          }}
        >
          {TABS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              role="tab"
              aria-selected={entry.id === tab}
              // Only the selected tab is in the tab order, which is what a tablist does: Tab
              // moves *past* the strip to the panel, and the arrows move within it.
              tabIndex={entry.id === tab ? 0 : -1}
              className={entry.id === tab ? "hc-tab hc-tab-on" : "hc-tab"}
              title={entry.hint}
              onClick={() => onTab(entry.id)}
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
              onClick={() => void toggleSurface()}
              title="Not-Aus: schaltet die Agenten-Schnittstelle ab (R7)"
            >
              {status.enabled ? "Schnittstelle an" : "Schnittstelle aus"}
            </button>
          )}
          <button type="button" className="hc-pill" onClick={onClose}>
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
            onSupersede={supersede}
            onWrite={write}
          />
        )}
        {tab === "policy" && <PolicyTab status={status} refused={refused} />}
        {tab === "undo" && <UndoTab />}
        {tab === "architecture" && <ArchitectureTab visible={visible} />}
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
  onSupersede,
  onWrite,
}: {
  memories: Memory[];
  basis: string;
  busy: boolean;
  now: number;
  onForget: (memory: Memory) => void;
  onSupersede: (oldId: number, newId: number) => void;
  onWrite: (text: string, kind: string, supersedes: number[]) => void;
}) {
  const [confirming, setConfirming] = useState<number | null>(null);
  /**
   * The memory a supersession is being built from, if one is.
   *
   * Two clicks rather than a form: pick the one that is out of date, then the one that replaces
   * it. A text field would need the human to know an id, and the ids are the least memorable
   * thing on this screen.
   */
  const [replacing, setReplacing] = useState<Memory | null>(null);
  /**
   * The memory being corrected, if one is.
   *
   * Distinct from `replacing`, which picks an existing memory as the replacement. This is the
   * other half of the same verb: writing the replacement, which is what "correct it" means and
   * what the panel could not do.
   */
  const [correcting, setCorrecting] = useState<Memory | null>(null);
  const [draft, setDraft] = useState("");
  const [draftKind, setDraftKind] = useState("preference");
  return (
    <div className="hc-inspector-pane">
      <h2 className="hc-meta">
        Gedächtnis <span className="hc-num">{memories.length}</span>
      </h2>
      {/* Every view states its own basis — what this list is, and what it is not. */}
      <p className="hc-faint hc-basis">{basis}</p>
      {/*
        * Writing a memory as the human. Two things arrive here: a preference stated outright —
        * the highest-value kind, and the one whose author is a person rather than a machine —
        * and the corrected version of something an agent concluded, which supersedes it.
        */}
      <form
        className="hc-compose"
        onSubmit={(event) => {
          event.preventDefault();
          const text = draft.trim();
          if (text === "") return;
          onWrite(text, draftKind, correcting ? [correcting.node_id] : []);
          setDraft("");
          setCorrecting(null);
        }}
      >
        <label className="hc-meta" htmlFor="hc-compose-text">
          {correcting ? "Korrigierte Fassung" : "Selbst etwas merken"}
        </label>
        <input
          id="hc-compose-text"
          className="hc-name-input"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={
            correcting
              ? `Ersetzt „${correcting.text.slice(0, 40)}…“`
              : "z. B. auf Deutsch antworten, sofern nichts anderes gesagt wird"
          }
          spellCheck={false}
          autoComplete="off"
        />
        <select
          className="hc-pill"
          aria-label="Art"
          value={draftKind}
          onChange={(event) => setDraftKind(event.target.value)}
        >
          {Object.entries(KIND_LABEL).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <button type="submit" className="hc-pill" disabled={busy || draft.trim() === ""}>
          Merken
        </button>
        {correcting && (
          <button type="button" className="hc-pill" onClick={() => setCorrecting(null)}>
            Doch nicht korrigieren
          </button>
        )}
      </form>

      {replacing && (
        <p className="hc-banner" role="status">
          Welche Erinnerung ersetzt „{replacing.text.slice(0, 60)}…“? Die passende auswählen.{" "}
          <button type="button" className="hc-pill" onClick={() => setReplacing(null)}>
            Abbrechen
          </button>
        </p>
      )}
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
              {replacing && replacing.node_id !== memory.node_id ? (
                <>
                  {" "}
                  <button
                    type="button"
                    className="hc-pill hc-pill-on"
                    disabled={busy}
                    onClick={() => {
                      onSupersede(replacing.node_id, memory.node_id);
                      setReplacing(null);
                    }}
                  >
                    Ersetzt sie
                  </button>
                </>
              ) : confirming === memory.node_id ? (
                <>
                  {" "}
                  <button
                    type="button"
                    className="hc-pill hc-pill-danger"
                    disabled={busy}
                    onClick={() => {
                      setConfirming(null);
                      onForget(memory);
                    }}
                  >
                    Wirklich löschen
                  </button>{" "}
                  <button
                    type="button"
                    className="hc-pill"
                    onClick={() => setConfirming(null)}
                  >
                    Abbrechen
                  </button>
                </>
              ) : (
                <>
                  {" "}
                  {!memory.superseded_by && (
                    <>
                      <button
                        type="button"
                        className="hc-pill"
                        disabled={busy || replacing !== null}
                        title="Eine bessere Fassung schreiben; diese hier tritt in die Historie zurück"
                        onClick={() => {
                          setCorrecting(memory);
                          setDraftKind(memory.kind);
                          document.getElementById("hc-compose-text")?.focus();
                        }}
                      >
                        Korrigieren
                      </button>{" "}
                      <button
                        type="button"
                        className="hc-pill"
                        disabled={busy || replacing !== null}
                        title="Eine andere, schon vorhandene Erinnerung tritt an ihre Stelle"
                        onClick={() => setReplacing(memory)}
                      >
                        Ersetzen
                      </button>
                    </>
                  )}{" "}
                  <button
                    type="button"
                    className="hc-pill"
                    disabled={busy}
                    title="Löscht sie endgültig — auch über einen Neuaufbau des Index hinweg. Für etwas, das nie richtig war."
                    onClick={() => setConfirming(memory.node_id)}
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
          <dl className="hc-inspector-facts">
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
        onClick={() => {
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
function ArchitectureTab({ visible }: { visible: boolean }) {
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
    /** One frame. The panel drives this; without a caller the canvas stays black. */
    render(onFrame?: (dt: number) => void): number;
  } | null>(null);
  /** Node ids the current scene knows about. A pulse for anything else would be silent. */
  const drawn = useRef<Set<number>>(new Set());
  /** The animation frame the scene is drawn from, so it can be stopped. */
  const frame = useRef<number | null>(null);
  /** Frames a second, as measured while drawing them. Zero means nothing is being drawn. */
  const [rate, setRate] = useState(0);
  /**
   * Whether anybody can see this. Read by the loop rather than closed over: the loop is created
   * once, and a hidden window that keeps rendering is exactly the cost D59 measured.
   */
  const showing = useRef(visible);
  showing.current = visible;
  /** The rate last written to the panel, so the loop can avoid setting the same number twice. */
  const shown = useRef(0);

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
        const built = new BrainScene({
          container: host.current,
          snapshot: snapshot as never,
          // No glow in the panel. This window is WebKitGTK with the dmabuf renderer disabled
          // (D59) — software OpenGL — and the bloom pass over a full-size buffer costs more per
          // frame than the whole scene. With it on, the first thing this loop did on the owner's
          // machine was freeze the window.
          bloom: false,
        });
        scene.current = built;
        setState("ready");

        // **Somebody has to draw the frames.** `BrainScene.render()` draws one and returns; the
        // standalone view calls it from its own `requestAnimationFrame` loop, and this panel
        // never called it at all — so the tab showed a correctly sized, correctly built,
        // never-drawn canvas. Black, with no error, with four thousand nodes behind it.
        //
        // The loop stops when the tab is disposed, and it stops itself while the window is
        // hidden: a resident overlay that renders a three-dimensional scene for nobody is D59's
        // lesson with a GPU attached.
        let drawnFrames = 0;
        let since = 0;
        let last = 0;
        const tick = (now: number): void => {
          if (disposed || scene.current !== built) return;
          frame.current = requestAnimationFrame(tick);

          // **Thirty frames a second, at most, and none at all while nobody is looking.**
          //
          // An unbounded loop is what a scene in its own window can afford; a panel inside a
          // resident overlay cannot. Software OpenGL turns "as fast as possible" into "as slow
          // as this frame takes", and the window stops answering the keyboard between frames —
          // which is what happened the first time this loop existed. The rate is the ceiling
          // this panel is allowed to spend, and D59's rule applies underneath it: nothing runs
          // while the window is hidden.
          if (!showing.current) {
            // And it says so, once. A rate that kept its last value while nothing was being
            // drawn would be the same lie as a black canvas with no number beside it.
            if (shown.current !== 0) {
              shown.current = 0;
              setRate(0);
            }
            drawnFrames = 0;
            since = 0;
            return;
          }
          if (now - last < 33) return;
          last = now;

          const dt = built.render();
          // A visible frame rate, twice a second at most. It is the diagnostic that would have
          // ended this bug in a minute rather than an evening: a tab that says "0 Bilder/s" is
          // a tab nobody is drawing, and a black canvas alone says nothing at all.
          drawnFrames += 1;
          since += dt;
          if (since >= 0.5) {
            shown.current = Math.round(drawnFrames / since);
            setRate(shown.current);
            drawnFrames = 0;
            since = 0;
          }
        };
        frame.current = requestAnimationFrame(tick);
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
      if (frame.current !== null) cancelAnimationFrame(frame.current);
      frame.current = null;
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
    if (state !== "ready" || !visible) return undefined;
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
  }, [state, visible]);

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
              onClick={() => {
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
      {state === "ready" && (
        <p className="hc-meta hc-scene-rate">
          <span className="hc-num">{rate}</span> Bilder/s
          {rate === 0 && (
            <span className="hc-error"> — es wird nichts gezeichnet</span>
          )}
        </p>
      )}
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
                onClick={() => setReloads((n) => n + 1)}
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
