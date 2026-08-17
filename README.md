# Hippocampus

**A memory daemon for your filesystem — and the file manager that grew out of it.**

> Status: early. M0 (daemon skeleton) is done and tested. See `docs/03-PROGRESS.md`.

---

A filesystem is already a memory. It is just a very bad one: it stores content, but no
**episodes** (when, why, in what context), no **salience** (what matters), and no
**associations** (what belongs with what). That is why people *search* for files instead of
*remembering* them.

Hippocampus inverts the priority:

> The file manager is not the application. It is one of several views onto a local memory daemon.

The daemon — `cortexd` — watches the filesystem, records what happens, indexes what matters,
condenses it over time, and forgets what stopped being relevant. A CLI, a GUI, and an MCP server
for AI agents are equal clients on top of it.

## What makes it different

**The index is a cache, never a data store.** Hippocampus stores pointers, offsets, hashes and
relationships. It never stores a copy of your files, never writes sidecar files next to them,
and never imposes a folder structure. Delete the index and it rebuilds itself. Your filesystem
stays fully usable without this program.

**It forgets.** Almost every "second brain" project only accumulates, and after eighteen months
it is large, slow, and full of noise that its owner no longer trusts. Hippocampus decays
salience over time and demotes cold material to a metadata-only tier. Pinned items are immune.
Forgetting is safe precisely because the index is reconstructible.

**No AI in the hot path.** The benchmark for search is `fzf` and Spotlight, not a chat window.
Lexical and vector retrieval return results in under 80 ms; models then *improve* the ranking
progressively. The result list is never empty and never blocked on a model.

**Agents are first-class, and sandboxed.** The MCP surface gives agents read access to files and
write access to the memory. Filesystem mutations require an allow-list, a confirmed dry-run, and
land in an undo journal. An agent with unchecked `fs.move` over an indexed home directory can do
more damage in 200 ms than a backup undoes.

**Nothing leaves your machine.** A fresh install makes zero outbound connections. No telemetry,
no update checks, no cloud models. Federation with a remote memory is optional and additive —
the system is complete without it.

## Status and roadmap

| Milestone | Content | State |
|---|---|---|
| M0 | Daemon skeleton, schema, JSON-RPC socket, CLI | **done** |
| M1 | fanotify watcher, metadata index, episodes, trigram path search | next |
| M2 | Extraction, chunking, embeddings, hybrid search | |
| M3 | Agent Surface: MCP, policy engine, undo journal | |
| M4 | Search overlay (first GUI) | |
| M5 | Salience, decay, tiering, consolidation | |
| M6 | The full file manager | |
| M7 | Federation with a remote memory | |
| M8 | Brain-state visualisation | |
| M9 | Packaging and release | |

Details, including the executable definition of done for each milestone, are in
`docs/01-BUILD-PROMPT.md`.

## Building

Requires Rust 1.85 or newer.

```sh
git clone <this repository>
cd hippocampus
./scripts/setup-dev.sh      # installs the git hooks
cargo test --workspace
cargo build --release
```

## Trying it

```sh
# Start the daemon (use --root to keep everything in one throwaway directory)
./target/release/cortexd --root /tmp/hippocampus-demo &

# Index something, metadata only
./target/release/cortex --root /tmp/hippocampus-demo index ~/Documents

# Search it
./target/release/cortex --root /tmp/hippocampus-demo search report

# See what it knows
./target/release/cortex --root /tmp/hippocampus-demo status
```

Every command takes `--json` for scripting and for agent use.

Without `--root`, the daemon uses the XDG locations: configuration in
`~/.config/hippocampus/`, index in `~/.local/state/hippocampus/`, socket in
`$XDG_RUNTIME_DIR/hippocampus/`.

## What it deliberately does not do

- **Index your secrets.** `~/.ssh`, `~/.gnupg`, `.env` files, keys, browser profiles and
  password stores are excluded by a deny-list that configuration cannot switch off.
- **Read file contents by default.** Metadata indexing may cover your whole home directory;
  content extraction only happens inside roots you explicitly configure.
- **Talk to the network.** Not until you turn something on.

The full set is in `docs/05-HARD-RULES.md`.

## Contributing

Read `CLAUDE.md` first — it is the entry point for anyone (or anything) working in this
repository, and it explains the documentation layout and the rules that are not negotiable.

## License

Not yet chosen. See decision D13 in `docs/02-DECISIONS.md`. Until then, all rights reserved.
