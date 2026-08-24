# 07 — The Agent Surface, tool by tool

What an agent can ask this daemon for, what it gets back, and — for the four that matter most —
when to reach for which. Written for whoever is wiring an agent to it, and for whoever is adding
a tool to it.

The same capabilities reach an agent three ways and there is one implementation behind them:
the Unix socket (JSON-RPC), the `cortex` CLI, and `cortex-mcp` over stdio. MCP names use
underscores where the wire uses dots — clients disagree about whether a dot is acceptable in a
tool name, and the mapping is one line in `cortex-mcp/src/tools.rs` against an argument with
every client forever.

```bash
claude mcp add hippocampus -- /path/to/cortex-mcp --actor claude
```

---

## Start here: the four that carry the weight

**`brain_context(task, budget_tokens)`** — the tool to reach for at the start of a task, before
you are stuck rather than after. **Read the text block; it is the pack.** The structured content
beside it carries the same items *without* their text — an id, an arm, a kind, a scope and a
basis each — so that you can report an outcome and argue with a memory without paying for its
words a second time. A pack that spent its budget four times over would be a pack that does not
have one. Everything else on this page exists so that what it returns can
be corrected, attributed or improved. It answers with a block fitted to a token budget:
preferences that always apply, what the human has open right now, the memories that bear on the
task, and the files those memories are about. Each item carries who wrote it, when, and which
arm found it.

**`brain_remember(text, kind, scope)`** — anything you worked out that would cost the next
session the same effort. A `fact`, a `preference`, a `procedure`, a `decision`, a `failure`, an
`observation`. Say which; `observation` is the default *and* the kind that decays first.

**`brain_outcome(pack_id, helped[], noise[])`** — the one that is easy to skip and shouldn't be.
Naming two memories as useful in the same report also associates them, which is how this memory
learns that two things belong to the same kind of problem.
It is the only signal that teaches this memory what is worth keeping, and it has a property no
recency-and-frequency model has: **a memory that keeps arriving in your context and is never
reported useful decays faster than one that is never retrieved at all.** Reporting honestly that
something was noise is worth more than reporting that everything helped.

**`brain_search(query)`** — when you are looking for a *file* rather than for what is known. The
two are different questions and the second one is the one this system is unusual at.

---

## Reading the memory

| tool | what it answers |
|---|---|
| `brain_search(query, mode, limit)` | files, by meaning, by words, or by path |
| `brain_recall(cue, depth)` | pattern completion: a fragment, and what belongs with it — files, the graph around them, **and the memories the cue names** |
| `brain_neighbors(target, edge_types, depth)` | the associative graph around one node |
| `brain_timeline(from, to)` | what happened, in order |
| `brain_working_set()` | what the human is doing right now, with the basis it was derived from |
| `brain_explain(query, target)` | why something ranked where it did — or, for a memory, what it has earned and whether it is still believed |
| `brain_diff(since)` | everything that changed since a cursor. **Exists so that you do not poll** |
| `brain_graph(limit)` | the whole brain, for something that draws it. The one call documented as large |

## The agent's own memory

| tool | what it does |
|---|---|
| `brain_context(task, budget_tokens, kinds, session_id, project_paths)` | a context pack |
| `brain_remember(text, kind, scope, about, confidence, supersedes, ttl_secs)` | write a memory |
| `brain_outcome(pack_id, helped, noise, note)` | report what a pack was worth |
| `brain_memories(kinds, about, include_superseded, limit)` | list what you can see |
| `brain_supersede(old, new)` | one memory replaces another |
| `brain_forget_memory(memory_id)` | delete one outright |
| `agent_session_begin(name, task)` / `agent_session_end(session_id)` | attribution for one piece of work |

**Three calls answer even with the kill switch off**, because they observe the agent layer
rather than act for one: `agent_status`, `audit_recent` and `agent_sessions`. That is the line —
reading about agents is not agency — and it is drawn there because switching the surface off is
exactly the moment somebody wants to look at what happened before they did. Everything that
writes, including `session_begin` and `session_end`, refuses.

### Scope, in one paragraph

Every memory belongs to a scope: `session` expires with the task, `agent` is private to one
identity and is the default, `project` is shared inside a zone, `global` is everyone and is never
a default. **You name the *kind* of scope you want and the daemon resolves it against your own
identity** — you cannot name another agent's scope, correctly spelled or not. Writing to `global`
is a deliberate act and is called out in the audit log as one.

### What to remember, and what not to

Worth remembering: a build flag that is not obvious, an approach that was tried and failed, a
convention the code does not state, a preference the human expressed once, the reason a decision
went the way it did.

A memory is a sentence or two — **1 400 characters at most**, which is what fits inside the
embedding model's 512 tokens. Anything longer is a document: write it to a file and remember one
sentence about where it is. The limit is enforced rather than truncated, because a memory stored
whole and embedded from its first part only would have a vector describing the beginning of
something whose text says something else, and nothing anywhere would fail.

Not worth remembering: anything a file already says — the file is indexed, and a memory
restating it costs a slot in every future pack to tell you something `brain_search` would have
found. Nor anything true only for the next ten minutes; that is what `session` scope is for.

When you write something close to what is already there, the answer names the existing memories
rather than merging them. **Deciding whether two statements contradict each other is your job**
— this daemon has no language model in it, on purpose — so either supersede the old one or leave
both.

## Files

| tool | gated |
|---|---|
| `fs_list(path, glob, depth, sort)` · `fs_read(path, byte_range)` · `fs_stat(path)` · `fs_places()` | no |
| `fs_write(path, content, mode)` · `fs_move` · `fs_trash` · `fs_mkdir` · `fs_rename` | **yes** |
| `undo(last, plan, dry_run)` | no — reversing is always allowed |

**Every mutation is planned before it happens.** A gated tool called without a plan id changes
nothing and answers with what it *would* do; call again with that plan's id to apply it. A plan
carries a fingerprint of every target, expires in two minutes, is consumed on use, and cannot be
spent on a file it does not describe. Writes go only inside an allow-list, which is empty by
default.

---

## For whoever adds a tool

Three rules, and the first two are what keep the three surfaces from drifting apart.

1. **The tool lives in `cortex-agent`, not in the bridge.** `cortex-mcp` is a table mapping one
   name to one method plus stdio framing (D36). A bridge that interpreted anything would be a
   way round the policy engine, whose entire job is to be unavoidable.
2. **Gate first, then act, then audit** — in that order, with the caller's own arguments, so
   that a refusal records what was asked for. `Surface::gate_args` is the door.
3. **If it writes something that is not derivable from the filesystem, write the episode first.**
   The row is a projection of it. This is rule R10 and it is the one that is easy to get wrong:
   an agent's annotation, a memory, a deletion and an outcome report are all assertions, and a
   feature that writes the projection without the assertion has made the index un-rebuildable.
   The failure is silent until somebody runs `cortex rebuild`, and then it is total.

The reference for what "not derivable" costs in practice is `crates/cortex-store/src/memories.rs`
and D74; the tests that hold it are `the_whole_memory_layer_survives_a_rebuild` and
`a_thousand_memories_survive_a_rebuild`.
