# 09 — Installing, configuring, and what to do when it does not work

For the second user: the one who has never seen this repository, has no access to any private
one, and wants a working search in under ten minutes (M9). Read top to bottom the first time;
the configuration reference and the troubleshooting section are for coming back to.

---

## 1. Install

Three ways. All of them put five binaries somewhere and two systemd *user* units beside them;
none of them starts anything on your behalf (R5).

**The Debian package.**

```sh
sudo apt install ./hippocampus_<version>_amd64.deb
cortex setup
```

Built from a checkout with `packaging/deb/build.sh`; needs only `dpkg-deb`. Runtime dependencies
are GTK 3 and WebKitGTK 4.1 for the overlay; `poppler-utils`, `ffmpeg`, `udisks2` and
`wl-clipboard` are recommended and each unlocks one thing (PDF previews, video thumbnails,
mounting drives, the clipboard) — without them that thing is absent and says so.

**Per user, from source.** Nothing outside your home directory.

```sh
git clone <this repository> && cd hippocampus
./scripts/setup-dev.sh          # the git hooks, including the hygiene gate
./scripts/install-user.sh       # builds, installs into ~/.local/bin and ~/.config/systemd/user
cortex setup
```

Requires Rust 1.85 or newer, Node 20 or newer for the overlay, and the WebKitGTK 4.1
development headers. `scripts/uninstall-user.sh` reverses it and leaves your index alone.

**The AppImage.** `packaging/appimage/build.sh` produces an AppDir and, if `appimagetool` is on
your path, an AppImage that carries the daemon, the CLI and the overlay. It uses the system's
WebKitGTK rather than bundling it. Run `Hippocampus.AppImage` for the overlay and
`Hippocampus.AppImage cortex …` for the CLI.

## 2. Set up

```sh
cortex setup
```

Three questions, each with a default you can take with Enter:

1. **Which directories to index** — paths and stats only. Default: your home directory.
2. **Which directories to read** — the contents of files, for search by meaning. Default:
   `~/Documents`, `~/Projects` and `~/Notes`, whichever exist. Nothing outside these is ever
   opened (hard rule R3), and the deny-list applies inside them and cannot be shrunk (R1).
3. **Whether to download the embedding model** — about 120 MB, the only network access this
   software ever makes, and only on this answer. `none` leaves you with path and word search,
   which is most of the value; `cpu` or `gpu` adds meaning.

It writes `~/.config/hippocampus/config.toml`, enables and starts the daemon as a user service,
waits for it, indexes the directories from question 1, and tells you how long that took. On
a home directory of a hundred thousand files that is a minute or two. Content is read
afterwards, in the background, at the rate the machine has to spare; `cortex extract` shows
how far it has got.

`cortex setup --yes` takes every default without asking; `--metadata-roots`, `--content-roots`
and `--model` answer the questions from the command line; `--force` replaces an existing
configuration.

Then:

```sh
cortex search invoice                          # a word, a path fragment, or a meaning
cortex explain invoice ~/Documents/x.pdf       # why it ranked there, or why it did not
systemctl --user enable --now hippocampus-overlay.service   # the window, on Ctrl+Alt+Space
```

## 3. Running without a second brain

This is the ordinary configuration, not a degraded one. Federation is off unless
`[federation] enabled = true` is in the configuration, and with it off the daemon queues
nothing, opens nothing, and `cortex sync status` says so in one sentence. Every test in the
repository runs this way; `crates/cortexd/tests/federation.rs` asserts that the daemon starts
and serves normally when federation is on and its bridge is missing, which is the other half
of the same promise (D10). What federation is, when you want it, is `08-FEDERATION.md`.

## 4. Configuration reference

Three layers: the built-in defaults, `~/.config/hippocampus/config.toml`, and anything a plugin
brings in `~/.config/hippocampus/plugins/*.toml`. Unknown keys are refused at startup so a typo
cannot pass as an option. `cortexd --check` prints the effective configuration and exits.

State lives in `~/.local/state/hippocampus/` (mode 0700): the index `cortex.db`, the vector
indexes, downloaded models. The socket is `$XDG_RUNTIME_DIR/hippocampus/cortex.sock`.

