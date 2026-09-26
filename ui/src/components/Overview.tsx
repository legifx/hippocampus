/**
 * The start screen: what is in here, and what the agents have been doing to it.
 *
 * **Why it is arranged the way it is.** The first question somebody has about a memory service is
 * "what is in it", and the second is "is the agent side actually being used, or is it decoration".
 * Those are two different shapes of answer — a few big numbers, and a few breakdowns — so they get
 * two different regions: a row of tiles for the numbers that stand alone, and cards for the
 * numbers that only mean something next to the other numbers in their group.
 *
 * **What this component does not do.** It does not add up, average, sort or shade anything: every
 * count, every share and the sentence at the top are computed by the daemon (D4). The window's job
 * here is to put them where the eye expects them and to say which reading they came from. The one
 * thing this screen prints itself is the timestamp, because "when was this true" is a fact about
 * the window's own clock, not about the index.
 *
 * The headline is the exception that proves the rule: it is prose, it comes from the daemon, and it
 * is shown untouched. A window that rephrased it would be a window making claims about the agents
 * on its own authority.
 */

import { motion } from "framer-motion";
import { formatAge, formatSize, messageOf } from "../format";
import { OVERVIEW_WINDOWS, type OverviewSession, type Slice } from "../ipc";
import { GlassButton } from "./glass";
import type { OverviewState } from "../hooks/useOverview";
import { spring, stagger } from "../motion";
import { Glyph } from "./NavRail";

/**
 * What the daemon's vocabulary is called on screen.
 *
 * The daemon sends names, not labels: `tier: "cold"` is a fact about the database, and „kalt" is
 * how a person says it. An unknown name falls through to the raw one — a vocabulary that grows
 * shows up as one unlabelled row instead of vanishing from the screen.
 */
const NAMES: Record<string, string> = {
  working: "Arbeitsspeicher",
  stm: "Kurzzeit",
  ltm: "Langzeit",
  cold: "kalt",
  pending: "wartet",
  done: "extrahiert",
  skipped: "übersprungen",
  failed: "fehlgeschlagen",
  human: "Mensch",
  fact: "Fakten",
  preference: "Vorlieben",
  procedure: "Vorgehen",
  decision: "Entscheidungen",
  failure: "Fehlschläge",
  observation: "Beobachtungen",
};

function label(name: string): string {
  const named = NAMES[name];
  if (named !== undefined) return named;
  // `agent:claude` is an actor, not a word in the vocabulary: it reads as „Agent claude".
  if (name.startsWith("agent:")) return `Agent ${name.slice("agent:".length)}`;
  return name;
}

/**
 * The colour of a segment.
 *
 * A name that has a meaning which must not move — a failure is red wherever it appears, a
 * successful extraction is green — gets that meaning. Everything else takes a step of one neutral
 * ramp, in the order the daemon sent it, because the order of a breakdown is the daemon's reading
 * of it and the ramp is a picture of that order and nothing else.
 */
const TONE: Record<string, string> = {
  done: "hc-tone-done",
  failed: "hc-tone-failed",
  skipped: "hc-tone-skipped",
};

function toneOf(part: Slice, index: number): string {
  const named = TONE[part.name];
  if (named !== undefined) return named;
  return `hc-ramp-${Math.min(index, 4)}`;
}

function percent(share: number): string {
  return new Intl.NumberFormat("de-DE", {
    style: "percent",
    maximumFractionDigits: share > 0 && share < 0.1 ? 1 : 0,
  }).format(share);
}

/**
 * A breakdown, drawn twice: once as proportions and once as numbers.
 *
 * Both halves are needed and neither can do the other's job — a bar alone cannot be read at
 * 3 % against 2 %, and a table alone cannot be read at a glance. The legend carries the counts the
 * daemon sent; nothing here is recomputed.
 */
function Breakdown({ parts, empty }: { parts: Slice[]; empty: string }) {
  if (parts.length === 0) return <p className="hc-ov-note">{empty}</p>;

  return (
    <>
      <div className="hc-bar" role="img">
        {parts.map((part, index) => (
          <span
            key={part.name}
            className={`hc-bar-seg ${toneOf(part, index)}`}
            style={{ flexGrow: part.share }}
            title={`${label(part.name)}: ${part.count.toLocaleString("de-DE")}`}
          />
        ))}
      </div>
      <div className="hc-legend">
        {parts.map((part, index) => (
          <span key={part.name} className="hc-legend-item">
            <span className={`hc-legend-dot ${toneOf(part, index)}`} />
            {label(part.name)}
            <span className="hc-legend-count">{part.count.toLocaleString("de-DE")}</span>
            <span className="hc-legend-share">{percent(part.share)}</span>
          </span>
        ))}
      </div>
    </>
  );
}

