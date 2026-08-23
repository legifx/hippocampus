/**
 * Playing a consolidation pass back, so that a person watching can describe what happened.
 *
 * That sentence is Definition-of-Done item 1 for M8, and it rules out the obvious
 * implementation — a progress bar with a summary at the end. What a pass actually does has
 * five distinct steps, each of which changes the picture in a different way, and the animation
 * shows them one at a time in the order the daemon performed them:
 *
 * 1. **Replay** — every episode in the window lights up where it happened.
 * 2. **Discard** — the bursts too large to be a person are named and dropped.
 * 3. **Associate** — the edges that survived are drawn in.
 * 4. **Score** — every node, until now the same size, grows to the salience it earned.
 * 5. **Tier** — the layout becomes shells and the demoted nodes visibly move outward.
 *
 * The numbers in the captions come from the report the daemon produced. Nothing here computes
 * a statistic of its own: if the animation and the report ever disagreed, the animation would
 * be a fiction, and the whole point of this view is that it is not one.
 */

import type { BrainScene } from "./scene";
import type { ConsolidationReport, GraphSnapshot } from "./snapshot";
import type { I18n, Key } from "./i18n";

export interface Caption {
  step: number;
  steps: number;
  title: string;
  detail: string;
  /** Set on the final step so the interface can show the whole report. */
  report?: ConsolidationReport;
}

interface Phase {
  key: Key;
  seconds: number;
  detail: (report: ConsolidationReport, i18n: I18n) => string;
  enter?: () => void;
  tick?: (t: number) => void;
  leave?: () => void;
}

export class ConsolidationPlayback {
  private phases: Phase[] = [];
  private index = -1;
  private clock = 0;
  private running = false;
  private eventCursor = 0;

  constructor(
    private scene: BrainScene,
    private snapshot: GraphSnapshot,
    private i18n: I18n,
    private onCaption: (caption: Caption | null) => void,
  ) {}

  get isRunning(): boolean {
    return this.running;
  }

  /** True when there is a pass to play at all. */
  get available(): boolean {
    return this.snapshot.report !== null;
  }

  start(): void {
    const report = this.snapshot.report;
    if (!report) return;
    this.stop();
    this.running = true;
    this.index = -1;
    this.clock = 0;
    this.eventCursor = 0;
    this.phases = this.build(report);
    this.advance();
  }

  stop(): void {
    if (this.index >= 0 && this.index < this.phases.length) {
      this.phases[this.index].leave?.();
    }
    this.running = false;
    this.index = -1;
    this.scene.resetScales();
    this.scene.setUniformity(0);
    this.scene.commitScales();
    this.scene.setRevealForType("co_access", 1);
    this.scene.setRevealForType("similar_to", 1);
    this.scene.commitEdges();
    this.onCaption(null);
  }

  update(dt: number): void {
    if (!this.running || this.index < 0) return;
    const phase = this.phases[this.index];
    this.clock += dt;
    const t = phase.seconds <= 0 ? 1 : Math.min(1, this.clock / phase.seconds);
    phase.tick?.(t);
    if (this.clock >= phase.seconds) this.advance();
  }

  private advance(): void {
    if (this.index >= 0) this.phases[this.index].leave?.();
    this.index++;
    this.clock = 0;
    if (this.index >= this.phases.length) {
      this.running = false;
      return;
    }
    const phase = this.phases[this.index];
    phase.enter?.();
    const report = this.snapshot.report!;
    this.onCaption({
      step: this.index + 1,
      steps: this.phases.length,
      title: this.i18n.t(phase.key),
      detail: phase.detail(report, this.i18n),
      report: this.index === this.phases.length - 1 ? report : undefined,
    });
  }

  private build(report: ConsolidationReport): Phase[] {
    const events = this.snapshot.events;
    const moved = report.movements.reduce((sum, m) => sum + m.count, 0);

    return [
      {
        key: "phase.replay",
        seconds: 4.5,
        detail: (r, t) =>
          t.t("phase.replay.detail", { episodes: t.n(r.episodes_replayed), bursts: t.n(r.bursts) }),
        enter: () => {
          this.eventCursor = 0;
          this.scene.setLayout("tree");
          this.scene.setUniformity(1);
          this.scene.commitScales();
          this.scene.setRevealForType("co_access", 0);
          this.scene.commitEdges();
        },
        tick: (t) => {
          // The events are replayed in their own order at a rate set by the phase, so a busy
          // afternoon looks busy. A fixed number per frame would flatten exactly the rhythm
          // that makes a burst visible as a burst.
          const upTo = Math.floor(events.length * t);
          while (this.eventCursor < upTo) {
            const event = events[this.eventCursor++];
            if (event.node !== null) this.scene.pulse(event.node, 1.4);
          }
        },
      },
      {
        key: "phase.discard",
        seconds: 2.0,
        detail: (r, t) => t.t("phase.discard.detail", { discarded: t.n(r.bursts_discarded) }),
      },
      {
        key: "phase.associate",
        seconds: 3.2,
        detail: (r, t) => t.t("phase.associate.detail", { edges: t.n(r.edges_written) }),
        tick: (t) => {
          this.scene.revealFraction("co_access", t);
          this.scene.commitEdges();
        },
        leave: () => {
          this.scene.setRevealForType("co_access", 1);
          this.scene.commitEdges();
        },
      },
      {
        key: "phase.score",
        seconds: 3.0,
        detail: (r, t) => t.t("phase.score.detail", { nodes: t.n(r.nodes_scored) }),
        tick: (t) => {
          this.scene.setUniformity(1 - t);
          this.scene.commitScales();
        },
        leave: () => {
          this.scene.setUniformity(0);
          this.scene.commitScales();
        },
      },
      {
        key: "phase.tier",
        seconds: 3.6,
        detail: (r, t) =>
          t.t("phase.tier.detail", { moved: t.n(moved), pinned: t.n(r.pinned_held) }),
        enter: () => this.scene.setLayout("shells"),
      },
      {
        key: "phase.done",
        seconds: 0.1,
        detail: (r, t) => t.t("phase.done.detail", { ms: t.n(r.duration_ms) }),
      },
    ];
  }
}
