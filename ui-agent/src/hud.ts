/**
 * Everything on top of the canvas.
 *
 * Plain DOM rather than a framework: this is one screen with eight panels, and its state is a
 * selected node plus a caption. A component library would be the largest dependency in the
 * project and would buy nothing.
 *
 * The panels have one job between them — making the picture legible. A three-dimensional graph
 * without a legend is decoration, and the two questions a viewer asks first are "what does the
 * size mean" and "am I looking at real data". Both are answered before anything is clicked.
 */

import { EDGE_STYLE, TIER_STYLE, css, hslToRgb, nodeColor } from "./palette";
import type { I18n, Key } from "./i18n";
import type { Caption } from "./consolidation";
import { TIERS, type GraphNode, type GraphSnapshot, type Origin } from "./snapshot";

export interface HudCallbacks {
  onLayout: (mode: "tree" | "shells") => void;
  onConsolidate: () => void;
  onReset: () => void;
  onBloom: (enabled: boolean) => void;
  onLanguage: () => void;
  onFocus: (nodeId: number) => void;
}

function element<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

export class Hud {
  private root: HTMLElement;
  private statsBody = element("dl", "stats");
  private legendBody = element("div", "legend-body");
  private inspectorBody = element("div", "inspector-body");
  private captionBox = element("div", "caption hidden");
  private reportBox = element("div", "report hidden");
  private basisText = element("p", "basis");
  private fpsText = element("span", "fps");
  private buttons: Record<string, HTMLButtonElement> = {};
  private selected: GraphNode | null = null;
  private neighbourCounts = new Map<number, number>();

  constructor(
    container: HTMLElement,
    private snapshot: GraphSnapshot,
    private origin: Origin,
    private i18n: I18n,
    private callbacks: HudCallbacks,
    private shownEdges: number,
  ) {
    for (const edge of snapshot.edges) {
      this.neighbourCounts.set(edge.src, (this.neighbourCounts.get(edge.src) ?? 0) + 1);
      this.neighbourCounts.set(edge.dst, (this.neighbourCounts.get(edge.dst) ?? 0) + 1);
    }

    this.root = element("div", "hud");
    container.appendChild(this.root);

    const topLeft = element("div", "panel top-left");
    const heading = element("div", "heading");
    heading.appendChild(element("h1", undefined, ""));
    heading.appendChild(element("p", "subtitle", ""));
    const badge = element("span", `badge badge-${origin}`);
    heading.appendChild(badge);
    topLeft.appendChild(heading);
    topLeft.appendChild(this.statsBody);
    topLeft.appendChild(this.basisText);
    this.root.appendChild(topLeft);

    const bottomLeft = element("div", "panel bottom-left");
    bottomLeft.appendChild(element("h2", undefined, ""));
    bottomLeft.appendChild(this.legendBody);
    this.root.appendChild(bottomLeft);

    const topRight = element("div", "panel top-right");
    topRight.appendChild(element("h2", undefined, ""));
    topRight.appendChild(this.inspectorBody);
    this.root.appendChild(topRight);

    const controls = element("div", "controls");
    const make = (name: string, key: Key, handler: () => void): HTMLButtonElement => {
      const button = element("button", undefined, this.i18n.t(key));
      button.dataset.key = key;
      button.addEventListener("click", handler);
      this.buttons[name] = button;
      controls.appendChild(button);
      return button;
    };
    make("layout", "layout.shells", () => {
      const next = this.buttons.layout.dataset.mode === "shells" ? "tree" : "shells";
      this.callbacks.onLayout(next);
    });
    this.buttons.layout.dataset.mode = "tree";
    make("consolidate", "action.consolidate", () => this.callbacks.onConsolidate());
    make("reset", "action.reset", () => this.callbacks.onReset());
    const bloom = make("bloom", "action.bloom", () => {
      const enabled = this.buttons.bloom.classList.toggle("off");
      this.callbacks.onBloom(!enabled);
    });
    bloom.title = "bloom";
    make("language", "action.language", () => this.callbacks.onLanguage());
    controls.appendChild(this.fpsText);
    this.root.appendChild(controls);

    this.root.appendChild(this.captionBox);
    this.root.appendChild(this.reportBox);

    this.retranslate();
  }

