/**
 * Interface strings. German by default, per the project's language rule: code and documentation
 * in English, anything a person reads on screen internationalised from the start.
 *
 * Flat keys and no interpolation library — the substitutions here are numbers into sentences,
 * and a dependency for that would be larger than the feature.
 */

export type Locale = "de" | "en";

const de = {
  "app.title": "Agent-Ansicht",
  "app.subtitle": "Was die Agents sehen",
  "origin.live": "live",
  "origin.baked": "Aufnahme",
  "origin.generated": "erzeugt",
  "layout.tree": "Baum",
  "layout.shells": "Schalen",
  "layout.label": "Anordnung",
  "action.consolidate": "Konsolidierung abspielen",
  "action.stop": "Abbrechen",
  "action.reset": "Ansicht zurücksetzen",
  "action.bloom": "Leuchten",
  "action.labels": "Beschriftung",
  "action.language": "English",
  "stats.nodes": "Knoten",
  "stats.edges": "Assoziationen",
  "stats.containment": "Enthält-Kanten",
  "stats.episodes": "Episoden",
  "stats.shown": "gezeigt",
  "stats.lastConsolidation": "letzte Konsolidierung",
  "stats.never": "nie gelaufen",
  "tier.working": "Arbeitsspeicher",
  "tier.stm": "Kurzzeit",
  "tier.ltm": "Langzeit",
  "tier.cold": "Kalt",
  "legend.tiers": "Ebenen",
  "legend.zones": "Zonen",
  "legend.edges": "Kanten",
  "legend.concepts": "Begriffe",
  "legend.encoding": "Größe = Salienz · Farbe = Zone · Helligkeit = Ebene · hell = Begriff",
  "edge.contains": "enthält",
  "edge.co_access": "gemeinsam benutzt",
  "edge.similar_to": "ähnlich",
  "edge.annotates": "annotiert",
  "edge.imports": "importiert",
  "edge.derived_from": "fasst zusammen",
  "node.salience": "Salienz",
  "node.tier": "Ebene",
  "node.degree": "Verbindungen",
  "node.touches": "Zugriffe (7 Tage)",
  "node.pinned": "angeheftet",
  "node.size": "Größe",
  "node.modified": "geändert",
  "node.neighbors": "Nachbarn",
  "node.none": "Nichts ausgewählt",
  "node.hint": "Einen Knoten anklicken",
  "phase.replay": "Wiedergabe",
  "phase.replay.detail": "{episodes} Episoden, in {bursts} Handlungen gruppiert",
  "phase.discard": "Aussortieren",
  "phase.discard.detail": "{discarded} Handlungen als Maschinenaktivität verworfen",
  "phase.associate": "Verknüpfen",
  "phase.associate.detail": "{edges} Kanten aus gemeinsamer Benutzung",
  "phase.score": "Bewerten",
  "phase.score.detail": "{nodes} Knoten neu bewertet",
  "phase.tier": "Einordnen",
  "phase.tier.detail": "{moved} Knoten wechseln die Ebene, {pinned} angeheftete bleiben",
  "phase.done": "Bericht",
  "phase.done.detail": "in {ms} ms",
  "report.notDone": "Bewusst nicht getan",
  "report.top": "Am saliensten",
  "report.demoted": "Eine Ebene vergessen",
  "basis.label": "Grundlage",
  "error.title": "Keine Daten",
  "error.hint": "cortexd läuft nicht, oder graph.json fehlt.",
  "fps": "Bilder/s",
};

const en: Record<keyof typeof de, string> = {
  "app.title": "Agent View",
  "app.subtitle": "What the agents see",
  "origin.live": "live",
  "origin.baked": "snapshot",
  "origin.generated": "generated",
  "layout.tree": "Tree",
  "layout.shells": "Shells",
  "layout.label": "Layout",
  "action.consolidate": "Play consolidation",
  "action.stop": "Stop",
  "action.reset": "Reset view",
  "action.bloom": "Glow",
  "action.labels": "Labels",
  "action.language": "Deutsch",
  "stats.nodes": "nodes",
  "stats.edges": "associations",
  "stats.containment": "containment",
  "stats.episodes": "episodes",
  "stats.shown": "shown",
  "stats.lastConsolidation": "last consolidation",
  "stats.never": "never ran",
  "tier.working": "Working",
  "tier.stm": "Short-term",
  "tier.ltm": "Long-term",
  "tier.cold": "Cold",
  "legend.tiers": "Tiers",
  "legend.zones": "Zones",
  "legend.edges": "Edges",
  "legend.concepts": "Concepts",
  "legend.encoding": "size = salience · hue = zone · brightness = tier · pale = concept",
  "edge.contains": "contains",
  "edge.co_access": "used together",
  "edge.similar_to": "similar",
  "edge.annotates": "annotates",
  "edge.imports": "imports",
  "edge.derived_from": "abstracts",
  "node.salience": "Salience",
  "node.tier": "Tier",
  "node.degree": "Connections",
  "node.touches": "Touches (7 days)",
  "node.pinned": "pinned",
  "node.size": "Size",
  "node.modified": "modified",
  "node.neighbors": "Neighbours",
  "node.none": "Nothing selected",
  "node.hint": "Click a node",
  "phase.replay": "Replay",
  "phase.replay.detail": "{episodes} episodes, grouped into {bursts} acts",
  "phase.discard": "Discard",
  "phase.discard.detail": "{discarded} acts thrown away as machine activity",
  "phase.associate": "Associate",
  "phase.associate.detail": "{edges} edges from co-access",
  "phase.score": "Score",
  "phase.score.detail": "{nodes} nodes rescored",
  "phase.tier": "Tier",
  "phase.tier.detail": "{moved} nodes change tier, {pinned} pinned nodes held",
  "phase.done": "Report",
  "phase.done.detail": "in {ms} ms",
  "report.notDone": "Deliberately not done",
  "report.top": "Most salient",
  "report.demoted": "Forgotten a tier",
  "basis.label": "Basis",
  "error.title": "No data",
  "error.hint": "cortexd is not running, or graph.json is missing.",
  "fps": "fps",
};

const TABLES: Record<Locale, Record<string, string>> = { de, en };

export type Key = keyof typeof de;

export class I18n {
  constructor(public locale: Locale = "de") {}

  t(key: Key, vars?: Record<string, string | number>): string {
    const table = TABLES[this.locale] ?? de;
    let text = table[key] ?? de[key] ?? String(key);
    if (vars) {
      for (const [name, value] of Object.entries(vars)) {
        text = text.replaceAll(`{${name}}`, String(value));
      }
    }
    return text;
  }

  toggle(): Locale {
    this.locale = this.locale === "de" ? "en" : "de";
    return this.locale;
  }

  /** Numbers grouped the way the current locale groups them. */
  n(value: number): string {
    return value.toLocaleString(this.locale === "de" ? "de-DE" : "en-GB");
  }

  date(unixSeconds: number): string {
    if (!unixSeconds) return "—";
    return new Date(unixSeconds * 1000).toLocaleString(
      this.locale === "de" ? "de-DE" : "en-GB",
      { dateStyle: "medium", timeStyle: "short" },
    );
  }

  bytes(value: number): string {
    const units = ["B", "kB", "MB", "GB", "TB"];
    let v = value;
    let unit = 0;
    while (v >= 1024 && unit < units.length - 1) {
      v /= 1024;
      unit++;
    }
    return `${unit === 0 ? v : v.toFixed(1)} ${units[unit]}`;
  }
}
