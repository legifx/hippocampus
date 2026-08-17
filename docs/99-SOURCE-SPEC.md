# Projekt-Spezifikation: Hippocampus File Explorer
### Ein Dateimanager für Linux/Niri, der gleichzeitig ein Second Brain für Menschen und Agents ist

> **Zweck dieses Dokuments:** Input für einen Build-Prompt-Agent. Es ist *kein* Build-Prompt.
> Es ordnet die Idee, legt Begriffe fest, trifft Vorentscheidungen und markiert explizit,
> was noch offen ist. Der Build-Prompt-Agent soll Abschnitt 16 (Offene Entscheidungen)
> beantwortet bekommen, bevor er den finalen Prompt schreibt.

---

## 1. Kernthese

Ein Dateisystem ist bereits ein Gedächtnis — nur ein extrem schlechtes. Es speichert Inhalte,
aber keine **Episoden** (wann, warum, in welchem Zusammenhang), keine **Salienz** (was ist wichtig)
und keine **Assoziationen** (was gehört wozu). Deshalb suchen Menschen Dateien, statt sie sich
zu *erinnern*.

Das Projekt dreht die Priorität um:

> **Der Dateimanager ist nicht die Anwendung. Der Dateimanager ist eine von mehreren Ansichten
> auf einen lokalen Gedächtnis-Daemon.**

Daraus folgt die wichtigste Architekturentscheidung des gesamten Projekts:

**Der Kern ist ein headless Daemon. GUI, CLI, TUI und MCP-Server sind gleichrangige Clients.**

Das ist nicht kosmetisch. Es entscheidet, ob das Projekt in 6 Monaten noch existiert:
- Der Daemon ist ohne GUI testbar (Integrationstests, Benchmarks, Fuzzing).
- Der Agent-Zugriff ist kein nachträglicher Aufsatz, sondern gleichberechtigter First-Class-Client.
- Wenn die GUI-Technologie sich als falsche Wahl herausstellt, wirft man 20 % weg statt 100 %.
- Die Server-Synchronisation spricht mit dem Daemon, nicht mit einer laufenden GUI — und der
  PC-Daemon kann laufen, auch wenn kein Fenster offen ist.

---

## 2. Verbindliches Glossar

Der Build-Prompt-Agent muss diese Begriffe exakt so verwenden. Uneinheitliche Begriffe sind
die häufigste Ursache für architektonische Drift in agentisch gebauten Systemen.

| Begriff | Bedeutung |
|---|---|
| **Cortex** | Der Daemon auf dem lokalen PC. Kern des Systems. |
| **Main Brain** | Die bestehende Second-Brain-Instanz auf dem Homeserver. Autoritative semantische Ebene. |
| **Node** | Ein Gedächtniseintrag. Kann eine Datei, ein Verzeichnis, ein Chunk, ein Konzept oder eine Annotation sein. |
| **Episode** | Ein zeitgestempeltes Ereignis (Datei geöffnet, erstellt, gesucht, verschoben). Unveränderlich, append-only. |
| **Working Set** | Was der Mensch *gerade jetzt* tut. Flüchtig, TTL-basiert. |
| **STM** | Short-Term Memory. Heiße, vollständig indizierte Zone der letzten N Tage. |
| **LTM** | Long-Term Memory. Konsolidiertes, verdichtetes Wissen. Lebt primär im Main Brain. |
| **Consolidation** | Der Batch-Prozess, der Episoden zu semantischem Wissen verdichtet und nach oben schiebt. |
| **Salience** | Wichtigkeits-Score eines Node. Steuert Konsolidierung, Ranking und Vergessen. |
| **Human View** | Die auf Menschen optimierte GUI-Ansicht. |
| **Agent View** | Die Visualisierung des Brain-Zustands. Für Menschen lesbar, aber sie *zeigt*, was Agents sehen. |
| **Agent Surface** | Die tatsächliche Maschinenschnittstelle (MCP + lokale API). Nicht zu verwechseln mit Agent View. |
| **Core** | Der öffentliche, veröffentlichbare Teil des Codes. |
| **Extension** | Privater, persönlicher Aufsatz. Landet nie in einem öffentlichen Repo. |

> ⚠️ **Wichtige Klarstellung zur ursprünglichen Idee:** In der Rohfassung sind *Agent View*
> (die Visualisierung) und *Agent Surface* (die eigentliche Schnittstelle) vermischt. Das sind
> zwei völlig verschiedene Komponenten mit unterschiedlicher Priorität. Die Schnittstelle ist
> essentiell. Die Visualisierung ist wunderschön, aber Phase 5.

---

## 3. Das neurowissenschaftliche Modell → Software-Mapping

Das Hippocampus-Vorbild ist keine Metapher zur Dekoration. Richtig angewandt liefert es
konkrete, nicht-offensichtliche Designentscheidungen. Falsch angewandt wird es zu einem
Vokabular, das über generisches RAG geklebt wird.

### 3.1 Die zentrale Einsicht

**Der Hippocampus speichert keine Erinnerungen. Er speichert Indizes auf neokortikale Muster.**

Übertragen: **Das Brain speichert keine Dateiinhalte. Das Dateisystem ist der Neokortex.**
Das Brain speichert Zeiger, Beziehungen, Episoden und verdichtete Repräsentationen.

Konsequenzen, die daraus zwingend folgen:
- Keine vollständige Dateikopie in der Datenbank. Chunks + Embeddings + Offsets, sonst nichts.
- Das Dateisystem bleibt jederzeit ohne die App voll benutzbar. Kein Lock-in, keine Sidecar-Files
  im Weg, keine proprietäre Ordnerstruktur.