  /** Redraw every string. Called on load and whenever the language is toggled. */
  retranslate(): void {
    const t = (key: Key, vars?: Record<string, string | number>) => this.i18n.t(key, vars);
    (this.root.querySelector(".top-left h1") as HTMLElement).textContent = t("app.title");
    (this.root.querySelector(".top-left .subtitle") as HTMLElement).textContent = t("app.subtitle");
    (this.root.querySelector(".badge") as HTMLElement).textContent = t(
      `origin.${this.origin}` as Key,
    );
    (this.root.querySelector(".bottom-left h2") as HTMLElement).textContent = t("legend.encoding");
    (this.root.querySelector(".top-right h2") as HTMLElement).textContent = t("node.neighbors");

    this.buttons.layout.textContent =
      this.buttons.layout.dataset.mode === "shells" ? t("layout.tree") : t("layout.shells");
    this.buttons.consolidate.textContent = t("action.consolidate");
    this.buttons.reset.textContent = t("action.reset");
    this.buttons.bloom.textContent = t("action.bloom");
    this.buttons.language.textContent = t("action.language");

    this.renderStats();
    this.renderLegend();
    this.renderInspector();
    this.basisText.textContent = `${t("basis.label")}: ${this.snapshot.basis}`;
  }

  private renderStats(): void {
    const t = (key: Key) => this.i18n.t(key);
    const n = (value: number) => this.i18n.n(value);
    this.statsBody.replaceChildren();
    // Two different things were being compared here in the first version: the drawn edges
    // include containment, which is derived from parent links and is not in the edge table at
    // all, so "3.119 shown / 120" read as a bug in the daemon. They are counted apart now.
    const contains = this.snapshot.edges.filter((e) => e.type === "contains").length;
    const rows: Array<[string, string]> = [
      [t("stats.nodes"), `${n(this.snapshot.nodes.length)} ${t("stats.shown")} / ${n(this.snapshot.totals.nodes)}`],
      [t("stats.containment"), n(contains)],
      [t("stats.edges"), `${n(this.shownEdges - contains)} ${t("stats.shown")} / ${n(this.snapshot.totals.edges)}`],
      [t("stats.episodes"), n(this.snapshot.totals.episodes)],
      [
        t("stats.lastConsolidation"),
        this.snapshot.totals.last_consolidation
          ? this.i18n.date(this.snapshot.totals.last_consolidation)
          : t("stats.never"),
      ],
    ];
    for (const [label, value] of rows) {
      this.statsBody.appendChild(element("dt", undefined, label));
      this.statsBody.appendChild(element("dd", undefined, value));
    }
  }

  private renderLegend(): void {
    const t = (key: Key) => this.i18n.t(key);
    this.legendBody.replaceChildren();

    const tiers = element("div", "legend-group");
    tiers.appendChild(element("h3", undefined, t("legend.tiers")));
    for (const tier of TIERS) {
      const row = element("div", "legend-row");
      const swatch = element("span", "swatch");
      const style = TIER_STYLE[tier];
      swatch.style.background = css(hslToRgb(210, style.saturation, style.lightness));
      row.appendChild(swatch);
      const count = this.snapshot.totals.tiers[tier] ?? 0;
      row.appendChild(
        element("span", undefined, `${t(`tier.${tier}` as Key)} · ${this.i18n.n(count)}`),
      );
      tiers.appendChild(row);
    }
    this.legendBody.appendChild(tiers);

    const zones = element("div", "legend-group");
    zones.appendChild(element("h3", undefined, t("legend.zones")));
    const shown = [...this.snapshot.zones].sort((a, b) => b.nodes - a.nodes).slice(0, 8);
    for (const zone of shown) {
      const row = element("div", "legend-row");
      const swatch = element("span", "swatch");
      swatch.style.background = css(hslToRgb(zone.hue, 0.75, 0.55));
      row.appendChild(swatch);
      row.appendChild(element("span", undefined, `${zone.name} · ${this.i18n.n(zone.nodes)}`));
      zones.appendChild(row);
    }
    if (shown.length > 0) this.legendBody.appendChild(zones);

    const edges = element("div", "legend-group");
    edges.appendChild(element("h3", undefined, t("legend.edges")));
    const present = [...new Set(this.snapshot.edges.map((e) => e.type))];
    for (const type of present) {
      const style = EDGE_STYLE[type];
      if (!style) continue;
      const row = element("div", "legend-row");
      const swatch = element("span", "swatch line");
      swatch.style.background = css(style.color);
      row.appendChild(swatch);
      row.appendChild(element("span", undefined, t(`edge.${type}` as Key)));
      edges.appendChild(row);
    }
    this.legendBody.appendChild(edges);
  }

