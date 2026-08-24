# Forgetting Is the Feature

### A filesystem-native memory architecture for humans and agents, and an argument that the field is optimising the wrong objective

**Status of this document.** This is a design paper with a measured implementation behind it, not
a peer-reviewed result. Every number reported in §9 was produced by the system described here,
on one machine, in one session, and is reproducible from the repository. Every claim that is *not*
measured is marked as an argument or a hypothesis. The comparison in §12 is against published
descriptions of other systems, not against re-implementations of them on a shared benchmark —
that work does not exist yet and §13 says so plainly. A reader looking for a benchmark table
proving superiority will not find one here; a reader looking for an architecture, its
justification, and an honest account of what it cost to build will.

---

## Abstract

Systems that give language models long-term memory almost universally optimise for **retention**:
store more, embed more, retrieve more. We argue this is the wrong objective, and that it produces
a characteristic failure eighteen months in — an index that is large, slow, and full of material
its owner no longer trusts, which is the point at which people quietly stop using it.

We describe **Hippocampus**, a memory daemon whose distinguishing property is that it *forgets*,
and whose forgetting is safe because the index is a **cache** rather than a store: everything is
reconstructible from the filesystem plus an append-only episode log. Salience is a bounded,
weighted sum of seven named components that always explains itself; it decays; nodes fall through
four tiers rather than being deleted; associations are formed from co-occurrence in time rather
than from content similarity alone; and pins are an exemption from the machinery rather than a
large number inside it.

For the half of the memory an *agent* writes — what it learned that is not in any file — we add
one component and one argument. The component is what a memory has earned; the argument is that
**retrieval is not evidence of usefulness**, because a memory that appears in a context window
was chosen by the ranking under evaluation rather than by anybody, so counting its appearance in
its favour closes the loop that the evaluation was supposed to open. The consequence is
counter-intuitive and, we think, correct: a memory that keeps arriving and is never reported
useful should decay faster than one that is never retrieved at all.

