# The search overlay

The first GUI onto the memory daemon: a launcher, not a file manager.

```sh
npm install
npm run tauri dev        # the window, against the daemon on the standard socket
```

The daemon has to be running (`cortexd`). This is a client; it holds no index of its own.

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
| `npx playwright test` | layout at 400–1400 px, the keyboard checklist, render latency |
| `cargo test -p hippocampus-ui` | the bridge, against a real `cortexd` |

The Playwright suite runs against `harness.html`, which is the app wired to a fixture instead of
a daemon. It reuses a Chrome for Testing already on the machine rather than downloading its own.

## Two rules that are not negotiable

**Every list is virtualised** and **the frontend does no data processing**. Both are conditions
D4 attached to using a web stack at all, and both are asserted by tests rather than reviewed:
`src/test/architecture.test.ts` will fail on a new runtime dependency, on a `filter` or a `sort`
anywhere in `src/`, on a second import of the Tauri API, and on a hex colour outside
`tokens.css`.