/*
 * The start screen arrives the way the panels do (§5): each element on the content spring, one
 * after another — and nothing at all when the person asked for less motion, because `spring` and
 * `stagger` both collapse in that case. There is no second place that has to know the preference.
 *
 * The delay is the element's own index rather than a container's `staggerChildren`, because these
 * are grid children: a wrapper per tile would be a box that exists only to be animated, and the
 * grid would then be laying out wrappers. `stagger` caps the index on its own, so the rule about
 * "surfaces with fewer than about twenty elements" cannot be broken by adding a card here.
 */
const arrival = (index: number) => ({
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0 },
  transition: { ...spring("content"), delay: stagger(index, true) },
});

/** One number that stands on its own. `opens` marks the ones that lead somewhere. */
function Tile({
  index,
  title,
  value,
  sub,
  meta,
  opens,
  onClick,
}: {
  index: number;
  title: string;
  value: number;
  sub?: string;
  meta?: string;
  opens?: boolean;
  onClick?: () => void;
}) {
  const body = (
    <>
      <span className="hc-tile-label">{title}</span>
      <span className="hc-tile-value">{value.toLocaleString("de-DE")}</span>
      {sub && <span className="hc-tile-sub">{sub}</span>}
      {meta && <span className="hc-tile-meta">{meta}</span>}
    </>
  );

  if (opens && onClick) {
    return (
      <motion.button
        type="button"
        className="hc-tile"
        data-opens="true"
        onClick={onClick}
        {...arrival(index)}
      >
        {body}
      </motion.button>
    );
  }

  return (
    <motion.div className="hc-tile" {...arrival(index)}>
      {body}
    </motion.div>
  );
}

/** A ratio drawn as a fill in a well. */
function Meter({ share, tone }: { share: number; tone?: string }) {
  return (
    <div className="hc-meter">
      <div
        className={`hc-meter-fill ${tone ?? ""}`}
        style={{ width: `${Math.min(Math.max(share, 0), 1) * 100}%` }}
      />
    </div>
  );
}

function Card({
  index,
  title,
  note,
  children,
}: {
  index: number;
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <motion.section className="hc-card glass glass-sheen" {...arrival(index)}>
      <div className="hc-card-title">
        <span className="hc-card-title-text">{title}</span>
        {note && <span className="hc-card-note">{note}</span>}
      </div>
      {children}
    </motion.section>
  );
}

function Row({ k, v, unit }: { k: string; v: string; unit?: string }) {
  return (
    <div className="hc-kv-row">
      <span className="hc-kv-key">{k}</span>
      <span className="hc-kv-dots" />
      <span className="hc-kv-value">
        {v}
        {unit && <span className="hc-kv-unit">{unit}</span>}
      </span>
    </div>
  );
}

/** One agent session: one run at something, and what it cost and earned. */
function SessionLine({ session, now }: { session: OverviewSession; now: number }) {
  const open = session.ended_at === undefined || session.ended_at === null;

  return (
    <div className="hc-session">
      <span className="hc-session-name">{session.name}</span>
      <span className="hc-session-task" title={session.task ?? ""}>
        {session.task ?? "ohne Aufgabe"}
      </span>
      <span className="hc-session-meta">
        <span className={open ? "" : "hc-session-meta-bad"}>
          {open ? "läuft" : `vor ${formatAge(session.started_at, now)}`}
        </span>
        <span>{session.calls.toLocaleString("de-DE")} Aufrufe</span>
        {session.refusals > 0 && (
          <span className="hc-session-meta-bad">
            {session.refusals.toLocaleString("de-DE")} abgelehnt
          </span>
        )}
        <span>{session.memories.toLocaleString("de-DE")} Erinnerungen</span>
      </span>
    </div>
  );
}

