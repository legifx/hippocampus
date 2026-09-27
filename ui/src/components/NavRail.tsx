/**
 * The rail: where you are, what else there is, and what the daemon is doing.
 *
 * The window used to have exactly one screen and no way to say so — the places list was reached
 * through a sentinel path and the agent panel through a keystroke, and neither was visible until
 * you already knew it existed. A rail answers "where am I" and "what else is there" at a glance,
 * which is the whole of what was missing.
 *
 * Two destinations and one panel, all of them states the window already had:
 *
 * - **Übersicht** — the start screen: what is in the index, and how the agents have been using it.
 * - **Dateien** — the browser, which is what this window did before, unchanged. The places list is
 *   a path inside it rather than a third destination, because it always was one and a rail that
 *   said otherwise would be a second navigation model beside `useView` (M10).
 * - **Gehirn** — the brain as the agents use it, live: 3D view, heatmap, live log, the curator.
 * - **Inspector** — the Agent Inspector, which used to be behind a keystroke.
 *
 * The counts beside them come from the same statistics call the start screen uses. They are
 * printed, not derived: the daemon counted, the window repeats the number.
 *
 * The glyphs are inline paths rather than a font or a package: four of them, each one shape, and
 * a rail whose icons come from somewhere else is a rail that breaks when that somewhere does.
 */

import { motion } from "framer-motion";
import { formatSize } from "../format";
import { kitSpring } from "../motion";
import type { OverviewDaemon, OverviewIndex } from "../ipc";

export type Section = "overview" | "files" | "brain";

/**
 * How long the daemon has been up, in the shortest form that is still honest.
 *
 * The same idea as the CLI's `format_duration`, and deliberately not shared with it: one is a
 * terminal printer and one is a rail footer, and the second one has no room for `1d 4h 12m`.
 */