```toml
[index]
metadata_roots = []            # directories indexed for paths and stats
content_roots = []             # directories whose file contents may be read (R3)
extra_deny = []                # glob patterns added to the built-in deny-list; it never shrinks
debounce_ms = 500              # how long a burst of filesystem events is allowed to settle
reconcile_interval_secs = 3600 # full comparison of index against filesystem
respect_gitignore = true       # .gitignore and .brainignore are honoured (R2)
watch_budget = 20000           # inotify watches the daemon may hold; beyond it, degraded and said so
zone_budget = 64               # how many spatial zones (coloured areas) are detected

[search]
default_limit = 50
candidates_per_arm = 100       # per retrieval arm before fusion
rrf_k = 60.0                   # reciprocal-rank-fusion constant
max_salience_boost = 1.5       # how much a well-used file may rise
rerank = true                  # cross-encoder reordering after results are shown, if the model is there
rerank_top_n = 30
reranker_model = "mmarco-mMiniLMv2-L12-H384-v1"

[embedding]
model = "multilingual-e5-small"
enabled = true                 # false: never embed, even with a model present
device = "auto"                # auto | cpu | cuda
threads = 8
batch_size = 16
gpu_memory_mb = 1024           # the model's share of a GPU other things also use (D31)
# runtime_path = "..."         # a specific ONNX runtime library, if not the fetched one

[resources]
cpu_share_active = 0.15        # of the machine, while you are using it (D22)
cpu_share_idle = 0.8           # while you are not
idle_after_secs = 120
memory_budget_mb = 250         # the daemon sheds caches above this
nice_level = 10

[agent]
enabled = true                 # the Agent Surface; `cortex agent-surface off` is the runtime switch (R7)
write_allowlist = []           # where an agent may change files at all; empty means nowhere
require_dry_run = true         # every mutation is planned first, then confirmed by plan id
undo_journal_depth = 1000

[federation]
enabled = false
transport = "none"             # none | file-drop | plugin
peer = "peer"                  # a label, never an address
# drop_dir = "..."             # for file-drop
# plugin = "..."               # for plugin: plugins/<name>.toml
batch_size = 500
batch_interval_ms = 1000
max_backoff_ms = 300000
send_chunk_text = false        # file content never leaves unless this is true
send_file_activity = true      # which files were touched (metadata) does, unless this is false

[log]
filter = "info"                # a `tracing` filter, e.g. "cortexd=debug"
```

## 5. Troubleshooting

**"no daemon listening".** `systemctl --user status hippocampus-cortexd.service`, then
`journalctl --user -u hippocampus-cortexd -n 50`. The daemon refuses to start twice against one
index (the lock file in the runtime directory) and refuses an index written by a newer version;
both say so in the journal.

**Search finds nothing.** `cortex status` — are `files` and `dirs` non-zero? If not, the
directories from `metadata_roots` were not indexed: `cortex index <dir>` does it now.
`cortex explain <query> <path>` says why a specific file did or did not appear, arm by arm.

**Search by meaning finds nothing.** `cortex model` — is there a model? `cortex extract` — has
content been read? Extraction runs in the background at the configured CPU share and needs the
file to be under a content root. Without a model, `explain` says the semantic arm is absent.

**The watcher says DEGRADED.** More directories than `watch_budget` allows. Raise it and
`sysctl fs.inotify.max_user_watches`, or narrow `metadata_roots`. Until then the hourly
reconciliation catches what the watcher misses.

**Something is slow.** Not by reasoning about it. The overlay logs any daemon call over 50 ms:

```sh
journalctl --user -u hippocampus-overlay --since "-1d" -o cat | grep -E "slow call|slow preview"
```

Equal durations across different calls finishing together are a queue, not slowness. And
measure what the processes actually do before believing either — `03-PROGRESS.md` has the
cgroup snippet, and the note about a measurement that read zero for a process spinning a core.

**The overlay shows glitches or black areas.** A known pairing of WebKitGTK's DMABUF renderer
with some NVIDIA drivers. `systemctl --user edit hippocampus-overlay.service` and add
`Environment=WEBKIT_DISABLE_DMABUF_RENDERER=1` under `[Service]`. It costs nothing measurable
(D59).

**The overlay does not appear on Ctrl+Alt+Space.** The compositor may own the combination;
bind `hippocampus --show` in it instead. On niri the window is a layer surface and needs no
special rule.

**An agent is refused.** `cortex agent-surface` shows the switch; `cortex audit` shows what was
asked and why it was refused. A gated tool without a plan id is refused by design (R6).

**Start over.** `cortex rebuild` derives the index from the filesystem and the episode log
again (R10). Deleting `~/.local/state/hippocampus/` loses the episodes — what was opened when,
and everything agents remembered — and nothing else.

## 6. Limitations, honestly

- **Linux, Wayland, one compositor tested.** The overlay is built for niri; it runs elsewhere,
  and the hotkey and the layer-shell behaviour are only verified there.
- **The overlay is WebKitGTK.** Software rendering on some driver pairings; the 3D view of the
  brain is capped at thirty frames a second inside the window for that reason (D92).
- **Content extraction covers text, code, PDF, and office formats through their text.** Images
  are thumbnailed, not described; audio and video are not transcribed. Files outside content
  roots are a path and a stat.
- **Search by meaning needs a model download**, and the model is small and multilingual rather
  than large and English. It is chosen for latency on a shared GPU, not for the best possible
  ranking.
- **Network locations are offered when mounted, not mounted for you** (D84). Mounting SMB or
  SFTP is the desktop's job today.
- **Federation ships a protocol and two transports, not a peer.** A real second brain needs a
  bridge, written as a plugin, outside this repository.
- **The learning signal needs use.** Salience, tiering and the agents' memories are only as
  good as the history behind them; a fresh index has a flat legend and says so.
- **It has been used daily by one person.** M6's gate — a week of exclusive use — and M9's —
  a second person installing it unaided — are recorded in `03-PROGRESS.md` when they happen.