export function Overview({
  state,
  windowSecs,
  onWindow,
  onFiles,
  onAgents,
  onAudit,
}: {
  state: OverviewState;
  windowSecs: number;
  onWindow: (secs: number) => void;
  onFiles: () => void;
  onAgents: () => void;
  onAudit: () => void;
}) {
  const overview = state.overview;
  const now = Date.now() / 1000;

  return (
    <div className="hc-ov" role="region" aria-label="Übersicht">
      <div className="hc-ov-inner">
        <div className="hc-ov-head">
          <h1 className="hc-ov-title">Übersicht</h1>
          <span className="hc-ov-when">
            {overview
              ? `Stand ${new Date(overview.generated_at * 1000).toLocaleTimeString("de-DE")}`
              : state.loading
                ? "liest …"
                : "kein Stand"}
          </span>
          <span className="hc-ov-head-mid" />
          <div className="hc-segs seg seg-md well" role="group" aria-label="Zeitfenster">
            {OVERVIEW_WINDOWS.map((w) => (
              <button
                key={w.secs}
                type="button"
                className={
                  w.secs === (overview ? overview.window_secs : windowSecs)
                    ? "hc-seg seg-item raised"
                    : "hc-seg seg-item"
                }
                aria-pressed={w.secs === (overview ? overview.window_secs : windowSecs)}
                onClick={() => onWindow(w.secs)}
              >
                {w.label}
              </button>
            ))}
          </div>
          {/* The kit's glass button, in the pill shape, because this is a control and not a chip
              that reports something: the card chips below stay spans. */}
          <GlassButton size="sm" variant="glass" onClick={state.reload}>
            neu lesen
          </GlassButton>
        </div>

        {overview === null ? (
          <section className="hc-card glass glass-sheen">
            <p className="hc-ov-note hc-ov-error">
              {state.error
                ? `Der Daemon antwortet nicht: ${messageOf(state.error)}`
                : "Der Daemon hat noch nichts gezählt."}
            </p>
          </section>
        ) : (
          <>
            <motion.section className="hc-card glass glass-sheen hc-ov-hero" {...arrival(0)}>
              {/* `.raised-flat`, not the kit's `raised`: this is a badge on the ground and not a row,
                  so §8.2 would allow the drop — but the badge never had one, and a port that moves a
                  material to one place is not the job in which to give it one. */}
              <span className="hc-ov-hero-glyph raised-flat">
                <Glyph name="brain" />
              </span>
              <div className="hc-ov-hero-text">
                <p className="hc-ov-headline">{overview.headline}</p>
                <p className="hc-ov-basis">{overview.basis}</p>
                <div className="hc-chip-row">
                  <GlassButton size="sm" variant="ghost" onClick={onAgents}>
                    Agenten-Inspector
                  </GlassButton>
                  <GlassButton size="sm" variant="ghost" onClick={onAudit}>
                    Audit-Protokoll
                  </GlassButton>
                  <GlassButton size="sm" variant="ghost" onClick={onFiles}>
                    Dateien ansehen
                  </GlassButton>
                </div>
              </div>
            </motion.section>

            <div className="hc-tiles">
              <Tile
                index={1}
                title="Dateien"
                value={overview.index.files}
                sub={`in ${overview.index.dirs.toLocaleString("de-DE")} Ordnern`}
                opens
                onClick={onFiles}
              />
              <Tile
                index={2}
                title="Blöcke"
                value={overview.index.chunks}
                sub={`${overview.index.embeddings.toLocaleString("de-DE")} Vektoren`}
                meta={`Abdeckung ${percent(overview.index.vector_coverage)}`}
              />
              <Tile
                index={3}
                title="Episoden"
                value={overview.index.episodes}
                meta={`${overview.index.episodes_window.toLocaleString("de-DE")} im Fenster`}
              />
              <Tile
                index={4}
                title="Erinnerungen"
                value={overview.memory.total}
                sub={`${overview.memory.human_confirmed.toLocaleString("de-DE")} bestätigt`}
              />
              <Tile
                index={5}
                title="Konzepte"
                value={overview.index.concepts}
                sub={`${overview.index.edges.toLocaleString("de-DE")} Verknüpfungen`}
              />
              <Tile
                index={6}
                title="Agentenaufrufe"
                value={overview.agents.calls_window}
                sub={`${overview.agents.refusals_window.toLocaleString("de-DE")} abgelehnt`}
                meta={overview.window_label}
                opens
                onClick={onAudit}
              />
            </div>

            <div className="hc-cards">
              <Card index={7} title="Ebenen" note={`${overview.index.zones} Zonen`}>
                <Breakdown
                  parts={overview.index.tiers}
                  empty="Noch keine Datei indexiert."
                />
                <div className="hc-kv">
                  <Row
                    k="verknüpft"
                    v={overview.index.pinned.toLocaleString("de-DE")}
                    unit="Dateien"
                  />
                  <Row
                    k="Kopien"
                    v={overview.index.duplicates.toLocaleString("de-DE")}
                    unit="Dateien"
                  />
                  <Row
                    k="Vektoren zum Verwerfen"
                    v={overview.index.vectors_to_drop.toLocaleString("de-DE")}
                  />
                </div>
              </Card>

              <Card index={8} title="Extraktion" note={`${overview.index.annotations} Notizen`}>
                <Breakdown parts={overview.index.extract} empty="Keine Datei wartet." />
                <div className="hc-kv">
                  <Row
                    k="Vektoren"
                    v={percent(overview.index.vector_coverage)}
                    unit={`von ${overview.index.chunks.toLocaleString("de-DE")} Blöcken`}
                  />
                  <Row
                    k="Konzepte"
                    v={overview.index.concepts.toLocaleString("de-DE")}
                    unit={`aus ${overview.index.episodes.toLocaleString("de-DE")} Episoden`}
                  />
                </div>
              </Card>

              <Card index={9} title="Werkzeuggebrauch" note={overview.window_label}>
                <div className="hc-kv">
                  <Row
                    k="Aufrufe"
                    v={overview.agents.calls_window.toLocaleString("de-DE")}
                    unit={`von ${overview.agents.calls_total.toLocaleString("de-DE")} insgesamt`}
                  />
                  <Row
                    k="abgelehnt"
                    v={overview.agents.refusals_window.toLocaleString("de-DE")}
                    unit={percent(overview.agents.refusal_share)}
                  />
                </div>
                <Meter share={overview.agents.refusal_share} />
                <Breakdown
                  parts={overview.agents.tools.map((tool) => ({
                    name: tool.tool,
                    count: tool.calls,
                    share: tool.share,
                  }))}
                  empty="Kein Agent hat die Werkzeuge benutzt."
                />
                <div className="hc-kv">
                  <Row
                    k="Kontextpakete"
                    v={overview.agents.packs_window.toLocaleString("de-DE")}
                    unit={`von ${overview.agents.packs_total.toLocaleString("de-DE")}`}
                  />
                  <Row
                    k="Paketpositionen"
                    v={overview.agents.pack_items.toLocaleString("de-DE")}
                    unit="insgesamt"
                  />
                  <Row
                    k="davon geholfen"
                    v={overview.agents.pack_helped.toLocaleString("de-DE")}
                    unit={`Rauschen ${overview.agents.pack_noise.toLocaleString("de-DE")}`}
                  />
                </div>
              </Card>

              <Card
                index={10}
                title="Sitzungen"
                note={`${overview.agents.sessions_total} insgesamt · ${overview.agents.sessions_open} offen`}
              >
                {overview.agents.sessions.length === 0 ? (
                  <p className="hc-ov-note">Noch kein Agent hat hier gearbeitet.</p>
                ) : (
                  overview.agents.sessions.map((session) => (
                    <SessionLine key={session.id} session={session} now={now} />
                  ))
                )}
              </Card>

              <Card index={11} title="Erinnerungen" note={`${overview.memory.pinned} verknüpft`}>
                <Breakdown parts={overview.memory.kinds} empty="Noch nichts erinnert." />
                <div className="hc-kv">
                  <Row
                    k="bestätigt"
                    v={overview.memory.human_confirmed.toLocaleString("de-DE")}
                    unit={`ersetzt ${overview.memory.superseded.toLocaleString("de-DE")}`}
                  />
                  <Row
                    k="geholfen"
                    v={overview.memory.helped.toLocaleString("de-DE")}
                    unit={`Rauschen ${overview.memory.noise.toLocaleString("de-DE")}`}
                  />
                  <Row
                    k="nie abgerufen"
                    v={overview.memory.never_retrieved.toLocaleString("de-DE")}
                    unit={`abgerufen ${overview.memory.retrieved.toLocaleString("de-DE")}`}
                  />
                  <Row
                    k="läuft ab"
                    v={overview.memory.expiring.toLocaleString("de-DE")}
                  />
                </div>
                <Breakdown parts={overview.memory.scopes} empty="Keine Geltungsbereiche." />
              </Card>

              <Card
                index={12}
                title="Letzter Konsolidierungslauf"
                note={
                  overview.consolidation
                    ? `vor ${formatAge(overview.consolidation.ran_at, now)}`
                    : undefined
                }
              >
                {overview.consolidation === null ? (
                  <p className="hc-ov-note">
                    Die Konzepte sind noch nie durchgegangen. Bis dahin sind die Episoden roh.
                  </p>
                ) : (
                  <>
                    <div className="hc-kv">
                      <Row
                        k="Episoden gelesen"
                        v={overview.consolidation.episodes_replayed.toLocaleString("de-DE")}
                        unit={`${overview.consolidation.bursts} Ausbrüche`}
                      />
                      <Row
                        k="verworfen"
                        v={overview.consolidation.bursts_discarded.toLocaleString("de-DE")}
                        unit="Ausbrüche"
                      />
                      <Row
                        k="Verknüpfungen geschrieben"
                        v={overview.consolidation.edges_written.toLocaleString("de-DE")}
                        unit={`${overview.consolidation.movements} bewegt`}
                      />
                      <Row
                        k="Konzepte"
                        v={overview.consolidation.concepts_total.toLocaleString("de-DE")}
                        unit={`${overview.consolidation.concepts_pruned} verworfen`}
                      />
                      <Row
                        k="Kopien"
                        v={overview.consolidation.duplicates.toLocaleString("de-DE")}
                        unit={`${overview.consolidation.orphans} verwaist`}
                      />
                      <Row
                        k="Erinnerungen"
                        v={overview.consolidation.memories_total.toLocaleString("de-DE")}
                        unit={`${overview.consolidation.memories_expired} abgelaufen`}
                      />
                      <Row
                        k="Dauer"
                        v={(overview.consolidation.duration_ms / 1000).toFixed(1)}
                        unit="s"
                      />
                    </div>
                    {overview.consolidation.dry_run && (
                      <div className="hc-chip-row">
                        <span className="hc-chip" data-tone="info">
                          <span className="hc-chip-dot" />
                          Probelauf
                        </span>
                      </div>
                    )}
                  </>
                )}
              </Card>
            </div>

            <div className="hc-chip-row">
              <span
                className="hc-chip"
                data-tone={overview.daemon.agent_surface_enabled ? "on" : "off"}
              >
                <span className="hc-chip-dot" />
                {overview.daemon.agent_surface_enabled ? "Agenten an" : "Agenten aus"}
              </span>
              <span className="hc-chip" data-tone={overview.daemon.indexing_paused ? "warn" : "on"}>
                <span className="hc-chip-dot" />
                {overview.daemon.indexing_paused ? "Indexierung pausiert" : "Indexierung läuft"}
              </span>
              {overview.daemon.watcher_active !== null && (
                <span
                  className="hc-chip"
                  data-tone={overview.daemon.watcher_active ? "on" : "warn"}
                >
                  <span className="hc-chip-dot" />
                  {overview.daemon.watcher_active ? "Wächter läuft" : "Wächter steht"}
                </span>
              )}
              {overview.daemon.require_dry_run && (
                <span className="hc-chip" data-tone="info">
                  <span className="hc-chip-dot" />
                  immer Probelauf
                </span>
              )}
              {overview.daemon.write_allowlist.length > 0 && (
                <span className="hc-chip" data-long="true" title={overview.daemon.write_allowlist.join(", ")}>
                  <span className="hc-chip-dot" />
                  Schreiben nur in {overview.daemon.write_allowlist.join(", ")}
                </span>
              )}
              <span className="hc-chip" title={overview.daemon.database}>
                <span className="hc-chip-dot" />
                {formatSize(overview.daemon.index_memory_bytes)} Pfadindex
              </span>
              <span className="hc-chip">
                <span className="hc-chip-dot" />
                cortexd {overview.daemon.version} · Schema {overview.daemon.schema_version}
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

