# The search overlay

The first GUI onto the memory daemon: a launcher, not a file manager.

```sh
npm install
npm run tauri dev                   # the window, with hot reload
npm run tauri build -- --no-bundle  # a standalone binary in ../target/release/hippocampus
```

The daemon has to be running (`cortexd`). This is a client; it holds no index of its own.

**The overlay is resident.** It starts hidden and is summoned with `ctrl+alt+space`; Escape
clears the query, then puts it away. `--show` presents the window at startup, which is what you
want when running it by hand. Closing it hides it — Tauri exits with its last window, and a
launcher that quits when the compositor asks it to close has stopped being resident without
saying so.

Why resident: the cold start is 418 ms against a 300 ms budget, and 316 ms of that is the
toolkit rather than this code (D41). Paying it once at login is the mitigation;
`scripts/install-user.sh` sets up the systemd user units that do it.

**Build it through the Tauri CLI, not with `cargo`.** `cargo build --release -p hippocampus-ui`
produces a binary that compiles, starts, and then shows *"Could not connect to localhost"*:
`tauri-build` decides dev-versus-production from whether the Tauri CLI is driving the build
rather than from the cargo profile, so a plain `cargo build` embeds the dev server's URL instead
of the frontend. `cargo test -p hippocampus-ui` is unaffected — those tests never open a window.

## Layout

```
src/            React. Renders what the daemon sends and works out nothing of its own (D4).
  ipc.ts        the only door to the daemon — every component reads from here
  tokens.css    docs/06-DESIGN-SYSTEM.md, as values
  motion.ts     the named springs from §5 of that document
src-tauri/      Rust. The socket, the window, the hotkey, and opening a file.
  bridge.rs     a split connection: calls pipeline, notifications become window events (D38)
tests/          Playwright, against a real layout engine
```

## Tests

| Command | What it covers |
|---|---|
| `npx vitest run` | the windowing arithmetic, the rows, and the architectural rules |
| `npm run test:ui` | layout at 400–1400 px and the keyboard checklist, in parallel |
| `npm run test:perf` | render latency, alone and with one worker |
| `npm run test:e2e` | both of the above, in that order |
| `cargo test -p hippocampus-ui` | the bridge and the thumbnailer supervisor, against a real `cortexd` |

The latency tests run by themselves on purpose. Playwright's worker pool is per-invocation, so
a timing assertion sharing eight cores with seven other browser instances measures the cores —
it failed exactly that way once before being split out.

The Playwright suite runs against `harness.html`, which is the app wired to a fixture instead of
a daemon. It reuses a Chrome for Testing already on the machine rather than downloading its own.

## Two rules that are not negotiable

**Every list is virtualised** and **the frontend does no data processing**. Both are conditions
D4 attached to using a web stack at all, and both are asserted by tests rather than reviewed:
`src/test/architecture.test.ts` will fail on a new runtime dependency, on a `filter` or a `sort`
anywhere in `src/`, on a second import of the Tauri API, and on a hex colour outside
`tokens.css`.
