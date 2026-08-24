/**
 * What the brain knows about the selected file.
 *
 * This is the panel that makes the application worth having over `fzf`: not "here is a file"
 * but "here is what it is connected to, what was written about it, and what happened to it".
 * Everything in it comes from `brain.neighbors` — the frontend asks and renders, and works out
 * nothing of its own.
 *
 * Stagger applies here (design system §5) because this surface has a handful of elements rather
 * than two hundred.
 */

import { AnimatePresence, motion } from "framer-motion";
import { useEffect, useState } from "react";
import { ipc, type Memory, type NodeWhy, type SearchHit, type Source } from "../ipc";
import { formatAge, splitPath } from "../format";
import type { Row } from "../row";
import { spring, stagger } from "../motion";

/** What an episode's verb is called in a window whose default locale is German. */
const ACTION_LABEL: Record<string, string> = {
  create: "angelegt",
  modify: "geändert",
  open: "geöffnet",
  move: "verschoben",
  delete: "gelöscht",
  search: "gesucht",
  annotate: "notiert",
};

/** The parts of a salience score, in the order they are worth reading. */
const COMPONENT_LABEL: [keyof NonNullable<NodeWhy["components"]>, string][] = [
  ["recency", "zuletzt benutzt"],
  ["access", "wie oft benutzt"],
  ["edits", "wie oft geändert"],
  ["centrality", "wie verbunden"],
  ["pin", "angeheftet"],
  ["project", "im Projekt"],
  ["noise", "Rauschen"],
];

/** Words for the memory kinds, so a panel in German does not print an English enum. */
const KIND_LABEL: Record<string, string> = {
  fact: "Fakt",
  preference: "Vorliebe",
  procedure: "Ablauf",
  decision: "Entscheidung",
  failure: "Fehlschlag",
  observation: "Beobachtung",
};