  setSelected(node: GraphNode | null): void {
    this.selected = node;
    this.renderInspector();
  }

  private renderInspector(): void {
    const t = (key: Key) => this.i18n.t(key);
    this.inspectorBody.replaceChildren();
    if (!this.selected) {
      this.inspectorBody.appendChild(element("p", "muted", t("node.none")));
      this.inspectorBody.appendChild(element("p", "muted small", t("node.hint")));
      return;
    }
    const node = this.selected;
    const zone = this.snapshot.zones.find((z) => z.id === node.zone);
    const title = element("p", "path", node.path);
    title.style.color = css(nodeColor(zone?.hue, node.tier, node.kind), 1);
    this.inspectorBody.appendChild(title);

    const list = element("dl", "stats");
    const rows: Array<[string, string]> = [
      [t("node.salience"), node.salience.toFixed(3)],
      [t("node.tier"), t(`tier.${node.tier}` as Key)],
      [t("node.degree"), this.i18n.n(this.neighbourCounts.get(node.id) ?? 0)],
      [t("node.touches"), this.i18n.n(node.recent_touches)],
      [t("node.size"), node.kind === "dir" ? "—" : this.i18n.bytes(node.size)],
      [t("node.modified"), this.i18n.date(node.mtime)],
    ];
    if (node.pinned) rows.push([t("node.pinned"), "✓"]);
    for (const [label, value] of rows) {
      list.appendChild(element("dt", undefined, label));
      list.appendChild(element("dd", undefined, value));
    }
    this.inspectorBody.appendChild(list);

    const focus = element("button", "link", "→");
    focus.addEventListener("click", () => this.callbacks.onFocus(node.id));
    this.inspectorBody.appendChild(focus);
  }

  setCaption(caption: Caption | null): void {
    if (!caption) {
      this.captionBox.classList.add("hidden");
      this.reportBox.classList.add("hidden");
      return;
    }
    this.captionBox.classList.remove("hidden");
    this.captionBox.replaceChildren();
    const step = element("span", "step", `${caption.step}/${caption.steps}`);
    this.captionBox.appendChild(step);
    this.captionBox.appendChild(element("strong", undefined, caption.title));
    this.captionBox.appendChild(element("span", "detail", caption.detail));

    if (caption.report) this.showReport(caption.report);
    else this.reportBox.classList.add("hidden");
  }

  private showReport(report: Caption["report"]): void {
    if (!report) return;
    const t = (key: Key) => this.i18n.t(key);
    this.reportBox.classList.remove("hidden");
    this.reportBox.replaceChildren();

    const close = element("button", "close", "×");
    close.addEventListener("click", () => this.reportBox.classList.add("hidden"));
    this.reportBox.appendChild(close);

    if (report.movements.length > 0) {
      const moves = element("div", "report-group");
      moves.appendChild(element("h3", undefined, t("phase.tier")));
      for (const move of report.movements) {
        moves.appendChild(
          element(
            "p",
            "move",
            `${t(`tier.${move.from}` as Key)} → ${t(`tier.${move.to}` as Key)}   ${this.i18n.n(move.count)}`,
          ),
        );
      }
      this.reportBox.appendChild(moves);
    }

    if (report.top.length > 0) {
      const top = element("div", "report-group");
      top.appendChild(element("h3", undefined, t("report.top")));
      for (const item of report.top.slice(0, 5)) {
        top.appendChild(
          element("p", "highlight", `${item.salience.toFixed(2)}  ${item.path}`),
        );
      }
      this.reportBox.appendChild(top);
    }

    if (report.not_done.length > 0) {
      const open = element("div", "report-group");
      open.appendChild(element("h3", undefined, t("report.notDone")));
      for (const item of report.not_done) open.appendChild(element("p", "muted small", `· ${item}`));
      this.reportBox.appendChild(open);
    }
  }

  setLayoutButton(mode: "tree" | "shells"): void {
    this.buttons.layout.dataset.mode = mode;
    this.buttons.layout.textContent =
      mode === "shells" ? this.i18n.t("layout.tree") : this.i18n.t("layout.shells");
  }

  setFps(fps: number): void {
    this.fpsText.textContent = `${Math.round(fps)} ${this.i18n.t("fps")}`;
    this.fpsText.classList.toggle("warn", fps < 30);
  }
}