function uptime(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)} s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ${minutes % 60} min`;
  return `${Math.floor(hours / 24)} d ${hours % 24} h`;
}

export function Glyph({ name }: { name: "grid" | "folder" | "spark" | "brain" }) {
  const paths: Record<string, string> = {
    // Four panes: a dashboard, not a list.
    grid: "M2.5 2.5h4.4v4.4H2.5zM9.1 2.5h4.4v4.4H9.1zM2.5 9.1h4.4v4.4H2.5zM9.1 9.1h4.4v4.4H9.1z",
    folder: "M2.2 4.3h4l1.2 1.5h6.4v6.4a1 1 0 0 1-1 1H3.2a1 1 0 0 1-1-1z",
    // A spark: the agent panel. Four points rather than a robot, because a robot icon is a claim
    // about what the thing is that this project deliberately does not make.
    spark: "M8 2.2v3M8 10.8v3M2.2 8h3M10.8 8h3M4.6 4.6l1.6 1.6M9.8 9.8l1.6 1.6M11.4 4.6l-1.6 1.6M6.2 9.8l-1.6 1.6",
    // The mark: a hippocampus is a thing that remembers, and a ring with a fold is as close as a
    // 16 px path gets to saying so.
    brain: "M8 2.6c-3 0-5.2 1.9-5.2 4.6 0 1.5.8 2.6 2 3.3v2.9l2-1.4c.4.1.8.1 1.2.1 3 0 5.2-1.9 5.2-4.5S11 2.6 8 2.6z",
  };

  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}

export function NavRail({
  section,
  onSection,
  index,
  daemon,
  agentOpen,
  onAgents,
}: {
  section: Section;
  onSection: (section: Section) => void;
  index: OverviewIndex | null;
  daemon: OverviewDaemon | null;
  agentOpen: boolean;
  onAgents: () => void;
}) {
  return (
    <nav className="hc-rail glass" aria-label="Bereiche">
      <div className="hc-mark">
        <span className="hc-mark-glyph">
          <Glyph name="brain" />
        </span>
        <span className="hc-mark-name">Hippocampus</span>
      </div>

      <div className="hc-rail-group">Index</div>

      {/* The destination you are on wears the kit's raise — `.raised-flat`, because §8.2 takes the
          drop out of the kit's `raised` for a row — and one that is merely under the pointer gets the
          kit's nav hover, which is `accent-soft` and not the `selection` its list rows use. Both are
          materials: the recipe lives in `src/glass.css` and the fills in `shell.css`, and the same
          raise is what the places column puts on the place you are in. */}
      <button
        type="button"
        className="hc-nav"
        aria-current={section === "overview" ? "page" : undefined}
        onClick={() => onSection("overview")}
      >
        {section === "overview" && <motion.span layoutId="hc-nav-pill" className="hc-nav-pill" transition={kitSpring("spring")} />}
        <span className="hc-nav-glyph">
          <Glyph name="grid" />
        </span>
        <span className="hc-nav-label">Übersicht</span>
      </button>

      <button
        type="button"
        className="hc-nav"
        aria-current={section === "files" ? "page" : undefined}
        onClick={() => onSection("files")}
      >
        {section === "files" && <motion.span layoutId="hc-nav-pill" className="hc-nav-pill" transition={kitSpring("spring")} />}
        <span className="hc-nav-glyph">
          <Glyph name="folder" />
        </span>
        <span className="hc-nav-label">Dateien</span>
        {index && <span className="hc-nav-count">{index.files.toLocaleString("de-DE")}</span>}
      </button>

      <div className="hc-rail-group">Agenten</div>

      {/* The brain as the agents use it, live: a destination of its own, because it is a whole
          screen — the 3D brain, the heatmap, the live log and the curator — not a panel. */}
      <button
        type="button"
        className="hc-nav"
        aria-current={section === "brain" ? "page" : undefined}
        aria-label="Gehirn"
        title="Gehirn — Agenten live (Mod+B)"
        onClick={() => onSection("brain")}
      >
        {section === "brain" && <motion.span layoutId="hc-nav-pill" className="hc-nav-pill" transition={kitSpring("spring")} />}
        <span className="hc-nav-glyph">
          <Glyph name="brain" />
        </span>
        <span className="hc-nav-label">Gehirn</span>
      </button>

      <button
        type="button"
        className={agentOpen ? "hc-nav raised-flat" : "hc-nav"}
        aria-current={agentOpen ? "page" : undefined}
        onClick={onAgents}
      >
        <span className="hc-nav-glyph">
          <Glyph name="spark" />
        </span>
        <span className="hc-nav-label">Inspector</span>
        {index && (
          <span className="hc-nav-count">{index.episodes.toLocaleString("de-DE")}</span>
        )}
      </button>

      <div className="hc-rail-foot">
        {daemon ? (
          <>
            <div className="hc-rail-foot-line">
              <span className="hc-rail-foot-key">cortexd</span>
              <span className="hc-rail-foot-value">
                {daemon.version} · {uptime(daemon.uptime_secs)}
              </span>
            </div>
            <div className="hc-chip-row">
              <span
                className="hc-chip"
                data-tone={daemon.agent_surface_enabled ? "on" : "off"}
                // The dot alone has to carry this in a 58 px rail, so the meaning is not only in the
                // word: it is the chip's accessible name and its tooltip either way.
                title={daemon.agent_surface_enabled ? "Agenten an" : "Agenten aus"}
                aria-label={daemon.agent_surface_enabled ? "Agenten an" : "Agenten aus"}
              >
                <span className="hc-chip-dot" />
                <span className="hc-chip-label">
                  {daemon.agent_surface_enabled ? "Agenten an" : "Agenten aus"}
                </span>
              </span>
              {daemon.indexing_paused && (
                <span className="hc-chip" data-tone="warn">
                  <span className="hc-chip-dot" />
                  pausiert
                </span>
              )}
            </div>
            <div className="hc-rail-foot-line">
              <span className="hc-rail-foot-key">Pfadindex</span>
              <span className="hc-rail-foot-value">
                {daemon.indexed_paths.toLocaleString("de-DE")} · {formatSize(daemon.index_memory_bytes)}
              </span>
            </div>
            {daemon.write_allowlist.length > 0 && (
              <div className="hc-rail-foot-line">
                <span className="hc-rail-foot-key">schreiben</span>
                <span className="hc-rail-foot-value" title={daemon.write_allowlist.join(", ")}>
                  {daemon.write_allowlist.join(", ")}
                </span>
              </div>
            )}
          </>
        ) : (
          <div className="hc-rail-foot-line">
            <span className="hc-rail-foot-value">kein Stand</span>
          </div>
        )}
      </div>
    </nav>
  );
}