- Der Index ist **jederzeit vollständig neu aufbaubar**. Er ist ein Cache, kein Datenspeicher.
  Das ist eine harte Anforderung — `--rebuild-index` muss alles wiederherstellen können.

### 3.2 Mapping-Tabelle

| Biologische Struktur | Funktion | Software-Komponente |
|---|---|---|
| **Sensorischer Input** | Rohwahrnehmung | `fanotify`/`inotify` Filesystem-Watcher, Shell-Hooks, Editor-Hooks |
| **Entorhinaler Cortex** | Ein-/Ausgangstor, Grid Cells (Raumkarte) | Ingest-Router + Verzeichnisgraph als Koordinatensystem |
| **Place Cells** | „Ich war an diesem Ort" | Verzeichnisse als Orte; Zugriffssequenzen als Trajektorien |
| **Gyrus dentatus** | Pattern Separation — Ähnliches unterscheidbar machen | Content-Hashing, Near-Duplicate-Detection, Versionsunterscheidung, Chunk-Boundaries |
| **CA3** | Pattern Completion — Fragment → ganze Erinnerung | Vector-ANN + assoziative Graphtraversierung („das Ding mit der Kamera-Sache") |
| **CA1** | Abgleich Erwartung ↔ Realität, Ausgabe | Reconciliation-Layer, Re-Ranking, Ergebnis-Scoring |
| **Präfrontaler Cortex** | Arbeitsgedächtnis, Aufmerksamkeit | Working Set: offene Ordner, aktive Datei, aktuelle Task, TTL ~ Session |
| **Amygdala / dopaminerge Tags** | Emotionale Salienz-Markierung | Salience-Engine: Zugriffshäufigkeit, Recency, explizite Pins, Projektkontext |
| **Sharp-Wave Ripples / Replay** | Wiederabspielen im Ruhezustand | Idle-Time-Indexer: teure Arbeit (OCR, Embeddings, Whisper) nur bei Leerlauf |
| **Systemkonsolidierung (Schlaf)** | Hippocampus → Neokortex-Transfer | Nightly Consolidation Job: Episoden → semantische Nodes → Upload ins Main Brain |
| **Semantisches Gedächtnis** | Wissen ohne Kontext des Lernens | Knowledge Graph im Main Brain (bestehendes System) |
| **Prozedurales Gedächtnis** | Gelernte Handlungsabläufe | Workflow-Erkennung: „nach X folgt bei dir meist Y" → Aktionsvorschläge |
| **Synaptisches Pruning / Vergessen** | Aktives Löschen zur Erhaltung der Leistung | Decay-Funktion + Eviction. **Nicht optional.** |

### 3.3 Die Komponente, die alle vergessen: Vergessen

Fast jedes „Second Brain"-Projekt scheitert daran, dass es nur akkumuliert. Nach 18 Monaten ist
der Index groß, langsam und voller Rauschen — und der Nutzer vertraut ihm nicht mehr.

Der biologische Hippocampus löscht aggressiv. Das System muss das auch:

- **Decay:** `salience(t) = base · e^(−λ·Δt) + Σ(Verstärkungen)`. Nodes unter Schwellwert fallen
  aus dem heißen Index.
- **Tiering statt Löschen:** STM → LTM → Cold. Aus Cold wird nur bei explizitem Deep Search gelesen.
- **Pins sind immun.** Was der Mensch explizit merkt, verfällt nie.
- **Vergessen ist reversibel** — der Index ist rekonstruierbar (siehe 3.1). Das macht aggressives
  Pruning risikofrei.

---

## 4. Systemarchitektur

```
┌───────────────────────── LOKALER PC (nicht immer an) ──────────────────────────┐
│                                                                                │
│   CLIENTS                                                                      │
│   ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────────────┐       │
│   │ Human View │  │ Agent View │  │  CLI/TUI   │  │ MCP Server         │       │
│   │   (GUI)    │  │  (Vis)     │  │            │  │ (Agent Surface)    │       │
│   └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  └─────────┬──────────┘       │
│         └───────────────┴───────────────┴───────────────────┘                  │
│                                 │ lokale IPC (Unix Socket, JSON-RPC)           │
│   ┌─────────────────────────────▼──────────────────────────────────────────┐   │
│   │                          CORTEX (Daemon)                               │   │
│   │                                                                        │   │
│   │  Query Engine ── Fusion ── Rerank ── Query Understanding (kleines LLM)  │   │
│   │       │                                                                │   │
│   │  ┌────▼─────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────┐   │   │
│   │  │Working   │ │   STM    │ │   LTM    │ │ Salience │ │Consolidation│   │   │
│   │  │  Set     │ │ (hot)    │ │(local $)  │ │  Engine  │ │   Engine    │   │   │
│   │  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────┬──────┘   │   │
│   │                                                             │          │   │
│   │  Ingest: Watcher → Extractor → Chunker → Embedder ──────────┘          │   │
│   │  Stores: SQLite (Meta/Episoden) │ FTS │ Vector │ Graph                 │   │
│   └────────────────────────────────┬───────────────────────────────────────┘   │
│                                    │                                           │
│   ┌────────────────────────────────▼───────────────────────────────────────┐   │
│   │  EXTENSION-LAYER (privat, nie im öffentlichen Repo)                    │   │
│   │  Sync-Bridge · persönliche Ontologie · Projekt-Regeln · Secrets        │   │
│   └────────────────────────────────┬───────────────────────────────────────┘   │
└────────────────────────────────────┼───────────────────────────────────────────┘
                                     │ Tailscale, mTLS, Outbox-Pattern
┌────────────────────────────────────▼───────────────────────────────────────────┐
│  HOMESERVER (immer an) — MAIN BRAIN                                            │
│  Bestehendes Second Brain · Vector Store · Knowledge Graph · Server-Agents      │
└────────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Der Cortex-Daemon im Detail

### 5.1 Ingest-Pipeline

**Stufe 1 — Watcher (immer aktiv, billig)**
- `fanotify` mit `FAN_REPORT_FID` für mount-weite Überwachung. `inotify` skaliert nicht
  (Watch-Limits pro Verzeichnis) und ist für ein `$HOME` mit vielen Ordnern ungeeignet.
- Debouncing: Editoren schreiben Dateien 3–5× beim Speichern. Coalescing-Fenster ~500 ms.
- Erfasst: Pfad, Event-Typ, mtime, size, inode. **Kein Inhalt.** Latenz-Ziel < 5 ms pro Event.
- Schreibt sofort eine **Episode**. Episoden sind der Rohstoff für alles Weitere.

**Stufe 2 — Klassifikation (schnell)**
- MIME-Erkennung über Magic Bytes, nicht über Endung.
- Deny-List-Prüfung (siehe Abschnitt 12) — **vor** jedem Lesen des Inhalts.
- Entscheidung: `metadata_only` | `extract` | `deep_extract`.

**Stufe 3 — Extraction (teuer, Queue-basiert, Idle-priorisiert)**
- Text/Code: direkt.
- PDF: pdfium oder poppler. Bei Scans → OCR-Queue.
- Office: docx/xlsx/pptx via entsprechende Parser.
- Bilder: EXIF immer; OCR und Captioning optional/opt-in.
- Audio/Video: Transkription optional. *Anmerkung:* Es existiert bereits ein faster-whisper-Setup
  auf dem Server — Transkription gehört dorthin ausgelagert, nicht auf den PC.

**Stufe 4 — Chunking (Pattern Separation)**
- Struktur-bewusst, nicht naiv nach Zeichen: Markdown nach Überschriften, Code nach Funktionen
  (tree-sitter), PDF nach Seiten/Abschnitten.
- Ziel ~300–600 Tokens mit ~15 % Overlap.
- Content-Hash pro Chunk → unveränderte Chunks werden bei Re-Index nicht neu embedded.
  Das ist der größte einzelne Performance-Hebel im gesamten Ingest.

**Stufe 5 — Embedding**
- **Muss mehrsprachig sein.** Die Dateien sind gemischt deutsch/englisch. Ein englisch-only
  Modell (`bge-small-en`, `all-MiniLM`) ist hier disqualifiziert.
- Kandidaten: `multilingual-e5-small` (schnell, 384d), `bge-m3` (besser, deutlich teurer),
  `jina-embeddings-v3` (gut für DE/EN-Mix).
- Modell-Version wird pro Chunk mitgespeichert → Modellwechsel erfordert kein Full-Rebuild,
  sondern eine Migration im Hintergrund.

### 5.2 Speicherschichten

| Schicht | Inhalt | Store | TTL / Retention |
|---|---|---|---|
| **Working Set** | Aktive Datei, offene Ordner, letzte 20 Aktionen, aktuelle Task | In-Memory | Session |
| **STM** | Alles der letzten ~30 Tage: volle Chunks, Embeddings, Episoden | SQLite + lokaler Vector-Index | 30 Tage rollierend |
| **LTM (lokal)** | Konsolidierte Zusammenfassungen, Konzept-Nodes, Kanten | SQLite | Persistent, gepruned nach Salience |
| **LTM (Main Brain)** | Autoritative semantische Ebene | Bestehender Server-Stack | Persistent |
| **Cold** | Metadaten + Pfad, kein Embedding | SQLite | Persistent, minimal |

### 5.3 Consolidation Engine

Läuft nächtlich und bei längerem Idle. Das ist die Komponente, die aus einem Suchindex
ein Gedächtnis macht.

Ablauf pro Durchlauf:
1. **Replay:** Episoden des Zeitraums laden, nach Session/Projekt clustern.
2. **Abstraktion:** Pro Cluster ein semantischer Node erzeugen — „Am 14.8. wurde 3 h am
   Snowboard-Kamerasystem gearbeitet, betroffen: diese 7 Dateien." Hier ist ein LLM angebracht
   (asynchron, nicht latenzkritisch → darf das größere Modell sein oder gegen den Server laufen).
3. **Kantenbildung:** Co-Access, Import-Beziehungen im Code, semantische Ähnlichkeit,
   gemeinsame Session → Graph-Edges mit Gewicht.
4. **Salience-Neuberechnung** über alle Nodes.
5. **Promotion:** Nodes über Salience-Schwelle → Sync-Outbox für Main Brain.
6. **Pruning:** Nodes unter Schwelle → Tier-Downgrade.
7. **Report:** Ein kurzer, für Menschen lesbarer Bericht, was konsolidiert wurde. Wichtig für
   Vertrauen — ein Gedächtnis, dessen Arbeit unsichtbar ist, wird nicht geglaubt.

### 5.4 Salience-Modell

```
salience = w1·log(1+access_count)
         + w2·recency_decay(last_access)
         + w3·edit_frequency
         + w4·graph_centrality
         + w5·explicit_pin        (dominant, überschreibt alles)
         + w6·project_context     (liegt in aktivem Projektordner)
         − w7·noise_penalty       (Build-Artefakte, node_modules, Caches, Downloads-Müll)
```

Gewichte müssen konfigurierbar und die Berechnung **erklärbar** sein: Zu jedem Ranking muss
das System auf Nachfrage sagen können, *warum* etwas oben steht. Ohne das ist die Suche
nicht debugbar und nicht vertrauenswürdig.

---

## 6. Suche: Latenzbudget und Fusion

### 6.1 Die wichtigste Korrektur an der Ursprungsidee

Die Idee sagt: „Schnellsuchmodus, der mit einer kleinen lokalen KI läuft, in wenigen Sekunden."

**Wenige Sekunden ist zu langsam.** Der Vergleichsmaßstab ist nicht ChatGPT, sondern `fzf`
und Spotlight. Ab ~150 ms fühlt sich Suche träge an; ab 1 s tippt der Mensch lieber selbst
den Pfad.

**Ein LLM gehört nicht in den heißen Pfad.** Die Architektur muss so aussehen:

```
Tastendruck
   │
   ├─► [0–60 ms]   Trigram-Pfadmatch + BM25 + Vector-ANN, parallel
   │               → Reciprocal Rank Fusion → ERSTE ERGEBNISSE SICHTBAR
   │
   ├─► [~150 ms]   Cross-Encoder-Rerank der Top-30 → Liste ordnet sich neu
   │
   └─► [~400 ms]   Kleines LLM parst Query → strukturierte Filter
                   ("PDFs von letzter Woche über Kunden") → Liste verfeinert sich
```

Das Ergebnis ist nie leer und nie blockiert. Die KI *verbessert* das Ergebnis progressiv,
statt es zu erzeugen. Das ist der Unterschied zwischen einem Werkzeug und einem Demo.

### 6.2 Latenzbudget (P95, harte Zielwerte)

| Stufe | Budget |
|---|---|
| Keystroke → erste sichtbare Zeile | **< 80 ms** |
| Fuzzy-Pfadmatch (in-memory Trigram) | < 10 ms |
| BM25 Top-100 | < 30 ms |
| Vector-ANN Top-100 | < 40 ms |
| RRF-Fusion + Salience-Boost | < 5 ms |
| Cross-Encoder-Rerank Top-30 (async) | < 150 ms |
| LLM-Query-Parsing (async, konditional) | < 400 ms |
| Verzeichnis mit 10.000 Einträgen öffnen | < 100 ms bis erste Zeile |

Diese Zahlen gehören als Benchmark-Suite ins Repo. Nicht als Kommentar, sondern als CI-Gate.

### 6.3 Query-Modi

- **Locate:** Pfad/Name, rein lexikalisch. Der 90 %-Fall.
- **Recall:** Semantisch, Pattern Completion. „Das Ding, wo ich die Kamerawinkel verglichen habe."
- **Temporal:** „Woran habe ich Dienstag gearbeitet." Läuft rein über Episoden — kein Vector nötig.
- **Structural:** Graph-Traversierung. „Was hängt mit dieser Datei zusammen."
- **Deep:** Bezieht Cold Tier und Main Brain ein. Explizit ausgelöst, darf Sekunden dauern.

Der Modus wird heuristisch erraten und ist per Präfix erzwingbar (`>`, `~`, `@`, o. ä.).

### 6.4 Modellkandidaten für lokale Inferenz

| Rolle | Kandidat | Anmerkung |
|---|---|---|
| Embedding | `multilingual-e5-small` | DE/EN-Mix zwingend erforderlich |
| Reranking | `bge-reranker-v2-m3`, `jina-reranker-v2` | Cross-Encoder schlägt LLM-Reranking bei Kosten/Nutzen deutlich |
| Query-Parsing | Qwen3-0.6B / Llama-3.2-1B via llama.cpp | Grammar-constrained JSON-Output, keine freie Generierung |
| Konsolidierung | Größeres Modell oder Server-Route | Nicht latenzkritisch |

---

## 7. Human View — UX-Spezifikation

### 7.1 Die unbequeme Wahrheit vorab

Ein Dateimanager, der die Basics nicht kann, wird nicht benutzt — egal wie gut das Brain ist.
Und die Basics sind mehr Arbeit als das Brain. Das ist der größte Scope-Risikofaktor
des Projekts (siehe Abschnitt 15).

**Pflichtfunktionen, ohne die es ein Spielzeug bleibt:**
- Kopieren/Verschieben/Umbenennen mit Fortschritt, Abbruch und Konfliktdialog
- XDG-Trash-Spec (Papierkorb, kein `rm`)
- Rechte, Besitzer, symbolische Links korrekt dargestellt
- Mounts via `udisks2` (USB-Sticks, externe Platten)
- Thumbnails nach XDG-Thumbnail-Spec (Cache teilen, nicht neu erfinden)
- `xdg-mime` für „Öffnen mit"
- Wayland Drag & Drop, Copy/Paste über `wl-clipboard`-kompatible Mechanismen
- Archive: mindestens zip/tar entpacken
- Netzwerk-Locations (SMB/SFTP) — kann Phase 2 sein, aber der Homeserver macht es relevant

### 7.2 Niri-spezifische Anforderungen

Niri ist ein *scrollable-tiling* Compositor. Das erzeugt konkrete Constraints, die die meisten
GUI-Frameworks nicht von selbst erfüllen:

- **Spaltenbreite ist variabel und oft schmal.** Das Layout muss von ~400 px bis Vollbreite
  funktionieren. Responsive Breakpoints wie im Web, nicht ein festes Desktop-Layout.
- **Kein eigenes Fenster-Management annehmen.** Keine Popup-Fenster, keine eigenen
  Fenster-Tabs — Niri macht das Layout. Interne Panes ja, zusätzliche Toplevels nein.
- **Keyboard-first ist Pflicht, nicht Feature.** Wer Niri benutzt, benutzt die Maus ungern.
  Jede Aktion braucht ein Keybinding. Ein Command-Palette-Modus (Ctrl+K) ist zentral.
- **Reines Wayland.** Kein XWayland-Fallback, keine X11-spezifischen Annahmen. CSD korrekt
  handhaben oder serverseitig dekorieren lassen.
- **Startzeit zählt.** Bei tiling WMs wird häufig geöffnet und geschlossen. Kaltstart-Ziel < 300 ms
  — der Daemon läuft ohnehin, die GUI muss nur noch verbinden.

### 7.3 Interaktionsmodell

- **Dual-Pane optional**, Miller Columns optional, Liste als Default.
- **Command Palette** als primärer Einstiegspunkt für alles.
- **Suchmodus als Overlay**, nicht als separate Seite. Ergebnisse ersetzen die Liste live.
- **Kontextleiste:** Zu jeder selektierten Datei zeigt eine Seitenleiste, was das Brain über sie
  weiß — verwandte Dateien, letzte Episoden, Tags, warum sie salient ist.
- **Vergessen sichtbar machen:** Ein diskreter Hinweis, wenn Ergebnisse aus dem Cold Tier
  stammen. Der Mensch soll das Tiering-Modell verstehen können.

---

## 8. Agent Surface — die Maschinenschnittstelle

Das ist die eigentliche „KI-Ansicht" im funktionalen Sinn. Umsetzung: **MCP-Server**, plus
JSON-RPC über Unix Socket für lokale Clients, plus CLI (nützlich für Skripte und für dich in
Claude Code).

### 8.1 Vorgeschlagenes Tool-Set

**Lesen / Erinnern**
| Tool | Zweck |
|---|---|
| `brain.search(query, mode, filters, k)` | Hauptsuche, alle Modi |
| `brain.recall(cue, depth)` | Pattern Completion — Fragment → Kontext |
| `brain.neighbors(node_id, edge_types, depth)` | Assoziative Traversierung |
| `brain.timeline(from, to, filter)` | Episodischer Abruf |
| `brain.working_set()` | Was macht der Mensch gerade? Kontext für Agents. |
| `brain.explain(node_id)` | Warum ist das relevant? Salience-Begründung. |
| `brain.diff(since)` | Was hat sich geändert? Für polling-arme Agents. |

**Dateizugriff**
| Tool | Zweck |
|---|---|
| `fs.list(path, glob, depth)` | Verzeichnis lesen |
| `fs.read(path, byte_range?)` | Inhalt lesen |
| `fs.stat(path)` | Metadaten |
| `fs.write(path, content, mode)` | **Gated.** Siehe 8.2. |
| `fs.move` / `fs.trash` | **Gated.** |

**Gedächtnis schreiben (verändert keine Dateien)**
| Tool | Zweck |
|---|---|
| `brain.annotate(target, tags, note)` | Wissen anlagern, ohne Datei anzufassen |
| `brain.pin(target)` / `brain.forget(target)` | Salience manuell setzen |
| `brain.link(a, b, type)` | Explizite Kante |

### 8.2 Schreibrechte — harte Regel

**Agents bekommen standardmäßig Lesezugriff auf Dateien und Schreibzugriff nur auf das Brain.**

Dateisystem-Schreiboperationen laufen über eine Policy-Engine:
- Scope-Beschränkung auf konfigurierte Pfade (Allow-List, nicht Deny-List)
- Dry-Run-Pflicht: `fs.move` gibt erst einen Plan zurück, der bestätigt werden muss
- Jede Mutation erzeugt eine Episode mit Urheber-Kennung (welcher Agent, welche Session)
- Undo-Journal für alle agentischen Mutationen

Der Grund ist nicht Paranoia: Ein Agent mit ungebremstem `fs.move` auf einem indizierten
`$HOME` kann in 200 ms mehr Schaden anrichten, als ein Backup rückgängig macht.

### 8.3 Agent View (Visualisierung)

Für Menschen, um das Brain zu *verstehen*. Bewusst **niedrige Priorität** — Phase 5.

Sinnvoller Inhalt: Graph der Konzept-Nodes und ihrer Kanten, Salience als Größe/Helligkeit,
Tier als Tiefe/Farbe, Konsolidierungs-Events als Animation, aktueller Working Set hervorgehoben.

*Anmerkung:* Für die Graph-Darstellung existiert bereits ein `3d-force-graph`-basiertes Setup
(Graphify) auf dem Server. Das sollte wiederverwendet und nicht neu gebaut werden — im
Zweifel wird die Agent View schlicht ein eingebetteter View auf die bestehende Komponente.

---

## 9. Föderation: Cortex ↔ Main Brain

### 9.1 Rollenmodell

Das muss unmissverständlich festgelegt sein, sonst entstehen Konflikte, die niemand auflösen kann.

| Datenart | Autorität | Begründung |
|---|---|---|
| Dateien, Pfade, mtimes, lokale Struktur | **Cortex (PC)** | Nur der PC sieht sein Dateisystem |
| Episoden (lokale Ereignisse) | **Cortex (PC)** | Append-only, entstehen lokal |
| Semantische Konzepte, projektübergreifendes Wissen | **Main Brain (Server)** | Aggregiert über alle Quellen |
| Ontologie, Tag-Taxonomie | **Main Brain (Server)** | Muss konsistent über Geräte sein |
| Salience | **Beides, getrennt gehalten** | Lokale und globale Salience sind verschiedene Größen |

**Der Server ist nie die Autorität über den lokalen Dateizustand. Der PC ist nie die Autorität
über globales Wissen.** Diese Trennung eliminiert die meisten denkbaren Konfliktfälle.

### 9.2 Sync-Mechanik

- **Outbox-Pattern.** Der Cortex schreibt Sync-Events in eine lokale Outbox-Tabelle. Ein Worker
  arbeitet sie ab, wenn der Server erreichbar ist. Offline ist der Normalfall, nicht die Ausnahme.
- **Append-only Events + Lamport-Timestamps.** Kein bidirektionales State-Merging. Der Server
  konsumiert Events, der Cortex konsumiert Ontologie-Updates. Zwei Einbahnstraßen statt einer
  Kreuzung.
- **Idempotenz:** Jedes Event hat eine stabile ID. Doppelte Zustellung darf nichts kaputt machen.
- **Kein Dateiinhalt über den Sync**, außer explizit freigegeben. Standardmäßig fließen nur
  Metadaten, Zusammenfassungen und Embeddings nach oben.
- **Transport:** Tailscale (existiert bereits), zusätzlich mTLS. Kein offener Port.
- **Backpressure:** Nach 3 Tagen Offline darf der erste Sync den Server nicht fluten. Batch-Limits
  und Rate-Limiting gehören in die erste Version, nicht in die zweite.

### 9.3 Warum der Server das Main Brain ist

Weil er immer läuft. Der PC ist ein **Sinnesorgan mit lokalem Kurzzeitgedächtnis** — er nimmt
wahr, puffert, verdichtet und schiebt nach oben. Das ist exakt die biologische Rollenverteilung
zwischen Hippocampus und Neokortex, und es ist zufällig auch die richtige verteilte Systemarchitektur.

Genau hier liegt der Punkt, an dem die Hippocampus-Analogie aufhört, Dekoration zu sein.

---

## 10. Core / Extension — die Open-Source-Grenze

### 10.1 Prinzip

> Das öffentliche Repo enthält ein vollständig funktionsfähiges Produkt.
> Die persönliche Anpassung ist ein Plugin, kein Fork.

Das ist nicht nur eine Datenschutzfrage, sondern eine Qualitätsfrage: Ein Core, der ohne die
privaten Teile nicht läuft, ist schlecht geschnitten. Der Zwang zur sauberen Grenze macht die
Architektur besser.

### 10.2 Aufteilung

**Core (öffentlich):**
- Daemon, Ingest, alle Indizes, Suche, Memory-Tiers, Consolidation
- Human View, Agent View, CLI
- MCP-Server
- **Generische Sync-Schnittstelle** — ein definiertes Protokoll plus Referenz-Adapter, aber
  ohne Wissen über eine konkrete Gegenstelle
- Plugin-Loader

**Extension (privat):**
- Sync-Bridge zum konkreten Main Brain: Endpunkte, Schemas, Auth
- Persönliche Ontologie und Tag-Taxonomie
- Projektspezifische Regeln und Ordner-Semantik
- Alle Secrets (→ hier greift das bestehende `secretctl`-Vault-Setup)

### 10.3 Plugin-Mechanik

Empfehlung: **Plugins als separate Prozesse**, die dasselbe JSON-RPC-Protokoll sprechen wie
alle anderen Clients. Kein dynamisches Linken, kein ABI-Problem, keine Sprachbindung.

Vorteile: sprachunabhängig, crash-isoliert, separat versionierbar, und ein Plugin ist automatisch
auch als eigenständiges Tool testbar.

Der Core lädt aus `~/.config/<app>/plugins/*.toml` und startet die deklarierten Prozesse.

### 10.4 Hygiene-Regeln für das öffentliche Repo (nicht verhandelbar)

- Keine Hostnamen, Tailscale-Namen, IPs, Nutzernamen, Ordnerpfade aus dem Privatsystem
- Keine Secrets — auch nicht in Beispielkonfigurationen, auch nicht in Tests, auch nicht in
  Commit-Historie
- Trennung **von Anfang an**, nicht nachträglich. Nachträgliches Herausschneiden aus der
  Git-History ist Arbeit, die niemand macht — und Secrets in alten Commits sind der häufigste
  Leak überhaupt
- Pre-Commit-Hook mit Secret-Scanner, CI-Gate

### 10.5 Zweiter Nutzer

Ein Kollege soll das System ebenfalls nutzen. Konsequenzen:
- Erstinstallation muss ohne Extension-Layer funktionieren und in unter 10 Minuten laufen
- Onboarding-Flow: Welche Ordner indizieren? Welches Embedding-Modell? Erster Index-Lauf
  mit Fortschrittsanzeige
- Konfiguration klar getrennt in „Systemdefaults" / „Nutzerkonfiguration" / „Extension"
- Sein Setup mit seinem eigenen Main Brain (oder ganz ohne) muss ein unterstützter Fall sein,
  kein Sonderfall

---

## 11. Tech-Stack — Optionen mit Abwägung

### 11.1 Daemon-Sprache: Rust

Wenig Diskussionsbedarf. Begründung: die Latenzziele in 6.2 sind in Python nicht erreichbar;
`fanotify`-Handling, Tantivy, tree-sitter, llama.cpp-Bindings und ONNX Runtime sind alle
verfügbar; Single-Binary-Deployment vereinfacht die Veröffentlichung erheblich.

### 11.2 GUI-Toolkit — die eigentliche Entscheidung

| Option | Für | Gegen |
|---|---|---|
| **GTK4 + libadwaita** (gtk4-rs) | Nativ, schnellste Listen, korrektes Wayland-Verhalten „gratis", geringer RAM | Steilere Lernkurve, aufwendige 3D-Visualisierung, opinionated Styling |
| **Tauri v2** (Rust + Web-Frontend) | Passt zum vorhandenen Web/3D-Skillset, Agent View trivial (3d-force-graph existiert bereits), schnelle UI-Iteration | WebKitGTK-Performance bei sehr großen Listen erfordert Virtualisierung, höherer RAM, IPC-Overhead im Hot Path |
| **Iced / egui** | Pure Rust, sehr schnell | Ökosystem für Dateimanager-Primitiven (DnD, Thumbnails, Dialoge) dünn; viel Eigenbau |

**Empfehlung: Tauri v2** — aber unter zwei harten Bedingungen:
1. Jede Listenoperation ist virtualisiert (nur sichtbare Zeilen im DOM). Ohne das bricht die
   Ansicht bei 10.000 Dateien ein.
2. Kein Datenverarbeitungsschritt im Frontend. Das Frontend rendert, was der Daemon liefert —
   es filtert, sortiert und sucht nicht selbst.

Begründung der Empfehlung: Das Projekt hat zwei Ansichten, von denen eine (Agent View) im Web-Stack
trivial und in GTK aufwendig ist; das vorhandene Können liegt im Web-Bereich; und die
Daemon-Architektur macht einen späteren GUI-Wechsel billig. Wenn sich Punkt 1 in Phase 2 als
nicht erreichbar erweist, ist der Wechsel zu GTK4 explizit vorgesehen und kostet nur die
GUI-Schicht.

### 11.3 Storage

| Zweck | Wahl |
|---|---|
| Metadaten, Episoden, Graph-Kanten | SQLite (WAL-Modus) |
| Volltext | Tantivy, alternativ SQLite FTS5 bei kleinerem Scope |
| Vektoren lokal | `sqlite-vec` (einfach) oder LanceDB (skaliert besser) |
| Vektoren remote | Qdrant (existiert bereits auf dem Server) |
| Graph | Kantentabelle in SQLite. Eine eigene Graph-DB ist bei dieser Größenordnung Overkill. |

Alles in **einer** SQLite-Datei zu halten, wo möglich, ist ein unterschätzter Vorteil: Backup,
Migration und Debugging werden trivial.

---

## 12. Sicherheit und Privatsphäre — harte Regeln

Diese Punkte sind nicht optional und gehören in die erste Codezeile, nicht in ein späteres Ticket.

**Deny-List, greift vor jedem Inhaltszugriff:**
- `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.kube`, Keyring-Dateien
- `.env`, `*.pem`, `*.key`, `id_*`, `credentials*`, `*.kdbx`
- Browser-Profile (enthalten Session-Cookies), Passwort-Manager-Speicher
- `node_modules`, `.git/objects`, `target/`, `__pycache__`, Build-Verzeichnisse (Performance-Gründe)

**Weitere Regeln:**
- `.gitignore`-Semantik wird respektiert; zusätzlich eine eigene `.brainignore`
- Opt-in statt Opt-out für Inhaltsextraktion außerhalb explizit konfigurierter Ordner
- Der Index liegt verschlüsselt oder mindestens mit `0600` und in `~/.local/state`
- **Kein Netzwerkzugriff im Default-Zustand.** Cloud-LLMs, Telemetrie, Update-Checks: alles
  explizit einzuschalten. Ein Second Brain, das ungefragt telefoniert, ist unbenutzbar.
- Agent-Zugriffe werden protokolliert und sind einsehbar
- Ein sichtbarer Kill-Switch: Indizierung pausieren, Agent Surface deaktivieren

---

## 13. Roadmap mit Definition of Done

| Phase | Inhalt | Definition of Done |
|---|---|---|
| **M0** | Daemon-Skelett, SQLite-Schema, JSON-RPC-Socket, CLI | `cortex search foo` liefert Ergebnisse aus einem Testkorpus |
| **M1** | fanotify-Watcher, Metadaten-Index, Episoden-Log, Trigram-Suche | 100k Dateien indiziert; Pfadsuche < 10 ms; Watcher überlebt 24 h ohne Leak |
| **M2** | Extraction, Chunking, Embeddings, BM25 + Vector + RRF-Fusion | Benchmark-Suite grün gegen 6.2; Recall auf einem 50-Query-Goldset gemessen |
| **M3** | Human View: echter Dateimanager (Abschnitt 7.1) + Suchoverlay | Ersetzt den bisherigen Dateimanager im Alltag. Ehrlicher Test: 1 Woche exklusiv nutzen. |
| **M4** | MCP-Server, Policy-Engine, Undo-Journal | Claude Code arbeitet produktiv gegen die Schnittstelle |
| **M5** | Consolidation, Salience, Decay, Tiering | Nach 30 Tagen Nutzung ist die Suche nicht langsamer geworden |
| **M6** | Sync-Bridge zum Main Brain (als Extension) | Ein Tag offline, dann sauberer Reconcile ohne Duplikate |
| **M7** | Agent View / Visualisierung | Der Konsolidierungsprozess ist visuell nachvollziehbar |
| **M8** | Core/Extension-Split, Packaging, Docs, Release | Der Kollege installiert es ohne Hilfe und ohne Zugriff auf private Repos |

**M0–M2 sind headless.** Das ist Absicht. Wenn die Suche in der CLI nicht überzeugt, überzeugt
sie in keiner GUI.

---

## 14. Anhang: Datenmodell-Skizze

```sql
-- Der Neokortex-Zeiger: was existiert, wo
nodes(id, kind, path, mime, size, mtime, ctime, content_hash,
      tier, salience, pinned, first_seen, last_seen)
      -- kind: file | dir | chunk | concept | annotation

-- Der Hippocampus-Index: assoziative Verbindungen
edges(src, dst, type, weight, created_at, source)
      -- type: contains | co_access | imports | similar_to | derived_from | annotates
      -- source: observed | inferred | explicit

-- Episodisches Gedächtnis: append-only, unveränderlich
episodes(id, ts, actor, action, node_id, session_id, metadata_json)
      -- actor: human | agent:<name> | system
      -- action: create | modify | open | move | delete | search | annotate

-- Pattern-Separation-Einheiten
chunks(id, node_id, ordinal, byte_start, byte_end, hash, text_preview)

-- Vektoren, modellversioniert
embeddings(chunk_id, model_id, model_version, vector, created_at)

-- Föderation
sync_outbox(id, event_id, payload_json, created_at, attempts, last_error)
sync_state(peer, last_acked_lamport, last_sync_at)

-- Erklärbarkeit
salience_log(node_id, computed_at, score, components_json)
```

Designnotiz: `episodes` ist die einzige Tabelle, die nie mutiert wird. Alles andere ist aus ihr
und dem Dateisystem rekonstruierbar. Das ist die technische Umsetzung von Abschnitt 3.1 und
die Voraussetzung dafür, dass aggressives Vergessen ungefährlich ist.

---

## 15. Ehrliche Scope-Einschätzung und Risiken

Dieser Abschnitt gehört bewusst ins Dokument, weil ein Build-Prompt-Agent von sich aus
niemals widersprechen wird.

**Das hier sind drei Projekte in einem Trenchcoat:**
1. Ein vollwertiger Dateimanager (viel Arbeit, wenig Neuheit, aber unverzichtbar)
2. Eine lokale Suchmaschine mit Hybrid-Retrieval (anspruchsvoll, gut abgegrenzt)
3. Ein verteiltes Gedächtnissystem (der eigentlich interessante Teil)

Teil 1 ist der Zeitfresser und der wahrscheinlichste Grund für ein Steckenbleiben. Es gibt zwei
Auswege, die vor Projektbeginn bewertet werden sollten:

- **Option A — Volle Eigenentwicklung.** Wie oben beschrieben. Höchste Kohärenz, längster Weg.
- **Option B — Brain zuerst, Dateimanager später oder nie.** Der Cortex-Daemon plus MCP plus
  ein Suchoverlay (Launcher-artig, wie Raycast/Rofi) liefert vielleicht 80 % des tatsächlichen
  Alltagsnutzens bei 30 % des Aufwands. Der bestehende Dateimanager bleibt erst mal. Die GUI
  kommt, wenn der Daemon sich bewährt hat.

Option B ist strategisch die stärkere Wahl, und die Daemon-Architektur hält Option A jederzeit
offen. Das ist keine Reduktion der Ambition, sondern die Reihenfolge, in der die Ambition
überhaupt erreichbar ist.

**Weitere Risiken:**

| Risiko | Gegenmaßnahme |
|---|---|
| Embedding-Backfill über 500k Dateien blockiert die Maschine tagelang | Idle-Priorisierung, Chunk-Hash-Dedup, Nice-Level, harte Ressourcenbudgets |
| Watcher-Events gehen bei Systemlast verloren → Index driftet | Periodischer Reconcile-Scan gegen mtime; Index als Cache behandeln |
| Deutsch/Englisch-Mix bricht die semantische Suche | Mehrsprachiges Modell zwingend; Goldset mit deutschen Queries |
| Agent löscht oder verschiebt versehentlich | Policy-Engine, Dry-Run, Undo-Journal — ab M4, nicht später |
| Sync erzeugt Duplikate im Main Brain | Stabile Event-IDs, Idempotenz, Reconcile-Test in CI |
| Feature-Creep über die Hippocampus-Analogie | Jede biologische Analogie muss eine konkrete Designentscheidung begründen, sonst fliegt sie raus |

---

## 16. Offene Entscheidungen — vor dem Build-Prompt zu klären

Der Build-Prompt-Agent braucht diese Antworten. Ohne sie rät er, und geratene Annahmen
setzen sich im generierten Code fest.

1. **Option A oder B** aus Abschnitt 15? (Größte Einzelentscheidung.)
2. **GUI-Stack:** Tauri oder GTK4? Falls unentschieden → M0–M2 headless bauen und in M3 entscheiden.
3. **Korpusgröße:** Wie viele Dateien umfasst `$HOME` realistisch? (< 50k / 50k–500k / > 500k ändert
   die Index-Strategie erheblich.)
4. **Hardware:** CPU, RAM, GPU des PCs? Entscheidet, ob lokale Embeddings realistisch sind oder
   ob der Server das übernimmt.
5. **Main-Brain-Schnittstelle:** Welches Protokoll, welches Schema? Existiert dort bereits eine
   dokumentierte API oder muss sie mitdefiniert werden?
6. **Autorität bei Sync-Konflikten** — ist die Aufteilung in 9.1 so akzeptiert?
7. **Inhalte im Sync:** Fließen Chunk-Texte nach oben oder nur Zusammenfassungen und Vektoren?
8. **Projektname und Repo-Struktur:** Monorepo mit privatem Submodul, oder zwei getrennte Repos?
9. **Lizenz** für den Core.
10. **Nutzt der Kollege ein eigenes Main Brain, ein gemeinsames, oder keines?** Beeinflusst das
    Mandantenmodell erheblich und ist teuer nachzurüsten.
11. **Sprache der Benutzeroberfläche:** Deutsch, Englisch, oder i18n von Anfang an?

---

## 17. Was der Build-Prompt-Agent daraus machen soll

Empfohlene Struktur des zu erzeugenden Build-Prompts:

1. **Ein Prompt pro Milestone**, nicht ein Mega-Prompt. M0–M2 sind sequentiell abhängig,
   M3 und M4 können parallel laufen.
2. **Jeder Prompt enthält:** Kontext aus Abschnitt 1–3 (gekürzt), das relevante Datenmodell,
   die Definition of Done als ausführbare Tests, die harten Regeln aus Abschnitt 12.
3. **Tests vor Implementierung** bei allem, was Latenzziele hat. Die Benchmarks aus 6.2 sind
   die Spezifikation, nicht der Prosatext.
4. **Explizites Verbot**, den Core/Extension-Split zu verwässern. Der Agent muss angewiesen
   werden, bei jedem privaten Detail zu stoppen und nachzufragen, statt es einzubauen.
5. **Das Glossar aus Abschnitt 2 wortwörtlich** in jeden Prompt übernehmen.