We ground the design in three literatures that are rarely brought together: the **rational
analysis of human memory**, where the probability that a memory will be needed follows a power law
in recency and frequency (Anderson & Schooler, 1991); **systems consolidation and replay**, where a
fast-learning hippocampal store is gradually distilled into slow-learning cortex (McClelland,
McNaughton & O'Reilly, 1995); and **personal information management**, where three decades of
studies find that people locate their own files by orienteering through context rather than by
recalling names (Barreau & Nardi, 1995; Teevan et al., 2004).

We then argue, from the mechanics of transformer inference rather than from analogy, why a memory
for language models must be a **selection** system and not a storage system: attention cost is
quadratic in sequence length, effective context is measurably shorter than nominal context,
position within the prompt changes retrieval accuracy, and prompt caching makes *where* a memory
injects its output an architectural decision with a direct cost consequence.

Finally we report what building it actually cost, including the defects that no amount of
reasoning found and that only real data, a real clock and a real test suite exposed. We consider
that section the most useful part of the paper.

---

## 1. The accumulation trap

Every personal-knowledge system begins the same way. Capture is easy, so capture is what gets
built. The index grows monotonically. For the first months this is indistinguishable from success:
everything you look for is there.

Then three things happen at once, and they compound.

**Precision falls faster than recall rises.** The index now contains many documents that are
plausibly related to any query. Retrieval returns ten of them. Nine are irrelevant, but each was
plausibly relevant, so the user must read all ten to find out. The cost of a query has risen while
its value has not.

**Staleness becomes indistinguishable from currency.** A note from three years ago and a note from
Tuesday look identical to a cosine similarity. The system has no opinion about which one you
probably meant, because it has no notion of which one you have been *using*.

**Trust collapses, and it collapses non-linearly.** A user who is burned twice by a confidently
retrieved stale document stops treating retrieval as authoritative. From that point the system is
a search box they occasionally try, not a memory they rely on. Nothing in the index has changed;
what changed is the user's prior.

The literature on personal information management has described the endpoint of this for decades.
Whittaker (2011) frames the whole field as a shift from *consumption* to *curation* precisely
because keeping everything stopped working. Whittaker & Sidner's (1996) study of email overload
found people maintaining elaborate manual strategies to compensate for archives they could no
longer navigate. The pattern is not new. What is new is that we have automated the accumulation
and not the curation.

**The thesis of this paper:** a memory system's core competence is deciding what *not* to return,
and that competence must be visible, explainable and reversible. Everything else in this
architecture follows from that sentence.

---

## 2. What the current niche gets wrong

We are describing a category, not attacking particular projects. The systems named here are
serious work and several are cited approvingly later. The point is that a set of shared
assumptions produces a shared set of failures.

### 2.1 The vector store as a memory

The default architecture for LLM memory is: embed everything, store the vectors, retrieve top-k by
cosine similarity, put the results in the prompt. This descends directly from retrieval-augmented
generation (Lewis et al., 2020) and dense passage retrieval (Karpukhin et al., 2020), where it is
appropriate — those systems retrieve from a *corpus* that is not the user's own working history.

Applied to personal memory it inherits four structural properties that are wrong for the job:

**It is unranked by importance.** Cosine similarity measures topical relatedness. It has no
channel for "this is the file you have had open all week". Two documents about the same subject,
one of which you wrote yesterday and one of which you skimmed in 2021, are equally good answers.

**It is monotone.** Nothing in the architecture removes anything. Growth is unbounded by design,
and the quality degradation described in §1 is not a bug in any component — it is the sum of
correct components.

**It is opaque.** "Why did this rank third?" has no answer beyond a float. This matters more than
it appears: a user who cannot interrogate a ranking cannot correct it, and a system that cannot be
corrected cannot earn trust back once it has lost it.

**Its representation ages badly.** Embeddings are model-specific. Changing the embedding model
invalidates the store, so the system either re-embeds the entire corpus — expensive, and for many
people simply not done — or freezes on a model that gets worse relative to the state of the art
every quarter. A representation that cannot be regenerated cheaply is a liability, not an asset.

### 2.2 The agent-memory frameworks

The second generation is more sophisticated and more interesting. MemGPT (Packer et al., 2023)
treats the context window as virtual memory with paging between a small resident context and
external storage, an analogy we think is exactly right and return to in §3.1. Generative Agents
(Park et al., 2023) is the closest published relative of the scoring function in this paper: its
memory stream ranks retrievals by a weighted sum of recency, importance and relevance, with
recency as an exponential decay — the same three-ingredient recipe, arrived at independently and
for the same reason. A-MEM (Xu et al., 2025) organises memories as an interlinked Zettelkasten,
where new notes trigger updates to existing ones. Zep (Rasmussen et al., 2025) maintains a
temporal knowledge graph so that facts can be superseded rather than merely accumulated. Mem0
(Chhikara et al., 2025) extracts and consolidates salient facts from conversations and reports
substantial token savings from doing so.

These address the ranking problem and, in Zep's case, the staleness problem. Three assumptions
remain, and they are the ones this architecture rejects:

**The memory is the store.** In almost all of these, the memory *is* the authoritative copy of
what it knows. Losing it loses information. This forces conservative deletion policies, which
brings back accumulation through the back door: a system that cannot cheaply reconstruct itself
cannot afford to forget aggressively, so it does not.

**The unit of memory is the conversation.** These systems remember what was *said to the model*.
That is a small and peculiar slice of what a person actually works with. The files on the disk —
which is where the work is — are at best ingested into the memory, and once ingested, the copy
begins to diverge from the original the moment the original is edited.

**Memory is a service the agent calls.** The human cannot see it, cannot browse it, and is not the
one whose behaviour shapes it. If the memory has an opinion about what matters, that opinion was
formed from token traffic, not from what the human has been doing all week.

### 2.3 The consequence

A memory built on those three assumptions can be very good at recalling what was said to it, and
still be useless as *your* second brain — because your second brain's job is to know that the
three files you have had open since Tuesday are related to each other, that the design note you
argued about in March is relevant to the bug you are looking at now, and that the four thousand
files your build system wrote last night are not memories at all.

---

## 3. What a language model actually needs from a memory

This section argues from how inference works rather than from analogy to brains. The analogy comes
later, in §8, and it is used as a source of hypotheses, not of authority.

### 3.1 The context window is a budget, not a container

Self-attention is quadratic in sequence length (Vaswani et al., 2017). Doubling the retrieved
context roughly quadruples the attention cost of processing it and increases latency accordingly.
The practical consequence is that context is *rationed*, and a memory system's real output is not
"the relevant documents" but "the ranked subset that fits".

This reframes the design problem. A memory that returns twenty excellent results has not helped;
it has moved the selection problem into the prompt, where it is solved by a model that is worse at
it and charges for the attempt.

### 3.2 Effective context is shorter than nominal context, and position matters

Liu et al. (2023) demonstrated the "lost in the middle" effect: models retrieve information placed
at the beginning or end of a long context substantially more reliably than information in the
middle, and the degradation is large enough to matter for design. Long-context benchmarks such as
RULER (Hsieh et al., 2024) find that the length at which a model still performs the task reliably
is well below its advertised window.

Two design consequences follow, and both are architectural rather than cosmetic:

1. **Ranking must not stop at the retrieval boundary.** The order in which results are placed in
   the prompt is part of the retrieval system's output, not an afterthought for the caller.
2. **A hundred mediocre results are worse than five good ones**, not merely more expensive. The
   mediocre ones are in the middle of the window, where they dilute attention without contributing.

### 3.3 The prompt cache makes injection a structural decision

Providers cache the shared prefix of a prompt so that repeated calls in a session skip
recomputation. A memory system that injects freshly retrieved material *at the top* of the prompt
changes the prefix on every call and destroys the cache for the whole session. The same material
appended after the stable prefix costs a fraction as much.

This is a genuine architectural constraint that the memory literature rarely mentions: **where a
memory places its output determines the cost of every subsequent call in the session.** A memory
that maintains a "current context" block at the head of the system prompt is, in cost terms, an
expensive design regardless of how good its retrieval is.

### 3.4 A model cannot tell you what it was not shown

An LLM given three retrieved documents answers from three retrieved documents. It has no way to
know that the fourth — the one that would have changed the answer — was ranked eleventh. It will
not hedge, because nothing in its input indicates absence.

This puts the burden of calibration entirely on the retrieval system, and it is why
**explainability is not a nicety here but a safety property**. A ranking that can state its
components can be checked by the human who knows what is missing. A ranking that cannot is a
confident assertion with no error bars, delivered to a component that will treat it as fact.

### 3.5 Retrieval is ranking, and models improve ranking rather than producing results

The strongest retrieval systems remain hybrids: lexical scoring such as BM25 (Robertson &
Zaragoza, 2009) fused with dense retrieval, commonly by reciprocal rank fusion (Cormack et al.,
2009), and optionally reranked by a cross-encoder. Lexical retrieval finds the exact identifier
that embeddings smear; dense retrieval finds the paraphrase that lexical matching misses.

The relevant design rule: **the model belongs after the result set, not inside it.** A system in
which an LLM call is on the critical path of every search has a latency floor of hundreds of
milliseconds and a cost floor per keystroke, and both are unacceptable for something that competes
with `fzf`.

### 3.6 Tool results are the largest variable cost in an agent loop

In an agentic system, the tokens spent are dominated not by the model's output but by what tools
put into the context. A memory tool that returns a naive dump is the single most expensive
component in the loop. This is why the interface described in §5.3 returns summaries and handles
by default, why `brain.graph` — the one call that genuinely returns a large object — is documented
as needing a result gate, and why Mem0's reported token savings come primarily from *not returning
things*.

### 3.7 Retrieval is not evidence of usefulness

Every importance signal in the file half of this system rewards being touched: access count,
recency, degree. That is sound for files, because a person opening a file is a person finding it
useful — the act and the evidence are the same act.

It is unsound for an agent's memory, and the reason is a loop. A memory that appears in a context
pack was not chosen by anybody; it was chosen by the ranking under evaluation. Counting its
appearance as evidence of its value closes the loop: whatever ranked highly first goes on ranking
highly, and the system has no channel through which it could ever discover it was wrong. The
failure is invisible, because a self-confirming ranking looks exactly like a good one.

So the design needs a signal from outside the loop, and there is only one candidate: whether the
retrieved memory was *used*. **A memory that keeps arriving in context and is never reported
useful should decay faster than one that is never retrieved at all.** That is the
counter-intuitive consequence and it is the one that matters: a memory nobody asks for costs
nothing, and one that arrives in every pack and earns no mention is costing tokens on every turn
for nothing.

Two honesty constraints follow, and both are structural rather than advisory. An agent reporting
on its own success is an unreliable narrator, so the reporter is recorded and weighted — a human's
confirmation outranks an agent's self-report, which outranks silence — and a human may overrule
an agent on the same item, which is a different rule from "nobody may report twice" and has to be
written as one. And positive and negative reports are counted separately rather than netted,
because a memory that is useful in one context and noise in another needs splitting, and a net of
zero is exactly how that stays invisible.

### 3.8 An agent has a context window, not a result list

Every other retrieval interface in this system answers with rows. An agent cannot use rows. It
has a budget measured in tokens, and turning twenty results into two thousand tokens worth having
means dropping the restatements, deciding what fits, keeping the provenance and formatting the
rest — work that requires the scores and the vectors, which are on the daemon's side of the
boundary and not the agent's.

So the memory answers with a *pack*: budgeted, deduplicated, attributed, addressable. Four
properties, each present because omitting it is a known failure. The budget is in measured tokens
rather than characters over four, because that estimate is wrong by up to 40 % on code and on
German and the direction of the error decides between wasting money and truncating a system
prompt mid-structure. Preferences are included unconditionally, because they are tiny and their
entire value is being present. Diversification happens before truncation, because restatement is
the specific failure mode of embedding retrieval over a store that people and agents write to.
And every item carries its basis, because a memory an agent cannot attribute is a memory it
cannot argue with — and an agent that cannot argue with its memory will follow it off a cliff.

---

## 4. Seven design rules

Everything below follows from §§1–3. They are stated as rules because they were treated as rules:
each is enforced by a test, a schema constraint or a CI gate rather than by intention.

1. **The filesystem is ground truth; the index is a cache.** Nothing authoritative lives only in
   the index. `rm index && rebuild` must restore full function.
2. **One table is immutable.** The episode log is append-only and is never rewritten. Everything
   else is derivable from the filesystem plus that log.
3. **Forgetting is demotion, not deletion.** Four tiers. Falling out of the hot index costs
   nothing that cannot be recovered by rebuilding, which is what makes aggressive forgetting safe.
4. **Every score explains itself.** Salience is a sum of named components; the stored breakdown
   sums to the score actually used, and a test asserts it.
5. **A pin is an exemption, not a weight.** Explicit human intent is not a large number competing
   with inferences; it bypasses them.
6. **No model on the hot path.** Retrieval is lexical and vector fusion. Models improve ranking
   and perform consolidation asynchronously. The benchmark is `fzf`, not a chat window.
7. **No network by default.** Zero outbound connections on a fresh install. A second brain that
   phones home unasked is not one.

---

## 5. Architecture

### 5.1 Components

```
     filesystem events                     agents (MCP)      humans (CLI, GUI, Agent View)
            │                                    │                        │
            ▼                                    └──────────┬─────────────┘
   ┌──────────────────┐                                     ▼
   │  watcher         │                          ┌────────────────────────┐
   │  classifier      │  deny-list first         │  policy · kill switch  │
   │  extractor       │───────────────┐          │  audit · undo journal  │
   │  chunker         │               │          └───────────┬────────────┘
   └──────────────────┘               ▼                      ▼
                             ┌──────────────────────────────────────────┐
                             │  index (SQLite)                          │
                             │  nodes · edges · episodes · chunks · FTS │
                             │  vectors · salience_log · zones          │
                             └───────────────┬──────────────────────────┘
                                             │
                     ┌───────────────────────┴────────────────────────┐
                     ▼                                                ▼
            ┌────────────────┐                             ┌────────────────────┐
            │  retrieval     │  lexical + vector + path    │  consolidation     │
            │  RRF fusion    │  salience boost             │  replay→associate  │
            │  explain       │                             │  →score→tier→report│
            └────────────────┘                             └────────────────────┘
```

The daemon is headless. Every interface — command line, file manager, agent surface,
visualisation — is a client of the same JSON-RPC socket, which means no client can hold state the
others cannot see, and no client can compute an opinion the daemon does not have.

### 5.2 The data model, and why each part is shaped that way

**`nodes`** — files, directories, and later concepts. Carries `tier`, `salience`, `pinned`,
`zone_id`, and the kernel's `(dev, inode)` identity. That last pair is why a `mv` of a large
directory is recognised as a move rather than as a mass delete followed by a mass create, which
would destroy history and force a pointless re-index.

**`episodes`** — `(ts, actor, action, node_id, session_id, metadata)`. Append-only, never mutated.
`node_id` is deliberately **not** a foreign key: an episode about a deleted file must outlive the
file, because the log is the ground truth from which nodes are rebuilt. This decision is correct
and it caused a production defect; §10.1.

**`edges`** — typed and weighted: `contains`, `co_access`, `imports`, `similar_to`,
`derived_from`, `annotates`. Each carries a `source`: `observed`, `inferred`, or `explicit`. That
column exists so that a pruning pass which trusts only its own inferences can never delete a human
assertion.

**`zones`** — spatial regions of the filesystem, each with a stable hue. Zones are the unit of
"which project is this", and they are what makes a visualisation of a hundred thousand nodes
legible at all.

**`salience_log`** — the component breakdown per computation. This is what makes rule 4
enforceable rather than aspirational.

### 5.3 The agent surface

Agents get read access to files and write access to the brain. That asymmetry is the whole safety
model: an agent's note *about* a document cannot damage the document, and a brain an agent may
read but never write is a brain that learns nothing from the agent using it.

Filesystem mutations go through a policy engine with an allow-list, a mandatory dry run that
returns a plan, an episode per mutation attributed to the actor, and an undo journal that stores
the inverse operation *before* applying the forward one. Every access, allowed or refused, is
audited; the refusals are the interesting half.

---

## 6. The mathematics of forgetting

### 6.1 Salience

```
salience = w₁·access + w₂·recency + w₃·edits + w₄·centrality
         + w₅·pin + w₆·project − w₇·noise
```

The specification this was built from stated exactly that. Implementing it literally produces two
failures, and only one of them is visible by reading.

**Every component is bounded to [0,1] before weighting.** Without this the weights are not
comparable with each other — `w₁` would mean "per log-access" while `w₅` means "once" — and the
consequence is concrete: a file opened ten thousand times by a build script outranks everything a
human ever cared about. With bounding, a weight *is* the maximum contribution of its signal, and
the weight vector becomes a budget that a reader can interpret.

The bounding functions, and why each:

| component | unit form | why this shape |
|---|---|---|
| access | `ln(1+n) / ln(1+50)`, capped | logarithmic because the difference between 1 and 10 accesses matters and between 100 and 1000 does not |
| recency | `exp(−λ·Δt)`, half-life 14 d | see §6.2 |
| edits | `1 − exp(−e/3)` | saturating: three recent edits is "actively worked on"; thirty is not ten times more so |
| centrality | `degree / degree_p95`, capped | 95th percentile rather than max, so one hub cannot flatten everything else to zero |
| pin | `1` if pinned | see §6.4 |
| project | `1` if inside a recently touched zone | context, not content |
| noise | strongest matching pattern, `[0,1]` | subtracted; strongest wins rather than accumulating, so a vendored directory inside a cache does not score 2.0 |

### 6.2 The decay law, and the mistake worth reporting

The specification also contained, in a different paragraph:

```
salience(t) = base · e^(−λ·Δt) + Σ(reinforcements)
```

The first implementation treated these as two separate mechanisms: the weighted sum for scoring,
the decay law for "between recomputations". In a thirty-day simulation, **not one node ever
reached the Cold tier.** The cause is arithmetic and, in hindsight, obvious: `ln(1+access_count)`
is monotonically non-decreasing. A file opened once in 2019 holds a permanent floor of salience
for ever. Recency decayed; the floor did not; the floor was above the threshold.

The fix is that they are not two mechanisms. `access` and `edits` are **effective counts**: each
episode is a reinforcement of size 1, and the running total decays between episodes by the law
above. Concretely, folding the timestamps `t₁ < t₂ < … < tₙ` for a node:

```
c₀ = 0
cᵢ = cᵢ₋₁ · e^(−λ(tᵢ − tᵢ₋₁)) + 1
c  = cₙ · e^(−λ(now − tₙ))
```

which is precisely `Σᵢ e^(−λ(now − tᵢ))` — an exponentially weighted count. Fifty accesses two
years ago now count for less than three accesses this week.

**This is the paper's clearest single point.** The distinguishing property of a memory is not what
it stores; it is the shape of the function by which stored things lose their claim on attention. A
system whose evidence counters never decrease does not have a forgetting policy, whatever its
eviction code says.

**Grounding.** Anderson & Schooler (1991) analysed environmental data — word frequencies in news
headlines, in speech to children, in email — and found that the probability an item will be needed
again falls off as a power law in time since last use, and that human retention functions mirror
that environmental statistic. Their proposal is that memory has the form it does *because* it is
adapted to that statistic. ACT-R's base-level activation, `B = ln(Σⱼ tⱼ^(−d))`, is the same
quantity in log space: a decayed sum over past uses. We use exponential rather than power-law
decay for two engineering reasons — it composes into the incremental fold above, so a pass costs
one multiply per episode rather than a full re-integration, and it has a single interpretable
parameter, a half-life, that a user can be asked about. **The trade is real and we state it: a
power law has a heavier tail, and the exponential form will forget genuinely long-lived references
faster than human memory does.** The pin exists partly to cover that gap; §13 lists it as a threat
to validity.

### 6.3 Tiering: two deliberate asymmetries

Four tiers — Working, STM, LTM, Cold — with thresholds on the bounded scale rather than quantiles.
Quantiles were rejected because a quantile-based policy always demotes something, even on an index
where everything is genuinely in use.

**Promotion is immediate; demotion is one tier per pass.** Touching a file puts it in the Working
Set at once, because a memory that took a week to notice what you are doing would be useless.
Falling out happens a step at a time, because the two errors are not symmetric: promoting too
eagerly costs a little precision for a day, while demoting too eagerly costs a re-index and,
far worse, teaches the owner that the tiering cannot be trusted.

**A node younger than a grace window is not judged at all.** One week. Demoting a file the watcher
indexed an hour ago is forgetting something before ever having had it. This has a visible
consequence on a fresh install — every node sits in one tier and the tier legend reads
`working 0 / ltm 0 / cold 0` — which looks like a bug and is not. The snapshot therefore explains
it in words rather than leaving the reader to infer it, a point we return to in §11.

### 6.4 A pin is an exemption

The specification called the pin weight "dominant". We implement it as `w₅ = 10` against an
ordinary ceiling of 4.8 — so it is dominant — *and* as a branch that returns before the demotion
logic is reached. Both, deliberately: the weight makes a pinned node rank first, and the branch
makes it immune to a future change in the weights.

A test asserts that a node pinned and untouched for ten years outranks a node with every other
signal at maximum. The property being defended is not a number. It is that **explicit human intent
is not a participant in an inference competition it might lose.**

### 6.5 Association from co-occurrence, and the burst that must be thrown away

`co_access` is the edge type no content model produces. A test and the module it covers may share
not one word; a design note and the code it argued about may be in different languages. What they
share is that they were open together, and the episode log records that.

The algorithm: group episodes into bursts (same session, or within a 300-second gap); within each
burst, every pair of distinct nodes co-occurs; a pair becomes an edge after appearing in **at least
two** distinct bursts, weighted `1 − exp(−bursts/3)`.

The critical rule is the negative one. **A burst touching more than twelve nodes is discarded
whole.** A human editing a feature touches five or ten files in a quarter of an hour; a build
touches four thousand in twenty seconds. If builds formed edges the graph would be a single
hairball in which everything relates to everything and no traversal means anything. Not truncated
— *discarded*: truncating keeps an arbitrary slice of a meaningless event and makes it look
meaningful.

On the real index this discarded 45 of 68 bursts. On the simulator, 183 of 378. That ratio is not
a tuning artefact; it is what the ratio of machine activity to human activity on a developer's
machine actually looks like, and any system deriving associations from filesystem events without
such a filter is building its graph mostly out of build output.

### 6.6 Consolidation as a pure function of the log

A pass runs five steps in dependency order: **replay → associate → score → tier → report.**
Edges are written before scoring because centrality is a scoring component, and a pass that scored
first would rank against the previous pass's graph — which is how a system ends up permanently one
step behind itself.

**The whole log is replayed every pass**, not just the new episodes. Deriving only from new
episodes makes the resulting graph depend on how often consolidation happened to run: an index
consolidated hourly and one consolidated weekly would disagree about what is related to what, from
identical histories. Replaying everything and upserting makes the edge set a pure function of the
log — run it twice, get the same graph — which is the idempotence property the milestone required.
The cost is bounded by a cap on replayed episodes, and measured in §9 at 240 ms for 31 000
episodes.

### 6.7 Abstraction: from "these are related" to "this is the parser work"

Association alone can only answer questions about pairs. The step that makes a memory *semantic*
turns a recurring cluster into one thing with a name.

Label propagation over the `co_access` edges gives the communities — run to a fixed point in a
deterministic node order, ties broken by the smaller label, so the fixed point is unique rather
than merely reachable. A community of between three and sixty files becomes a **concept node**
with `derived_from` edges to its members and a salience equal to the mean of its three most
salient ones. Its membership is rewritten every pass, never merged: a file that left the cluster
must stop being part of the concept, or abstraction becomes another way of accumulating.

**The name is chosen lexically, and that is a decision rather than a shortcut.** A concept is
called after the terms its members' paths share and the rest of the index does not — frequent
inside, rare outside. A summariser would produce better names, and would also make this step
non-deterministic, unrunnable offline, untestable and unreproducible from a seed. Four properties
traded for nicer strings is a bad trade. The cluster type carries its members and its ranked
terms, so adding a summariser later is a function from a cluster to a better label and nothing
else changes.

Two rules fell out of building it, and both are about honesty rather than quality.

**The label is the identity**, because that is what lets a cluster which gains a file update its
concept instead of growing a rival beside it. Which forces the second rule: **a cluster with no
term the others lack is not named at all.** Two earlier attempts appended a disambiguator — the
member count, which produced `kettle (5)` three times over, and then the smallest member id,
which produced `kettle #1230`: unique, stable, and meaningless. **A node that looks like knowledge
and is not is worse than an absent node**, because the graph is read as evidence.

**And the machine's own shape is not vocabulary.** The longest common prefix of every indexed path
is, on a personal computer, that person's home directory. The first concept the real index
produced was called `axon · legi · projects` — one word that meant something and two that were the
shape of somebody's filesystem. Those tokens are excluded now, and it is called `axon`.

### 6.8 What is still missing, and why the report says so

Promotion of consolidated material into a federation outbox is not implemented; it belongs to the
sync milestone. And the labels above are lexical, not written by a summariser.

Every report the system prints ends with a `not done in this pass` section naming both. Three
lines of output. The reason is a claim about how projects fail: a system that quietly omits part
of what its own specification promised is how a team ends up believing it is further along than
it is, and the omission then survives as folklore instead of documentation. **Printing the gap in
the same paragraph as the achievement is the cheapest honesty mechanism we know of.**

---

## 7. Biological grounding, used carefully

Neuroscience is a source of hypotheses here, not of authority. Three ideas earned their place by
being implementable and by making a testable difference to behaviour.

**Complementary learning systems.** McClelland, McNaughton & O'Reilly (1995) proposed that the
hippocampus learns rapidly and sparsely while neocortex learns slowly and in a distributed way,
with the former gradually training the latter — because a single system that learned fast enough
to store an episode in one shot would suffer catastrophic interference with what it already knew.
This is the STM/LTM split, and it is why consolidation is a batch process rather than something
that happens on every write.

**Replay.** Hippocampal place-cell sequences are reactivated during rest and sleep (Wilson &
McNaughton, 1994), compressed within sharp-wave ripple events (Buzsáki, 2015), and this
reactivation is implicated in memory consolidation (Rasch & Born, 2013). Our `replay` step is
named after this and works the same way: the day's episodes are re-read in order during idle time,
and it is that re-reading — not the original event — that forms the associations.

**The new theory of disuse.** Bjork & Bjork (1992) distinguish *storage strength*, which does not
decrease, from *retrieval strength*, which does. Forgetting is loss of access, not loss of the
trace. This is exactly the tier model: nothing is deleted, retrievability decreases, and a
demotion is reversible. It is also the argument for why forgetting is safe *here* specifically —
the "storage" is the filesystem, which we did not touch.

**What we do not claim.** No claim that this is how brains work, no claim that biological
plausibility is evidence of engineering merit. The analogy earned its keep by generating three
design decisions — batch consolidation, replay-derived association, demotion rather than deletion
— each of which has an independent engineering justification given above. If the neuroscience were
overturned tomorrow the design would stand on §§1–3.

---

## 8. Human grounding: how people actually find their own files

This is the literature the AI-memory field most consistently ignores, and it is the one with the
most direct evidence.

**People navigate; they do not recall names.** Barreau & Nardi's (1995) study found users
overwhelmingly preferred location-based browsing over search for their own files, and — the more
interesting finding — that a major function of file placement is *reminding*: files are left where
they will be seen again. A memory that reorganises a user's files into its own structure destroys
this. Hippocampus writes nothing next to a user's files and imposes no folder structure, for
exactly this reason.

**Re-finding is orienteering, not teleporting.** Teevan et al. (2004) found people locate known
items through a series of small contextual steps rather than by jumping straight to the target with
a precise query, even when a precise query was available. This is the argument for the search
overlay being one list that starts as a directory and becomes results as you type, rather than two
separate modes.

**Context beats content for personal material.** Cutrell & Dumais's "Stuff I've Seen" work (2003)
found that time, people and other contextual cues dominate topical keywords when people search
material they have seen before. This is the direct justification for `co_access` edges, for zones,
and for the Working Set — all three are context, and none is content.

The consequence for the field: personal memory is a *different retrieval problem* from
open-domain question answering, and importing the open-domain architecture wholesale imports the
wrong prior.

---

## 9. What was measured

All numbers from one machine, 2026-08-23. Reproducible from the repository; nothing here is an
estimate.

**The real index** — 108 139 nodes (102 435 files, 5 704 directories), 9 534 episodes, 140 021
chunks, 0 embeddings:

| operation | time |
|---|---|
| consolidation, dry run | **109 ms** |
| consolidation, full pass with writes | **1 516 ms** |
| `brain.graph`, 6 022 nodes + ancestors + 1 200 events | ~1 s, 2.34 MB JSON |

The dry run reads everything and computes everything; the difference is one transaction updating
108 139 rows.

**The simulator** — a synthetic workspace, 3 069 nodes, 60 simulated days, 31 120 episodes, one
consolidation pass per simulated day:

| quantity | value |
|---|---|
| pass duration | **200–240 ms** |
| bursts found / discarded as machine activity | 378 / 183 |
| co-access edges derived | 120 |
| final tier distribution | working 412 · stm 992 · ltm 406 · cold 1 259 |
| pinned nodes held through 60 days of disuse | 1 of 1 |

The tier distribution is the result that matters: **41 % of the index has been demoted out of the
hot tier after sixty simulated days, and the pinned node survived all of it.** That is the
behaviour §1 argued for, produced by the engine rather than asserted.

**Tests.** 881 Rust tests pass under `cargo test --workspace` — 29 of them covering the scoring,
decay, tiering and association mathematics in isolation with no database, 14 covering consolidation
against a real SQLite index, and 2 guarding the response path against the hang in §10.3. Nine
further tests cover the visualisation's layout and snapshot validation.

**The visualisation**, on the real GPU, headless. The frame rate is pinned to 60 by the browser's
animation clock, so the measurement is the time spent inside each frame against the 16.67 ms a
60 Hz frame is allowed:

| nodes | idle p50 | rotating p95 | layout transition p95 | worst frame |
|---|---|---|---|---|
| 3 000 | 0.30 ms | 0.40 ms | 0.80 ms | 2.10 ms |
| 6 022 | 0.40 ms | 1.10 ms | 3.30 ms | 4.30 ms |
| 20 019 | 0.40 ms | 1.10 ms | 6.30 ms | 9.50 ms |

60 fps held in every mode at every size, including the layout transition, which rewrites every
instance matrix and every edge vertex per frame. At four times the required node count the worst
frame used under two thirds of its budget. Two draw calls, whatever the node count, is why.

**What is not measured, and should be.** Retrieval quality before and after consolidation, on a
gold set. Whether salience-boosted ranking beats unboosted ranking for a real user. Whether any of
this helps somebody find things. Each is a real gap and none is hidden behind a plausible-sounding
proxy.

---

## 9a. Field evidence: two generations of one owner's second brain

The measurements above are of this system. This section is about two others, built by the same
person, one after the other, and the pair is more informative than either alone: the first was
abandoned, and what the second changed is the argument.

### The first generation: a curated file system that never forgot

File-based, human-curated, about a year of use by several agents. 1 147 notes, 415 of them
curated canon. Mandatory frontmatter, a six-criterion intake gate, layers by curation stage, an
append-only journal, a weekly prune report, a 3D graph viewer built from its own links. Better
kept than most such systems.

**Its search is `grep -r` over the whole tree, backups included, with no ranking.** A query
returns the same note from three generations of backup *before* the canonical copy, in filesystem
order. An agent obeying that system's own read-before-you-work rule starts from a six-week-old
version.

**Its link graph decays under a rule that forbids it:** 75 of 415 canon notes have no link in
either direction, though the intake gate requires at least one. Manual linking does not survive
volume.

**And its forgetting mechanism has never once fired.** Seven consecutive weekly prune reports are
byte-identical and say "none", beside a health file in the same directory listing those 75
orphans. The criteria explain it:

```
status == "draft"  AND  stale 14 days  AND  body < 800 chars  AND  no links
```

Four conditions with AND — and the same system's intake gate *requires* a link and sets status to
stable. **The forgetting criteria were made unreachable by the intake rules of the same system**,
and nobody noticed for seven weeks, because a report saying "none" is indistinguishable from a
system with nothing to forget. Even when it fires it changes nothing: it is explicitly
report-only, and no human acted in those seven weeks.

That is not negligence. It is the correct response to a store that is **authoritative**. Throwing
something away is irreversible; so nobody does; so it grows monotonically. §1 argued that this is
where such systems end up. Here it is with a paper trail.

### The second generation: what its author changed

The successor is a running daemon — two memory tiers, a vector store, an LLM curator
consolidating in a nightly cycle, an MCP surface, a live 3D view. It replaced the first system;
the first was kept as the raw layer and is synced into it.

**Its first design sentence is the one this paper argues for, arrived at independently:**
*Markdown is the single source of truth; everything else — the relational store, the vector index
— is derived state, reconstructible at any time.* Its tiers are short / long / archive, and
unlike its predecessor **it actually demotes**: 9 notes in short memory, 1 295 in long, 1 193 in
archive. Forgetting works there because the same property is true there — the derived layer may
be thrown away.

Two of its recorded failures are worth more than its successes.

**A threshold sitting on a reachable maximum.** Its demotion threshold was 0.40, and the highest
score a note without access, focus or pin can reach is exactly its recency weight, also 0.40. So
every unused note sat on the demotion edge by construction, and its short memory ran at one note
of eighty. Their own account is the best sentence in either system's documentation: *a scoring
system can compute flawlessly and still be functionally dead — no unit test of the formula fails,
because the formula is right.* Their fix was not the number but an invariant: the clearance
between threshold and reachable maximum is now asserted by a test. **The same coupling was latent
in this system**, four per cent from the same failure; §10.5.

**Derived data collapsing silently.** It lost 939 of ~4 900 semantic edges twice in one day. Once
because a rebuild that produced nothing was allowed to replace everything with nothing; once
because `sqlite3.OperationalError` inherits from `DatabaseError`, so a handler treating
`DatabaseError` as corruption deleted the database whenever a write lock timed out — which, on a
5 400-rpm disk under load, it regularly did. Their conclusions — *stale is better than empty*, *a
lock is not damage*, *set aside, never delete* — are now rules here.

### What the pair says

The first system proves the accumulation trap empirically, in a well-run instance. The second
proves that the escape is exactly the property this paper puts first — that the authoritative
copy and the queryable index must be different things — because that is the change its author
made, unprompted by us, when the first one stopped being trustworthy.

And the second one supplies the more uncomfortable finding. Its short memory currently holds nine
notes, **eight of which have an access count of zero** and were placed there by a synchronisation
job rather than by anybody using them. With recency the largest term and access zero everywhere,
its working set is in practice a list of *what was written most recently*. That is what a
recency-dominant score degenerates into once the other signals are empty, and it is the argument
for bounding every component and for deriving association from co-occurrence rather than from
arrival time.

---

## 10. Defects that reasoning did not find

We report these because the failure modes are general and the mechanisms that caught them are
transferable.

### 10.0 A component that is set, stored, printed and never added up

The salience model gained a term for what a memory has *earned* from outcome reports. The field
was added to the input, the weight to the configuration, the component to the breakdown; the
value was computed, written to the database and displayed in the interface. `Components::sum()`
lists its fields by hand, and the new one was not among them.

Nothing failed. The term contributed exactly zero, and the symptom was two memories with entirely
different histories scoring identically to sixteen decimal places — which is only suspicious if
something happens to compare them, and nothing routinely does. The guard now assigns each
component a distinct power of two and asserts the total, so a missing term is not merely detected
but *named* by the difference. **The general lesson: a sum written by hand over a struct is a
place where adding a field is a silent no-op, and the only cheap defence is a test that makes the
absence arithmetically identifiable.**

Immediately behind it was a second one of the same family. Making memories participate in the
scoring pass meant the pass now held identifiers for nodes that the memory housekeeping was about
to delete, and the write failed the entire consolidation on a foreign key. The order of the steps
in a pass is part of its specification.

### 10.1 Immutable logs and referential integrity are in tension

`episodes.node_id` is deliberately not a foreign key, so the log survives deletion of the file it
refers to. `edges` *does* have a foreign key. Deriving an association between two nodes that have
since been deleted therefore fails the entire transaction with `FOREIGN KEY constraint failed` —
which is what the first consolidation pass on a real index did, after the same code had run
cleanly hundreds of times in tests and in a 31 000-episode simulation.

The simulator could never have found it, because nothing in the simulator is ever deleted. **The
general lesson: a synthetic history that only ever adds cannot exercise the code paths that make
an append-only log hard.** The fix is one join. The test is named after the situation rather than
the fix.

### 10.2 A monotone counter is not a memory

Described in §6.2. Worth restating as a defect class: **any evidence counter that never decreases
will eventually dominate any decaying signal it is summed with.** The failure is silent — the
system keeps producing plausible rankings — and it is only visible in a long-horizon simulation
that asks whether anything is ever forgotten. A test suite made only of unit tests over single
scoring calls would never have shown it. The test that found it simulates thirty days and asserts
that a specific node has left the hot tier.

### 10.3 An answer that will not fit is a hang, not an error

The protocol is newline-delimited JSON with an 8 MB line limit. A snapshot request for 50 000
nodes produced roughly 19 MB, so the daemon could not write it — and the concurrent response path
discarded the write error, leaving the caller waiting for an answer that was never coming. Both
processes at 0.0 % of one core.

The interesting part is not the limit; it is that the error was *available and thrown away*. A
`let _ =` on a write is a decision that failing to answer is acceptable, and on a request/response
protocol it never is. The fix is at two layers: a reply that will not fit comes back as an error
naming its own size and telling the caller to ask for less, and the method that can produce a
large answer caps its own request at a size measured to fit.

**The general form:** every point where a system discards an error is a point where a failure
becomes a hang, and a hang is the most expensive failure to diagnose because it produces no
evidence at all.

### 10.4 A connection pool deadlocks against itself, silently

The snapshot function held a pooled database connection while calling helpers that each ask for a
connection. On a file-backed store with four connections this is a latent bug under load. On an
in-memory store — which has exactly one connection, because `:memory:` cannot be opened twice —
it is an immediate deadlock. The test suite stopped: no error, no failure, no CPU load, no output.
It looked exactly like a slow compile.

**A hang is a worse failure than a crash** because it does not localise itself, and a resource
pool with fewer resources in tests than in production converts a latent bug into a total stop. We
consider the latter a feature: the test configuration found it on the first run.

---

### 10.5 Five things two other systems found for us

Not defects in this code, but blind spots in it, found by reading the two systems in §9a rather
than this one. The last two are the ones that mattered.

**A copy answers instead of its original.** `backup`, `snapshot`, `archive` and `old` are now
noise-penalised path segments. Identical histories, and the copy ranks below the file it copies —
tested.

**Orphans are the most useful number a memory prints about itself**, and the useful version is
restricted: nodes **somebody has touched** that no association reaches. Without that restriction
a fresh index of a home directory reports a hundred thousand orphans, which is true, useless, and
indistinguishable from a broken graph. With it, 1 468 here.

**Byte-identical twins deserve their own counter.** 24 408 of 108 342 nodes here share a content
hash — 22 %. They cost nothing to store, because embeddings are keyed by hash. They cost
something to read, which is the number that matters.

**A threshold has to clear the maximum that must cross it.** A file merely opened once, this
instant, with no edit, no edges and no active project, reaches `0.176 + 1.500 = 1.676`. The
Working threshold was **1.60** — a clearance of four per cent. Lowering the recency weight to 1.4
for any reason would have made the Working Set permanently unreachable, and every test here would
still have passed, because every test used a node with several signals at once. The threshold is
1.15 now; more to the point, `single_touch_ceiling()` exists and two tests hold the thresholds up
against it. **Compare limits against limits, not only behaviour against cases.**

**A collapse must not be committed.** `write_concepts` deleted every concept the current pass did
not propose — so one bad pass, a truncated replay or an empty edge set, would have removed the
entire semantic layer without an error. It now refuses to replace the concept set when a pass
proposes fewer than half of what exists, and says so. Stale concepts are better than none.

---

## 11. A memory has to be inspectable

A system that decides what you will and will not be shown, on the basis of a score you cannot see,
computed from a history you cannot inspect, is asking for a great deal of trust. §3.4 argued that
the model consuming its output cannot supply the missing calibration. Neither can the human,
unless the memory is legible.

This is why the visualisation is not a demo. It is the instrument through which the scoring
function is falsifiable by looking.

The encoding assigns exactly one variable per channel — hue is the zone, brightness is the tier,
size is the salience, a halo is the Working Set — and there are two layouts because there are two
questions. The **containment tree** shows what the index *is*, and is where a person can find
something they recognise. The **tier shells** show what the index *believes*: Working at the
centre, Cold on the outside. Forgetting is only visible in the second one, as a node moving
outward, and it only reads as movement because the node was somewhere stable beforehand.

Both layouts are deterministic functions of the snapshot. A force-directed layout — the obvious
choice, and the one the specification suggested — settles differently on every reload, which means
two screenshots of the same brain cannot be compared and a node cannot be found twice in the same
place. For an instrument, reproducibility beats organic appearance.

The consolidation playback replays the last pass in five captioned steps using the daemon's own
report. Nothing in the animation is computed in the frontend: if the animation and the report ever
disagreed, the animation would be a fiction, and an instrument that can lie about the thing it is
measuring is worse than no instrument.

Two details we would defend to anyone building something similar:

- **The snapshot states its own basis in words**, including the awkward cases. On a five-day-old
  index it says that every node is in one tier because nothing has left the grace window yet —
  because a legend reading `cold 0` otherwise reads as a broken tiering.
- **The statistics never compare two different things.** An early version showed "3 119 edges shown
  / 120 total", because containment edges are derived from parent links and are not in the edge
  table at all. It looked like a daemon bug. They are counted apart now.

---

## 12. Comparison

Against published descriptions, not against re-implementations. §13 states what this is worth.

| property | vector-store memory | conversational agent memory | Hippocampus |
|---|---|---|---|
| unit of memory | chunk | conversational fact | file, directory, episode, concept |
| authority | the store | the store | the filesystem; the index is a cache |
| reconstructible after deletion | no | no | **yes, by design and tested** |
| forgets | no | eviction policies | **decay → demotion, four tiers** |
| ranking explains itself | no | rarely | **component breakdown, tested to sum** |
| associations from | embedding similarity | LLM-extracted relations | **co-occurrence in time**, plus similarity |
| abstraction | none | LLM summaries | **communities of co-access, named from shared vocabulary** |
| knows what you are doing now | no | no | **Working Set from episodes** |
| human intent | none | none | **pins, exempt from inference** |
| model on the retrieval path | yes (embedding) | usually | **no** — hybrid lexical + vector |
| works offline | varies | rarely | **yes; zero network by default** |
| the human can inspect it | no | no | **yes — that is what §11 is** |

Where the others are ahead, plainly: conversational memory systems remember dialogue, which this
does not attempt; Zep's temporal knowledge graph handles superseded facts more precisely than a
salience score does; and Mem0 has published benchmark numbers on LoCoMo, which we have not
published on anything.

---

## 13. Limitations and threats to validity

Stated without hedging, because a paper whose limitations section is shorter than its results
section is advertising.

1. **No retrieval-quality evaluation.** We have shown the memory forgets and that it is fast. We
   have *not* shown that a user finds what they want more often. That requires a gold set and a
   before/after comparison and it is the single most important missing experiment.
2. **The comparison in §12 is against descriptions.** It is an architectural comparison, not a
   benchmark. Nobody has run these systems side by side on the same corpus, and until somebody
   does, §12 is an argument.
3. **Exponential rather than power-law decay** (§6.2) will forget long-lived references faster
   than the environmental statistics suggest it should. The pin partly covers this. It is a real
   deviation from the cognitive grounding we cite.
4. **The weights are chosen, not learned.** Seven weights, set by argument, validated only by
   whether the resulting tier distribution looks sane. They should be fitted against observed
   re-access — the data to do it is in the episode log.
5. **Concept names are lexical** (§6.7). The clustering is sound and the names are honest, but
   they are the vocabulary of a filesystem, not a summary of a subject. On an index with little
   history there are very few of them — one, on the machine measured here.
6. **Single-machine, single-user, one operating system.** Everything measured here ran on one
   Linux desktop. Multi-device federation is designed and not built.
8. **The simulator's history is invented.** Its *response* to that history is the real engine, but
   a generator written by the same author as the scoring function can accidentally generate the
   history the scoring function likes. §10.1 is a worked example of exactly that risk.

---

## 14. What would falsify this

A design paper should say what evidence would show it is wrong.

- **If a user, over months, repeatedly finds that things they needed had been demoted**, the
  central claim fails — forgetting would not be safe in practice even if it is reversible in
  principle. This is measurable: log every retrieval of a Cold-tier node, since each one is a case
  where the tiering was wrong.
- **If salience-boosted ranking does not beat unboosted ranking on a gold set**, then the scoring
  machinery is elaborate decoration and the system should be a fast hybrid search with pins.
- **If co-access edges are never traversed** in real use, association-by-co-occurrence is a
  hypothesis that did not pay for itself.
- **If explaining a ranking changes nobody's behaviour** — if the explanation is never looked at,
  or looking at it never changes what the user does — then §3.4's safety argument is theoretical
  and the cost of maintaining explainability should be reconsidered.

Each of these is instrumented or instrumentable today. None has been run.

---

## 15. Reproduction

The companion document `BUILD-PROMPT.md` in this directory specifies the complete system in
buildable form: milestones with executable acceptance criteria, the schema, the mathematics with
its failure modes, the hard rules, and the defects above as regression tests to write before
the code that would contain them.

It is written to be handed to a competent implementer — human or agent — who has not read this
paper. This paper is the argument; that document is the specification.

---

## References

Anderson, J. R., & Schooler, L. J. (1991). Reflections of the environment in memory.
*Psychological Science, 2*(6), 396–408.

Anderson, J. R., Bothell, D., Byrne, M. D., Douglass, S., Lebiere, C., & Qin, Y. (2004). An
integrated theory of the mind. *Psychological Review, 111*(4), 1036–1060.

Barreau, D., & Nardi, B. A. (1995). Finding and reminding: File organization from the desktop.
*ACM SIGCHI Bulletin, 27*(3), 39–43.

Bjork, R. A., & Bjork, E. L. (1992). A new theory of disuse and an old theory of stimulus
fluctuation. In A. Healy, S. Kosslyn, & R. Shiffrin (Eds.), *From Learning Processes to Cognitive
Processes* (Vol. 2, pp. 35–67). Erlbaum.

Buzsáki, G. (2015). Hippocampal sharp wave-ripple: A cognitive biomarker for episodic memory and
planning. *Hippocampus, 25*(10), 1073–1188.

Chhikara, P., Khant, D., Aryan, S., Singh, T., & Yadav, D. (2025). Mem0: Building production-ready
AI agents with scalable long-term memory. arXiv:2504.19413.

Cormack, G. V., Clarke, C. L. A., & Büttcher, S. (2009). Reciprocal rank fusion outperforms Condorcet
and individual rank learning methods. *SIGIR '09*.

Cutrell, E., Dumais, S. T., et al. (2003). Stuff I've Seen: A system for personal information
retrieval and re-use. *SIGIR '03*.

Edge, D., et al. (2024). From local to global: A graph RAG approach to query-focused
summarization. arXiv:2404.16130.

Hsieh, C.-P., et al. (2024). RULER: What's the real context size of your long-context language
models? arXiv:2404.06654.

Karpukhin, V., et al. (2020). Dense passage retrieval for open-domain question answering.
*EMNLP 2020*.

Lewis, P., et al. (2020). Retrieval-augmented generation for knowledge-intensive NLP tasks.
*NeurIPS 2020*.

Liu, N. F., et al. (2023). Lost in the middle: How language models use long contexts.
arXiv:2307.03172.

McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). Why there are complementary
learning systems in the hippocampus and neocortex. *Psychological Review, 102*(3), 419–457.

