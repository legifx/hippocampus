# Hippocampus — Agent Entry Point

**Read this first. Then read `docs/03-PROGRESS.md` to find out where the previous agent stopped.**

Hippocampus is a file manager for Linux/niri whose actual core is a headless memory daemon
(`cortex`). The file manager is one of several equal clients. See `docs/04-ARCHITECTURE.md`.

## Reading order for a new agent

| # | File | Why |
|---|---|---|
| 1 | `docs/03-PROGRESS.md` | **Current state.** What is done, what is next, how to resume. |
| 2 | `docs/00-GLOSSARY.md` | Binding vocabulary. Using different words causes architectural drift. |
| 3 | `docs/01-BUILD-PROMPT.md` | The milestone plan. One section per milestone, with Definition of Done. |
| 4 | `docs/02-DECISIONS.md` | Why things are the way they are. Append-only ADR log. |
| 5 | `docs/04-ARCHITECTURE.md` | Component map and data model. |
| 6 | `docs/05-HARD-RULES.md` | Security, privacy and repo hygiene. Non-negotiable. |

The source specification this project derives from is `docs/99-SOURCE-SPEC.md`.

## Non-negotiable rules for every agent working here

1. **Core / Extension split.** This repository is public. It must never contain hostnames,
   IP addresses, tailnet names, usernames, private folder paths, secrets, or anything specific
   to one person's setup. Anything personal belongs in the separate private extension repo.
   **If a task requires a private detail, stop and ask. Do not inline it.**
2. **The index is a cache, never a data store.** Everything must be reconstructible from the
   filesystem plus the append-only `episodes` table. `cortex --rebuild-index` must restore
   full function. No feature may violate this.
3. **Latency budgets are the specification.** The numbers in `docs/01-BUILD-PROMPT.md` §Benchmarks
   are CI gates, not aspirations. Write the benchmark before the implementation.
4. **No network access by default.** No telemetry, no update checks, no cloud calls unless the
   user explicitly enables them in config.
5. **Update the docs in the same commit as the code.** Specifically `docs/03-PROGRESS.md`
   (state) and `docs/02-DECISIONS.md` (whenever a decision changes). An agent that ships code
   without updating progress has not finished the task.

## Repository layout

```
crates/
  cortex-core      domain types, IDs, error types, config schema
  cortex-store     SQLite schema + migrations, FTS, vector index, episode log
  cortex-ingest    watcher, classifier, extractor, chunker, embedder
  cortex-query     search modes, RRF fusion, rerank, explainability
  cortex-agent     policy engine, kill switch, audit log, fs tools, undo journal
  cortex-memory    salience, decay, tiering, consolidation
  cortex-rpc       JSON-RPC over Unix socket (server side) + protocol types
  cortex-sync      generic federation protocol + reference adapter (no concrete peer)
  cortex-plugin    plugin loader (subprocess, same JSON-RPC)
  cortexd          the daemon binary
  cortex-cli       CLI client binary
  cortex-mcp       MCP server binary (Agent Surface) — a bridge, no policy in it
  cortex-preview   the thumbnailer: previews in a process that is meant to be killable
ui/                Tauri v2 + React frontend (from M3)
ui-agent/          the Agent View: three.js, read-only, its own Vite app (M8, D66)
bench/             benchmark suite — the CI gate
docs/              see reading order above
```

## Language

Code, comments, commit messages, CLI output and documentation: **English**.
The GUI is internationalised from the start (i18n), default locale **de-DE**.
