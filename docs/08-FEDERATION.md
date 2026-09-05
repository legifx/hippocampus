# 08 — Federation: the wire protocol

What leaves this machine, in what shape, through what, and what comes back. Written for whoever
builds the receiving end — a bridge in the private Extension, or a peer in any language — and
for whoever wonders whether anything leaves at all. (It does not, until configured: R5.)

The implementation is `crates/cortex-sync`; the reference peer is `hc-test-peer` in the same
crate, and `crates/cortex-sync/tests/federation.rs` is the Definition of Done run against it.
The reasoning is D11, D17, D18, D19 and D93–D95.

---

## The shape in one paragraph

The Cortex keeps an **outbox**: one row per event, each with a stable identifier and a Lamport
tick. A worker drains it in **batches** of at most `batch_size` events, no closer together than
`batch_interval_ms`, in clock order, through a **transport** that knows how to reach the peer.
The peer answers with an **ack** naming what it accepted and what it refused. Accepted rows are
marked; refused rows count an attempt; rows the ack is silent about are sent again. When the
peer cannot be reached, nothing is counted against any row and the worker backs off, doubling
up to `max_backoff_ms`. Offline is the normal case.

Two one-way streets: events go up; an **ontology** — the peer's taxonomy — may come down inside
an ack, and the Core stores the latest one and shows it. The Core interprets nothing in it.

---

## Where events come from

Nothing is authored for the peer. Every event is **derived** from a row the index already has,
and can be derived again with the same identifier (R10):

| source | identity | payload |
|---|---|---|
| a row in `episodes` | `episode:<id>` | the action, the actor, the session, the path of the node it was about (if any), and the episode's own metadata — for a memory that is the memory itself: scope, kind, text, confidence |
| a concept node | `concept:<label>\0<member>\0<member>…` | the label, the member paths, the member count |

An event id is `blake3(origin ‖ "\0" ‖ identity)[..32]`, where **origin** is a 32-hex-character
identifier minted once per index at random (`sync.origin` in `meta`). Two machines remembering
the same sentence produce two different events; one machine remembering it twice produces one.
The id never depends on the payload, so a resend after a payload gained a field is still the
same event (D18).

**What stays home.** Searches — what somebody typed — never leave. File activity (create,
modify, open, move, delete) leaves only while `send_file_activity` is true (the default: it is
metadata). Chunk text leaves only while `send_chunk_text` is true (default off). Vectors never
leave (D19). Nothing under the deny-list was ever indexed, so nothing under it can be here.

**Order.** Events are enqueued in episode order and sent in Lamport order, which is the same
order. A replay (`cortex sync replay`) clears the outbox and harvests again from the first
episode: the same identifiers in the same order, on fresh ticks that sort after everything the
peer has already seen.

---

## The batch

One JSON object. The version is bumped when a field changes meaning; adding an optional field
does not bump it.

```json
{
  "protocol": 1,
  "origin": "adea5f1656ac35947a98a0c283cff0d0",
  "lamport": 1204,
  "sent_at": 1725000000,
  "events": [
    {
      "event_id": "ae6213a25108e2920b470456b3f6d18d",
      "lamport": 1203,
      "origin": "adea5f1656ac35947a98a0c283cff0d0",
      "ts": 1724999500,
      "kind": "episode",
      "payload": {
        "action": "remember",
        "actor": "agent:claude",
        "session_id": "s-…",
        "metadata": { "scope": "global", "kind": "fact", "text": "…", "confidence": 0.8 }
      }
    },
    {
      "event_id": "…",
      "lamport": 1204,
      "origin": "…",
      "ts": 1724990000,
      "kind": "concept",
      "payload": { "label": "invoices", "members": ["/home/user/…"], "member_count": 3 }
    }
  ]
}
```

| field | meaning |
|---|---|
| `protocol` | `1` |
| `origin` | which index sent it |
| `lamport` | the sender's clock at sending; at least the highest `lamport` in `events` |
| `sent_at` | seconds since the epoch, on the sender |
| `events[].lamport` | strictly increasing per origin; gaps are normal and are not filled |
| `events[].ts` | when the thing happened, not when it was sent |
| `events[].kind` | `episode` or `concept`; a peer ignores kinds it does not know |

---

## The ack

```json
{
  "accepted": ["ae6213a25108e2920b470456b3f6d18d"],
  "rejected": [{ "event_id": "…", "reason": "no such scope" }],
  "lamport": 9000,
  "quiet_for_ms": 3600000,
  "ontology": { "version": 3, "tags": [{ "name": "finance", "parent": null, "description": null }] }
}
```

