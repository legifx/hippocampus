/**
 * Two files side by side, for the question the owner's own example asks:
 * "ist jetzt content.png oder content(1).png das richtige?"
 *
 * Select two rows — Ctrl+click, or Shift with an arrow key — and the preview slot shows both,
 * with every fact that can tell them apart lined up in the same order on both sides, and the
 * facts that *differ* marked. Reading two directory strings on two rows and holding one in
 * mind while looking at the other is what this replaces; for two pictures with nearly the same
 * name it is the difference between looking and guessing.
 *
 * This is a view over two `SearchHit`s and two `Row`s, nothing more: no diff is computed, no
 * bytes are compared. The daemon has no file-diff tool (the third review of 2026-09-05 was
 * right that `brain.diff` is an episode feed, not a comparison), and a pixel diff of two
 * screenshots is a feature nobody has asked for yet. What people asked for is to *see* both.
 */

import type { SearchHit } from "../ipc";
import type { Row } from "../row";
import { formatAge, formatSize, splitPath, tierLabel } from "../format";
import { usePreview } from "../hooks/usePreview";
import { PreviewBody } from "./PreviewPane";

export type Compared = { hit: SearchHit; row: Row };

type Fact = { label: string; left: string; right: string };

/** The facts, in the order a person checks them: where, how big, how old, then the rest. */
function facts(a: Compared, b: Compared): Fact[] {
  const dir = (c: Compared) => splitPath(c.hit.path).dir || "/";
  const date = (c: Compared) =>
    `${formatAge(c.hit.mtime)} · ${new Date(c.hit.mtime * 1000).toLocaleString("de-DE")}`;
  const out: Fact[] = [
    { label: "Ordner", left: dir(a), right: dir(b) },
    {
      label: "Größe",
      left: a.hit.kind === "dir" ? "—" : `${formatSize(a.hit.size)} (${a.hit.size.toLocaleString("de-DE")} B)`,
      right: b.hit.kind === "dir" ? "—" : `${formatSize(b.hit.size)} (${b.hit.size.toLocaleString("de-DE")} B)`,
    },
    { label: "Geändert", left: date(a), right: date(b) },
  ];
  if (a.row.mode || b.row.mode) {
    out.push({ label: "Rechte", left: a.row.mode ?? "—", right: b.row.mode ?? "—" });
  }
  if (a.row.owner || b.row.owner) {
    out.push({ label: "Besitzer", left: a.row.owner ?? "—", right: b.row.owner ?? "—" });
  }
  if (a.row.tier || b.row.tier) {
    out.push({
      label: "Ebene",
      left: (a.row.tier && tierLabel(a.row.tier)) || "—",
      right: (b.row.tier && tierLabel(b.row.tier)) || "—",
    });
  }
  if (a.row.linkTarget || b.row.linkTarget) {
    out.push({ label: "Verweis auf", left: a.row.linkTarget ?? "—", right: b.row.linkTarget ?? "—" });
  }
  return out;
}

export function ComparePane({ left, right }: { left: Compared; right: Compared }) {
  const leftState = usePreview(left.hit);
  const rightState = usePreview(right.hit);
  const rows = facts(left, right);
  let differing = 0;
  for (const fact of rows) {
    if (fact.left !== fact.right) differing += 1;
  }

  return (
    <section className="hc-preview hc-compare glass-flat" aria-label="Vergleich">
      <header className="hc-preview-head">
        <span className="hc-title">Vergleich</span>
        <span className="hc-meta">
          {differing === 0
            ? "kein Unterschied in den Angaben"
            : `${differing} ${differing === 1 ? "Unterschied" : "Unterschiede"}`}
        </span>
      </header>
      <div className="hc-compare-names">
        <span className="hc-title hc-compare-name" title={left.hit.path}>
          {splitPath(left.hit.path).name}
        </span>
        <span className="hc-title hc-compare-name" title={right.hit.path}>
          {splitPath(right.hit.path).name}
        </span>
      </div>
      <dl className="hc-compare-facts">
        {rows.map((fact) => {
          const differs = fact.left !== fact.right;
          return (
            <div
              key={fact.label}
              className={differs ? "hc-compare-fact hc-compare-differs" : "hc-compare-fact"}
              data-differs={differs ? "yes" : "no"}
            >
              <dt className="hc-meta">{fact.label}</dt>
              <dd className="hc-num">{fact.left}</dd>
              <dd className="hc-num">{fact.right}</dd>
            </div>
          );
        })}
      </dl>
      <div className="hc-compare-bodies">
        <div className="hc-preview-body" data-side="left">
          <PreviewBody state={leftState} />
        </div>
        <div className="hc-preview-body" data-side="right">
          <PreviewBody state={rightState} />
        </div>
      </div>
    </section>
  );
}
