# 05 — Hard Rules

These are not guidelines. They belong in the first line of code, not in a later ticket.
A change that violates one of these is rejected regardless of how well it works.

## R1 — Deny-list applies before any content access

The classifier checks the deny-list **before** the extractor is allowed to open a file.
Not after, not "usually". The list is compiled in, extendable by config, never shrinkable
below the built-in minimum.

Built-in minimum (paths, relative to any indexed root):

```
.ssh/            .gnupg/          .aws/            .kube/           .docker/config.json
.mozilla/        .config/google-chrome/    .config/BraveSoftware/   .config/chromium/
.local/share/keyrings/            .password-store/  .var/app/*/config/
```

Built-in minimum (name patterns):

```
.env  .env.*  *.pem  *.key  *.p12  *.pfx  *.kdbx  *.keystore  *.jks
id_rsa*  id_ed25519*  id_ecdsa*  id_dsa*  *credentials*  *secret*  *token*
.netrc  .pgpass  .my.cnf  known_hosts  authorized_keys
```

Performance deny-list (indexed as metadata only, never extracted):

```
node_modules/  .git/objects/  target/  build/  dist/  .next/  .venv/  venv/
__pycache__/  .pytest_cache/  .cargo/registry/  .rustup/  .npm/  .cache/
*.o  *.a  *.so  *.dylib  *.rlib  *.pyc  *.class  *.wasm
```

## R2 — Ignore-file semantics

`.gitignore` semantics are respected inside git repositories. Additionally a `.brainignore`
file is honoured with the same syntax, at any level. A `.brainignore` containing `*` in a
directory means the directory is metadata-only, not that it is invisible.

## R3 — Opt-in for content extraction outside configured roots

Metadata indexing may cover the whole home directory. **Content extraction only happens inside
explicitly configured roots.** A file outside those roots is a path and a stat, nothing else.

## R4 — Index location and permissions

The index lives in `$XDG_STATE_HOME/hippocampus/` (default `~/.local/state/hippocampus/`).
Directory mode `0700`, files `0600`. Never in the user's document folders, never in `/tmp`.

## R5 — No network by default

Fresh install performs zero outbound connections. No telemetry, no update check, no model
download without an explicit user action. Cloud model endpoints and federation are off until
configured. This is verified by a test that runs the daemon under a network namespace with
no route and asserts full local function.

## R6 — Agent writes are gated

Agents get read access to files and write access to the brain. Filesystem mutations go through
the policy engine: allow-list scoping, mandatory dry-run returning a plan, an episode with
actor attribution per mutation, and an undo journal. See `docs/01-BUILD-PROMPT.md` §M4.

## R7 — Kill switch

A visible, always-reachable control that (a) pauses indexing and (b) disables the Agent Surface.
Reachable from CLI (`cortex pause`, `cortex agent-surface off`) and from the GUI. State survives
a daemon restart.

## R8 — Audit log

Every agent access is logged with timestamp, agent identity, tool, arguments and result size.
Readable via `cortex audit`. The log is append-only and separate from `episodes`.

## R9 — Repo hygiene (this repository is public)

Forbidden in code, tests, fixtures, example configs, documentation **and commit history**:

- Hostnames, IP addresses, tailnet names, MAC addresses
- Usernames, real names, email addresses
- Absolute paths from a private system (`/home/<name>/...`)
- Any secret, token, key or password — including fake-looking ones in tests

Enforcement: a pre-commit hook running a secret scanner plus a custom pattern check, and the
same check as a CI gate. The hook is part of the repo (`scripts/pre-commit`) and installed by
`scripts/setup-dev.sh`.

Test fixtures use `/tmp`-based temp dirs created at runtime, never checked-in absolute paths.

## R10 — Reconstructability

`episodes` is the only table that is never mutated. Everything else must be derivable from the
filesystem plus `episodes`. Any feature that stores non-reconstructible state in the index is
rejected. `cortex --rebuild-index` is a tested command, not a hope.