export function ContextSidebar({
  hit,
  row,
  onOpenSource,
}: {
  hit: SearchHit | null;
  row?: Row | null;
  /** Go somewhere: the graph around this file is a place now, not a list in here (M10 §4.5). */
  onOpenSource?: (source: Source) => void;
}) {
  const [why, setWhy] = useState<NodeWhy | null>(null);
  const [note, setNote] = useState("");
  const [saying, setSaying] = useState<string | null>(null);
  /**
   * What agents have learned about this file.
   *
   * A second call rather than a field of the first, because "the graph around this node" and
   * "what is known about it that is not in it" are two questions and the daemon answers them
   * with two methods. Merging them here would be the frontend deciding what a context panel is,
   * which is the decision D4 keeps on the other side of the socket.
   */
  const [memories, setMemories] = useState<Memory[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hit) {
      setWhy(null);
      return;
    }

    // A selection can move faster than the daemon answers — an arrow key held down produces one
    // request per row — so a late answer for a row nobody is on any more is dropped.
    let current = true;
    setError(null);
    // Failing to fetch these must not blank the panel: what an agent learned is the newest
    // thing here and the least load-bearing.
    ipc
      .memoriesAbout(hit.path)
      .then((result) => current && setMemories(result.memories ?? []))
      .catch(() => current && setMemories([]));
    // Why this row is where it is, and what has happened to it — one call, because they are one
    // question asked at one moment (M10 step 4).
    ipc
      .nodeWhy(hit.path)
      .then((result) => current && setWhy(result))
      .catch(() => {
        // A file the index has never seen has no history and no score, which is a fact about
        // the file rather than a failure: the panel shows what it does have.
        if (current) setWhy(null);
      });

    return () => {
      current = false;
    };
  }, [hit?.path]);

  // The note field belongs to the file it was opened on: carrying half a sentence to the next
  // row would attach it to something else at the next Enter.
  useEffect(() => {
    setNote("");
    setSaying(null);
  }, [hit?.path]);

  if (!hit) {
    return (
      <aside className="hc-context" aria-label="Kontext">
        <p className="hc-faint">Nichts ausgewählt</p>
      </aside>
    );
  }

  const notes = why?.annotations ?? [];

  return (
    <aside className="hc-context" aria-label="Kontext">
      <h2 className="hc-title hc-context-title">{splitPath(hit.path).name}</h2>

      <Section title="Notizen" count={notes.length} empty="noch keine">
        <AnimatePresence initial={false}>
          {notes.map((note, index) => (
            <motion.div
              key={note.node_id}
              className="hc-note"
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ ...spring("content"), delay: stagger(index, true) }}
            >
              <p className="hc-note-text">{note.note}</p>
              <p className="hc-meta">
                {note.tags.map((tag) => (
                  <span key={tag} className="hc-pill">
                    {tag}
                  </span>
                ))}
                <span className="hc-num">{formatAge(note.ts)}</span>
                <span>{note.actor}</span>
              </p>
            </motion.div>
          ))}
        </AnimatePresence>
      </Section>

      <Section title="Agenten wissen" count={memories.length} empty="noch nichts gelernt">
        <AnimatePresence initial={false}>
          {memories.map((memory, index) => (
            <motion.div
              key={memory.node_id}
              className="hc-note hc-memory"
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ ...spring("content"), delay: stagger(index, true) }}
            >
              <p className="hc-note-text">{memory.text}</p>
              <p className="hc-meta">
                <span className="hc-pill">{KIND_LABEL[memory.kind] ?? memory.kind}</span>
                <span className="hc-num">{formatAge(memory.ts)}</span>
                <span>{memory.actor}</span>
                {memory.human_confirmed && <span title="Von einem Menschen bestätigt">✓</span>}
              </p>
            </motion.div>
          ))}
        </AnimatePresence>
      </Section>

      {/* Writing a note without leaving the window, and without touching the file (M10 §4.5).
          One line, Enter to keep it: a dialog for a sentence is a dialog nobody opens. */}
      <form
        className="hc-annotate"
        onSubmit={(event) => {
          event.preventDefault();
          const text = note.trim();
          if (text === "" || !hit) return;
          setNote("");
          setSaying("…");
          ipc
            .annotate(hit.path, text)
            .then(() => {
              setSaying("notiert");
              // Read it back rather than pushing it into the list here: what the panel shows
              // has to be what the daemon stored, or the two drift the first time one of them
              // rejects something.
              return ipc.nodeWhy(hit.path).then((result) => setWhy(result));
            })
            .catch((e: unknown) => setSaying(e instanceof Error ? e.message : "nicht gespeichert"));
        }}
      >
        <input
          className="hc-annotate-input"
          value={note}
          onChange={(event) => setNote(event.target.value)}
          // The window handles Enter — it opens the selected row — and it sees this key on its
          // way up. Without stopping it here, typing a note and pressing Enter opens a file
          // instead of keeping the note, which is how this was found.
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === "Escape") event.stopPropagation();
            if (event.key === "Escape") setNote("");
          }}
          placeholder="Notiz zu dieser Datei …"
          aria-label="Notiz"
        />
        {saying && <span className="hc-faint hc-annotate-said">{saying}</span>}
      </form>

      {/* The graph around this file is a place, so this is a way to *go* there rather than a
          list in a panel that could only ever show the first few (M10 §4.5). */}
      {onOpenSource && hit && (
        <button
          type="button"
          className="hc-context-open"
          onClick={() => onOpenSource({ kind: "neighbours", path: hit.path })}
        >
          Verbunden ansehen{why?.degree ? ` (${why.degree})` : ""}
        </button>
      )}

      {/* Why this row is where it is: the parts of the score the consolidation pass computed and
          stored, not a number recomputed here. Absent until a pass has run, and said rather than
          shown as a row of zeroes — a fresh index holds no judgements. */}
      <Section
        title="Warum"
        count={why?.components ? 1 : 0}
        empty={
          why === null
            ? "diese Datei kennt der Index noch nicht"
            : "noch nicht bewertet — die Konsolidierung war hier noch nicht"
        }
      >
        {why?.components && (
          <dl className="hc-facts hc-meta hc-why">
            {COMPONENT_LABEL.map(([key, label]) => {
              const value = Number(why.components?.[key] ?? 0);
              if (value === 0) return null;
              return (
                <div key={key}>
                  <dt>{label}</dt>
                  <dd className="hc-num">{value > 0 ? `+${value.toFixed(2)}` : value.toFixed(2)}</dd>
                </div>
              );
            })}
            <div>
              <dt>Salienz</dt>
              <dd className="hc-num">{why.salience.toFixed(2)}</dd>
            </div>
          </dl>
        )}
      </Section>

      {/* What happened to it, from the episode log — which is the half `mtime` cannot answer:
          reading a file is an episode too. */}
      <Section title="Verlauf" count={why?.episodes.length ?? 0} empty="nichts aufgezeichnet">
        {(why?.episodes ?? []).slice(0, 8).map((episode) => (
          <p key={episode.id} className="hc-episode hc-meta">
            <span>{ACTION_LABEL[episode.action] ?? episode.action}</span>
            <span className="hc-num">{formatAge(episode.ts)}</span>
            <span className="hc-faint">{episode.actor}</span>
          </p>
        ))}
      </Section>

      {row && (row.mode || row.owner || row.linkTarget !== null) && (
        // The `stat` facts, which only a directory listing has: a search result comes from the
        // index, and the index stores what a file is rather than the permissions it had when it
        // was last read. An out-of-date mode would be worse than none.
        <dl className="hc-facts hc-meta">
          {row.mode && (
            <div>
              <dt>Rechte</dt>
              <dd className="hc-num">{row.mode}</dd>
            </div>
          )}
          {row.owner && (
            <div>
              <dt>Besitzer</dt>
              <dd>{row.owner}</dd>
            </div>
          )}
          {row.linkTarget !== null && (
            <div>
              <dt>Verweist auf</dt>
              <dd className={row.broken ? "hc-error" : undefined}>
                {row.linkTarget}
                {row.broken && " — fehlt"}
              </dd>
            </div>
          )}
        </dl>
      )}

      <dl className="hc-facts hc-meta">
        <div>
          <dt>Rang</dt>
          <dd className="hc-num">{formatScore(hit)}</dd>
        </div>
        <div>
          <dt>Tier</dt>
          <dd>{hit.tier}</dd>
        </div>
        <div>
          <dt>Salienz</dt>
          <dd className="hc-num">×{hit.score.salience_boost.toFixed(2)}</dd>
        </div>
      </dl>

      {error && <p className="hc-error">{error}</p>}
    </aside>
  );
}

function Section({
  title,
  count,
  empty,
  children,
}: {
  title: string;
  count: number;
  empty: string;
  children: React.ReactNode;
}) {
  return (
    <section className="hc-section">
      <h3 className="hc-subtitle">
        {title} <span className="hc-faint hc-num">{count}</span>
      </h3>
      {count === 0 ? <p className="hc-faint">{empty}</p> : children}
    </section>
  );
}

/**
 * Which arms found this. Rendered rather than computed: the numbers come from the daemon's own
 * score breakdown, which is the same one `cortex explain` prints.
 */
function formatScore(hit: SearchHit): string {
  const arms: string[] = [];
  if (hit.score.path_rank !== null) arms.push(`Pfad ${hit.score.path_rank}`);
  if (hit.score.lexical_rank !== null) arms.push(`Wort ${hit.score.lexical_rank}`);
  if (hit.score.vector_rank !== null) arms.push(`Sinn ${hit.score.vector_rank}`);
  return arms.length === 0 ? "—" : arms.join(" · ");
}
