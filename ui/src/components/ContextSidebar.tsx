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
import { ipc, type NeighborsResult, type SearchHit } from "../ipc";
import { formatAge, splitPath } from "../format";
import type { Row } from "../row";
import { spring, stagger } from "../motion";

/** Everything the schema allows except `annotates`, which arrives separately. */
const RELATED_EDGES = ["contains", "co_access", "imports", "similar_to", "derived_from"];

export function ContextSidebar({ hit, row }: { hit: SearchHit | null; row?: Row | null }) {
  const [context, setContext] = useState<NeighborsResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!hit) {
      setContext(null);
      return;
    }

    // A selection can move faster than the daemon answers — an arrow key held down produces one
    // request per row — so a late answer for a row nobody is on any more is dropped.
    let current = true;
    setError(null);
    ipc
      // Every edge type *except* `annotates`. The notes come back in their own field of the
      // same answer, so asking the daemon to leave them out of the edge list is one argument —
      // and filtering them out here instead would be the frontend data processing that D4
      // forbids outright.
      .neighbors(hit.path, 1, RELATED_EDGES)
      .then((result) => current && setContext(result))
      .catch((e: unknown) => {
        if (!current) return;
        setContext(null);
        setError(e instanceof Error ? e.message : "the brain did not answer");
      });

    return () => {
      current = false;
    };
  }, [hit?.path]);

  if (!hit) {
    return (
      <aside className="hc-context" aria-label="Kontext">
        <p className="hc-faint">Nichts ausgewählt</p>
      </aside>
    );
  }

  const notes = context?.annotations ?? [];
  const related = context?.neighbors ?? [];

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

      <Section title="Verbunden" count={related.length} empty="nichts verknüpft">
        {related.map((neighbor, index) => (
          <motion.div
            key={neighbor.node_id}
            className="hc-related"
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ ...spring("content"), delay: stagger(index, true) }}
          >
            <span className="hc-pill">{neighbor.edge}</span>
            <span className="hc-related-path hc-meta">
              {neighbor.path ? splitPath(neighbor.path).name : `Knoten ${neighbor.node_id}`}
            </span>
          </motion.div>
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