Every field is optional; `{}` is a valid ack and means *nothing was delivered*.

| field | what the sender does with it |
|---|---|
| `accepted` | marks the row acknowledged; it is never sent again |
| `rejected` | one attempt against the row, with the reason kept; after 100 the row is dead-lettered and stays in the table for a human to look at |
| *(neither)* | the row is sent again in the next batch — silence is not consent |
| `lamport` | moves the sender's clock past it, never back (D11) |
| `quiet_for_ms` | a floor on the gap before the next batch. Backpressure is a protocol feature, not a deployment setting (D18): a peer in its nightly batch job says so here instead of dropping events |
| `ontology` | stored whole as the latest; `sync.status` shows its version |

An id in `accepted` that was not in the batch is ignored.

---

## Transports

The protocol does not care how the bytes travel (D17). The Core ships two ways that need no
address, and a third that is the Extension's.

### `file-drop`

```toml
[federation]
enabled = true
transport = "file-drop"
drop_dir = "/mnt/share/hippocampus-inbox"
```

One document per event, named `<event_id>.json`, written to a temporary name and renamed into
place. A resend overwrites the same name, so a peer that deduplicates files by name gets it
for free. **The directory is never created**: if it is not there, the share is not mounted and
the peer is unreachable — creating it would write the peer's mail onto the local disk where the
peer will never look. The peer may leave `ontology.json` in the same directory; it is read on
every push and never removed.

`hc-test-peer ingest --state peer.json --dir <drop_dir>` is what a consuming peer does: read
every document, sort by `lamport`, absorb, delete.

### `plugin`

```toml
[federation]
enabled = true
transport = "plugin"
plugin = "main-brain"          # → $XDG_CONFIG_HOME/hippocampus/plugins/main-brain.toml
```

```toml
# plugins/main-brain.toml
kind = "sync"
command = "/opt/hippocampus-ext/bin/bridge"
args = ["--profile", "home"]
timeout_ms = 30000
```

One process per batch. The Core writes one JSON-RPC request to the plugin's stdin and reads one
response line from its stdout:

```json
{"jsonrpc": "2.0", "id": 1, "method": "sync.push", "params": <batch>}
{"jsonrpc": "2.0", "id": 1, "result": <ack>}
```

A `result` is an ack. An `error` is a refusal of the whole batch (every event counts an
attempt). No answer, a non-zero exit without an answer, a command that cannot be started, or
the deadline passing — the process is killed — all mean *unreachable*: nothing is counted and
the worker backs off.

**What the plugin does with the batch is not the Core's concern and is not visible to it.** The
declaration names a command; the endpoint, the schema and the credentials live with the plugin.
`sync.status` shows `plugin:<name>` and nothing more (D9, D95).

`hc-test-peer push --state peer.json` is a complete plugin, and the shape to copy.

### The Extension

The private repository ships the bridge as a `sync` plugin: a program that takes the batch
above and talks to a concrete Main Brain in whatever way that brain wants. It may batch further,
translate, authenticate, or hold a connection open in a daemon of its own. The Core never
learns which of those it does.

---

## Backpressure and the offline case

| setting | default | meaning |
|---|---|---|
| `batch_size` | 500 | at most this many events per push |
| `batch_interval_ms` | 1000 | at least this long between pushes |
| `max_backoff_ms` | 300000 | the ceiling on the doubling wait after an unreachable peer |
| `peer` | `"peer"` | a label for the peer's sync state; not an address |

Three days offline at one event every thirty seconds is 8 640 events. On reconnection they go
out in eighteen batches over eighteen seconds, in order, and every one of them arrives exactly
once — asserted against the reference peer in `tests/federation.rs`, with a synthetic clock so
the three days take six seconds.

The outbox is a cache. Acknowledged episode events are pruned after a week; acknowledged
concept events are kept, because a concept is harvested by identity and the acknowledged row is
what stops it being sent again (D94).

---

## What you type

```bash
cortex sync status     # is it syncing, and if not, why not — in words, then numbers
cortex sync now        # send at once, interval and backoff set aside
cortex sync replay     # start the whole log over; same identifiers, a peer that keeps them sees nothing new
cortex status          # `outbox N pending` is the same number
```

Over the socket: `sync.status`, `sync.now`, `sync.replay`. None of them is an agent tool.