Packer, C., et al. (2023). MemGPT: Towards LLMs as operating systems. arXiv:2310.08560.

Park, J. S., O'Brien, J. C., Cai, C. J., Morris, M. R., Liang, P., & Bernstein, M. S. (2023).
Generative agents: Interactive simulacra of human behavior. *UIST '23*. arXiv:2304.03442.

Rasch, B., & Born, J. (2013). About sleep's role in memory. *Physiological Reviews, 93*(2),
681–766.

Rasmussen, P., et al. (2025). Zep: A temporal knowledge graph architecture for agent memory.
arXiv:2501.13956.

Robertson, S., & Zaragoza, H. (2009). The probabilistic relevance framework: BM25 and beyond.
*Foundations and Trends in Information Retrieval, 3*(4), 333–389.

Teevan, J., Alvarado, C., Ackerman, M. S., & Karger, D. R. (2004). The perfect search engine is
not enough: A study of orienteering behavior in directed search. *CHI '04*.

Vaswani, A., et al. (2017). Attention is all you need. *NeurIPS 2017*.

Whittaker, S. (2011). Personal information management: From information consumption to curation.
*Annual Review of Information Science and Technology, 45*(1), 1–62.

Whittaker, S., & Sidner, C. (1996). Email overload: Exploring personal information management of
email. *CHI '96*.

Wilson, M. A., & McNaughton, B. L. (1994). Reactivation of hippocampal ensemble memories during
sleep. *Science, 265*(5172), 676–679.

Xu, W., et al. (2025). A-MEM: Agentic memory for LLM agents. arXiv:2502.12110.
