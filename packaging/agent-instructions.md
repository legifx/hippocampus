## Gehirn: Hippocampus — Pflicht für jeden Agenten auf diesem Rechner

Alle Agenten teilen ein Gedächtnis: den Hippocampus-Daemon (`cortexd`). Erreichbar als MCP-Server
`hippocampus` (Werkzeuge `brain_*`) oder per CLI `cortex --as <dein-name> …`.

- **Erst fragen, dann suchen.** Bevor du nach Dateien, Entscheidungen, Logs oder Projektwissen
  suchst: `brain_ask("<Frage als Satz>")` (CLI: `cortex ask "<Frage>"`). Es liefert die passenden
  Pfade mit Begründung. Nur wenn es nichts findet, selbst greppen/lesen.
- **Zu Aufgabenbeginn:** `brain_context("<Aufgabe in einem Satz>", project_paths=["<Projektordner>"])`
  — Vorlieben, Arbeitsstand, die Erinnerungen dieses Projekts.
- **Aufschreiben, was die nächste Sitzung sonst neu herausfinden müsste:**
  `brain_remember(text, kind, scope)` — kind `fact|decision|procedure|preference|failure`,
  scope `project` (mit `about` = Pfad im Projekt) oder `global`. Ein Gedanke pro Eintrag, belegt.
  Nie: Chatverläufe, TODO-Listen, Zwischenstände, Geheimnisse.
- **Rückmelden:** nach einem Paket `brain_outcome(pack_id, helped=[…], noise=[…])` — ehrlich.
- Die Projektdoku im Repo (CLAUDE.md, README, docs/) bleibt der Einstieg; das Gehirn verweist
  darauf und ersetzt sie nicht.
