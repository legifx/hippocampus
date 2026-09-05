# What 2026 changed

### A research update to *Forgetting is the feature*, and the argument for one more milestone

**Status:** written 2026-09-05 against commit `45367ca`. The design paper this updates is
[`FORGETTING-IS-THE-FEATURE.md`](FORGETTING-IS-THE-FEATURE.md); the milestone it argues for is
**M12** in [`BUILD-PROMPT.md`](BUILD-PROMPT.md) §8 and in `docs/01-BUILD-PROMPT.md`. Where this
document and the build prompt disagree, the build prompt wins, because it has acceptance criteria.

**How the sources were gathered.** A three-round literature search (arXiv, OpenAlex, GitHub,
Hugging Face; 2025–2026; 27 sub-queries) produced 938 candidates, of which 230 were fetched as
text into a research library kept *outside* this repository (`hippocampus-research/agent-memory-2026/`,
`manifest.json` lists every item with its source URL). The final round still had a novelty rate
of 79 %, so the field is larger than what is read here; the reading below is of abstracts for all
230 and full text for the forty-odd that bear on this design. Every claim that carries a number
cites the paper it came from. Nothing here was run on this machine; §7 says what should be.

---

## 0. The one-paragraph version

The design paper was written in August 2026 against a field that stored everything and retrieved by
cosine. Three weeks later the field has moved in three directions, and each one lands on a part of
this design. **Forgetting has become mainstream** — half a dozen systems now decay, tier and
strategically forget, and the best of them report that a utility signal beats heuristic decay
(SF-AMS, +9.65 F1 on multi-hop) and that decay must be *type-conditioned* to generalise over time
(ScrubJay-MEM). That confirms §6 and corrects one thing in it. **Evaluation has turned on itself** —
controlled protocols (MemDelta, the retrieval-vs-utilisation diagnostic, Harness the Memory) find
that most published memory gains were embedding confounds, that the retrieval arm dominates the
write arm by 14–23 points, and that raw storage with no model in the write path matches expensive
extraction. That is this design's central bet, and it now has somebody else's numbers behind it.
**Security and truth-maintenance have become the open problem** — a memory that any file, tool or
peer can write into is a memory that can be poisoned once and obeyed for months; consolidation
silently promotes a third-party remark to a user instruction in 48 of 49 tested configurations; and
"newer supersedes older" turns out to presuppose three things this design never checked. That is
the gap. M12 closes it with four fields on a memory — origin, authority, validity, state — and one
rule about how they combine, plus the evaluation the paper admitted it did not have.

---

## 1. Forgetting: confirmed, and corrected in one place

### 1.1 What others built

| system | mechanism | what it reports | source |
|---|---|---|---|
| FadeMem | dual-layer store; per-memory *adaptive exponential* decay modulated by relevance, access frequency (as an exponentially time-decayed access rate, not a raw count) and recency; hysteresis on layer transitions | 45 % storage reduction on Multi-Session Chat, LoCoMo, LTI-Bench with better multi-hop | Wei et al. 2026, arXiv:2601.18642 |
| SF-AMS | "utility-driven survival": importance updated from usage, redundancy and temporal signals; four-layer hierarchy; entity anchoring | +9.65 F1 multi-hop (Qwen2.5-7B), +6.91 temporal, +6.53 open-domain over LightMem, Mem0, A-Mem on LoCoMo and LongMemEval-s | Yang et al. 2026, arXiv:2607.22562 |
| ScrubJay-MEM | per-memory, **type-conditioned** perishability coefficient and utility horizon; retroactive revision of decay parameters | only retrieval system with positive generalisation to held-out retention intervals; ablating type-conditioned decay collapses the gain; gains *reverse* on fact-consolidation tasks | Bhandari et al. 2026 ("Caching for the Future") |
| LUFY | six psychology-derived memory metrics with **learned** weights, used for both retrieval and forgetting | two-hour human conversations; forgetting most of the conversation while keeping emotionally salient parts raised satisfaction | Sumida et al. 2025, *Dialogue & Discourse* |
| budgeted forgetting | relevance-guided scoring under a bounded optimisation | long-horizon F1 above a 0.583 baseline without more context; fewer false memories | Fofadiya et al. 2026, arXiv:2604.02280 |
| DRM / ACT-R comparison | a production store hit *score inflation* (54 % of entries at 9–10, top-5 accuracy 34.2 %); seven ranking algorithms compared; ACT-R activation dynamics plus category gating won at 2.6 ms per ranking over Markdown files, no embeddings | Seki 2026, zenodo.19425474 |

### 1.2 What that means here

**The exponentially weighted access count is the right primitive.** FadeMem arrives at exactly the
correction §6.2 of the paper reports — replacing a raw count with an exponentially time-decayed
access rate — for the same reason. §6.2 stands.

**Correction: one half-life is wrong for memories.** ScrubJay-MEM's result is that a uniform decay
either discounts stable knowledge too fast or keeps ephemeral facts too long, and that a two-way
short/long split "covers only two points on a continuum with at least four distinguishable rates."
Hippocampus already has the vocabulary — six memory kinds (§M11) — but applies one
`decay_half_life_days` to all of them. A `preference` should outlive an `observation` by an order
of magnitude; a `failure` should sit between. **M12 makes the half-life a function of the kind**,
with the current value as the default for files, and adds the ScrubJay falsifier: retrievals of
memories that have been demoted are the measurement, per kind.

**Utility beats heuristics, and this design has the utility signal.** SF-AMS's largest gains come
from tracking usage and redundancy rather than time, which is what `brain.outcome` and the
`usefulness` component already are (D76). The design paper's "retrieved and never reported useful
decays faster than never retrieved" is, in SF-AMS's terms, a survival function on utility. Kept,
and now with an external number to compare against once the gold set exists (§7).

**The weights should be fitted.** LUFY learned its metric weights and it mattered. §13.4 of the
paper listed unfitted weights as a threat; M12 adds a step for fitting them against observed
re-access in the episode log, offline, with the fitted vector written into the consolidation report
so a human can read what changed.

**ACT-R is not dead here.** The paper chose exponential decay over ACT-R's power law for the
incremental fold. Seki's comparison found ACT-R activation beat static scores in production — but
it did so on 200 entries where the full re-integration is free. On a hundred thousand nodes the
fold still wins; the trade stays as stated in §6.2 and §13.3. What Seki adds is the failure mode to
test for: **score inflation**, everything clustering at the top of the scale. Hippocampus bounds
every component to [0,1], which is the structural defence; M12 adds the tier-distribution assertion
to the consolidation report so inflation is visible when it happens.

---

## 2. Evaluation: the field measured itself and found confounds

### 2.1 Findings

- **Retrieval dominates writing.** A 3×3 factorial on LoCoMo (three write strategies × three
  retrieval methods): accuracy spans 20 points across retrieval methods (57.1 → 77.2 %) and only
  3–8 across write strategies; raw chunks with *zero* LLM calls at write time match or beat Mem0-style
  extraction and MemGPT-style summarisation; failures manifest at retrieval, not utilisation.
  (Yuan et al. 2026, arXiv:2603.02473.)
- **One variable flips the conclusion.** MemDelta on LongMemEval-S: Mem0 beats MiniLM-RAG by ~11
  points and *loses* to the same RAG with a better embedding by 1.2; agent self-memory (42 %)
  underperforms plain retrieval (47 %); Sonnet refuses 63 % of full-context queries, which changes
  what the baseline even is. Recommendation: fix the embedding across comparisons, stratify by model
  family, report write-path cost before attributing gains to architecture. (Wang et al. 2026.)
- **No substrate dominates.** Across 52 systems, 62 % of evaluations use LoCoMo or LongMemEval;
  21 % report any efficiency metric; broad retrieval helps factual QA and *hurts* sequential
  decision-making. (Huang et al. 2026, "Harness the Memory", arXiv:2608.15008.) The same
  conclusion from a data-management angle, with localised maintenance cheaper than global
  reorganisation. (Zhou et al. 2026, arXiv:2606.24775.)
- **Recall benchmarks miss the failures that bite.** Production failures are predominantly
  *forgetting* failures — a rotated credential still recommended, a deleted record still surfacing —
  and no standard benchmark tests whether a store can be commanded to forget; a system can saturate
  recall and score 0/385 on forgetting. (Yang et al. 2026, "Control-Plane Placement".)
- **Knowing is not acting.** Paired Know/Act tests over 1 000 preferences and 16 systems: agents
  pass recall and fail to apply the same preference in a behavioural scenario; worst for health
  preferences. (Feng et al. 2026, "Know It, Act on It".)
- **Usability degrades with scale even when evidence is held fixed** — adding irrelevant sessions
  alone lowers accuracy. (Shao et al. 2026, arXiv:2605.07313.)

### 2.2 What that means here

**The central bet is now externally supported.** No language model in the write path (D75, §3.5 of
the paper) was an argument; Yuan et al. is a measurement of the same claim on somebody else's data.
The design keeps it and stops apologising for it.

**§12 of the paper must be demoted, not deleted.** It compares against descriptions; MemDelta shows
that even comparisons against implementations mislead unless one variable moves at a time. The
comparison stays as an *architectural* table with a sentence saying exactly that.

**The missing experiment has a protocol now.** §13.1 said "no retrieval-quality evaluation." M12
specifies it in MemDelta's shape: a local gold set built from the owner's own re-finding episodes,
one variable per run, the embedding fixed, and the four falsifiers of §14 each turned into a row.
The retrieval-vs-utilisation split is free here: `brain.outcome` already separates "was in the
pack" from "helped."

**A forgetting test suite is mandatory.** Criterion 9 of M11 (a deletion survives a rebuild) is one
row of what Control-Plane Placement calls the control plane. M12 adds the rest: supersede,
release, purge, non-revival, and "forget everything about X" — asserted per surface, scored by
substring absence, the way ForgetEval scores it.

---

## 3. Security: a memory anyone can write is a memory anyone can steer

### 3.1 Findings

- **Four write channels, nine structural vulnerabilities, six attack classes.** Memory is built from
  web pages, documents, e-mails and tool outputs and then treated as trusted; agents that write and
  retrieve more aggressively are more exploitable; prompt-injection defences do not cover it because
  the payload need not look like an instruction in context. (Dash et al. 2026, arXiv:2606.04329.)
- **Content and lineage are both malleable.** Three laundering channels — the agent's own
  summarisation, a trusted tool echoing attacker content, manufactured corroboration — erase an
  untrusted origin. Machine-checked result: no content- or lineage-based defence is sound under
  laundering; write-time origin binding is necessary; *non-malleable* origin-bound authority with
  elevation only through two independent trusted principals or a fresh action-bound human
  authorisation is sufficient. Existing defences fail at up to 68 % laundering success. (Louck et al.
  2026, arXiv:2606.24322.)
- **Authority collapse.** Consolidation turns "a third party said the user lives in Seattle" and "the
  user said so" into the same memory; observed in 48 of 49 consolidator × backbone configurations;
  unauthorised-action rate 50.3 % without authority metadata, 16.9 → 0.0 % with predicted-and-persisted
  authority labels, benign success unchanged. (Zhan et al. 2026, "When Memory Becomes Authority".)
- **Sleeper poisoning** stays dormant until a trigger session (Pulipaka et al. 2026,
  arXiv:2605.15338); **stealthy injection** into persistent personal agents through external content
  (Zhang et al. 2026, arXiv:2607.05189); MINJA-class attacks reach >95 % injection success under
  idealised conditions (Sunil et al. 2026, arXiv:2601.05504); MemSecBench traces one poison from
  persistence to consequence to repair (Chen et al. 2026, arXiv:2607.27080).

### 3.2 What that means here

Hippocampus has the log, the audit trail, the policy engine, the kill switch, the human's delete
right and the scope. It has **no notion of where a memory came from or what it is allowed to do.**
`brain.remember` accepts text from whoever holds the socket; the indexer turns every file's content
into retrievable chunks; a `sync` plugin will bring memories from a peer. All three are write
channels in Dash et al.'s sense, and today all three produce memories that a context pack presents
identically.

**M12 adds two fields and one rule.** `origin` — who caused the bytes to exist: `human`,
`agent` (this agent's own note), `tool` (an fs.read, a preview, a command), `file` (content
indexed from disk), `peer` (federation). `authority` — what the memory may be used *as*:
`instruction` (may change behaviour and gate a write), `fact` (may be stated), `observation`
(may be cited, with its origin), `untrusted` (retrievable on request only, never in a pack without
its label). The rule is Louck et al.'s: **authority is non-malleable downward and gated upward.** A
memory derived from an `untrusted` or `file` origin inherits the lowest authority of its sources —
the agent paraphrasing a web page does not launder it — and authority rises only when a human says
so (an action-bound confirmation in the Inspector) or two independent trusted origins agree. The
policy engine consults `authority` before any gated write that a memory motivated; an `untrusted`
instruction cannot cause an `fs.write`.

This costs two columns, one propagation rule in `brain.remember`, one check in the policy engine,
and a label in the pack. It is the cheapest change in M12 and the one with the strongest evidence.

---

## 4. Truth maintenance: "newer wins" presupposes three things

### 4.1 Findings

- The recency rule presupposes (1) a key that decides which records can conflict at all, (2) that
  write order tracks the order in which facts held — transaction time versus valid time, (3) that a
  newer record is at least as trustworthy, which runs against evidence that extraction and update
  are where errors are *made*. And "supersede" carries two questions belief revision separated
  decades ago: *the world changed* versus *the earlier record was wrong*. Retaining both records and
  labelling them is endorsed — and then deflated, because it moves the decision from write time to
  read time, where the measurements are weakest. (Mahendrakar 2026, "Newer Is Not Truer".)
- **Ghost memory**: old, current and transition facts coexist, get retrieved together and mislead;
  labelling state roles (`current`, `historical`, `transition`) in the evidence packet raises
  Graphiti/Zep's temporal F1 on LoCoMo from 0.03 to 0.17 and conflict accuracy by 0.24 absolute.
  (Shi et al. 2026, A-TMA, arXiv:2607.01935.)
- **Five update operations, not two.** Add, ignore, revise, reject, defer-for-verification — with
  accepted, pending and rejected ledgers; binary write/hold conflates them. (Xiao et al. 2026,
  TARL, arXiv:2608.03699.)
- **Bitemporal state with fail-closed release**: source-bound admission, derived lifecycle state,
  non-revival after retraction or deletion; 2 400/2 400 on a sealed evaluation versus 600/2 400
  ungoverned. (Xu 2026, GPM, arXiv:2608.12476.) Contradiction resolution is a write-time isolation
  question with a formal operator algebra. (Wang et al. 2026, TOKI, arXiv:2606.06240.)
- Memory updates as transactions with source support and complete-state recovery (Cui et al. 2026,
  MemTxn).

### 4.2 What that means here

The design already keeps both records (a superseded memory is excluded from retrieval, not deleted;
R10) and already refuses to let the daemon *decide* a supersession. What it lacks is exactly what
Mahendrakar names: the reason, the validity interval, and the state label at read time.

**M12 adds `valid_from` / `valid_to` beside `created_at`** — the daemon records both times and
never invents the first — and **`supersedes` takes a `reason`: `changed` or `wrong`.** The two
are retrieved differently: a memory superseded because the world changed is *history* and
answers "what was true in July"; one superseded because it was wrong is *retracted* and never
returns without the label. **The context pack labels every memory's state** — `current`,
`historical`, `retracted`, `pending` — the way A-TMA labels its evidence packets, which is the one
intervention in this list with a measured effect on answers.

**`pending` is TARL's defer.** A memory whose origin is `file`, `tool` or `peer` is admitted as
`pending`: retrievable when asked for, absent from a pack by default, promoted by a human, by an
outcome report that says it helped, or by the consolidation pass when two independent origins
agree. That is the quarantine the poisoning literature asks for, expressed in a state the design
already needs for truth maintenance. The rule is one enum, not a second store.

---

## 5. Retrieval and the context pack: small, measurable changes

- **Adaptive fusion.** Fixed-weight RRF is beaten on all five BEIR sets by per-query IDF-weighted
  RRF (up to +21.4 % NDCG@10), and a **negative result** matters here: post-RRF frequency-and-decay
  boosting, history-augmented recall and a cross-encoder reranker all *failed* to improve NDCG in a
  local-first SQLite system. (Steffens 2026, vstash.) A BM25-margin cascade that skips the dense arm
  when the lexical arm is confident cuts latency at parity accuracy. (Yuan et al. 2026, AgentIR.)
  → M12: per-query weighting of the RRF arms from query IDF, and a cascade that skips ANN when the
  trigram/BM25 margin is wide, both behind the existing latency gates. And the salience boost is
  now explicitly on trial: vstash's negative result is the paper's falsifier #2 observed elsewhere.
  If the boost does not beat unboosted on the gold set, it comes out of the ranking and stays in
  the explanation.
- **Context as an environment, not a summary.** Scroll keeps an append-only event log with stable
  addresses and evicts spans from the model's view while keeping them recoverable by address
  (94.8 % LongMemEval-S; +5.1 over the best memory system on BEAM-10M). (Lin et al. 2026,
  arXiv:2608.21690.) Context management as a callable tool with a structured workspace — stable
  task semantics, condensed long-term, high-fidelity short-term — beats append-only and static
  compression on SWE-Bench-Verified (57.6 %). (Liu et al. 2025, "Context as a Tool".)
  → Confirms the shape of `brain.context`: every item carries an address (path + byte range,
  memory id) and can be re-materialised; the pack is a *view*, the episode log is the truth. M12
  adds the one missing piece: the pack says what it *left out* and why (budget, pending, retracted)
  in a `omitted` summary, so an agent can ask for more instead of guessing.
- **Filesystem memory, measured.** Organised stores halve retrieval cost where material is large;
  but organisation *erodes* for all but the strongest management agent, no agent converts
  organisation into better answers, and changing the tool set reshapes the store as strongly as
  swapping the model. (Zhou et al. 2026, arXiv:2607.26637.) → This is the case for D74 restated by
  people who measured the alternative: the *daemon* organises (concepts, places, tiers) and the
  agent writes nodes; the agent is never asked to keep a directory of notes tidy.
- **Local, zero-LLM, hash-chained.** SuperLocalMemory 4.0 reports multi-channel RRF, bi-temporal
  recall, scope isolation, a hash-chained audit trail — and retracts two of its own earlier claims
  after finding ten mechanisms that were "implemented, reachable and effective: three different
  questions." (Bhardwaj 2026.) → Two things to copy: the audit log gains a hash chain in M12 (cheap;
  makes tampering with the log itself detectable, which the poisoning threat model needs), and the
  reporting discipline — every M12 criterion is bracketed by a test that fails when the mechanism is
  disabled.
- **Memory gravity.** A wiki-style single-user memory should protect entries whose removal would
  cascade (dependency centrality) and periodically *audit* its highest-gravity entries by
  suspending them and re-running logged queries. (Miteski 2026, "Memory as Metabolism".) →
  `centrality` already is this; the audit is a consolidation option for M12 §3, not a requirement,
  because it needs the logged-query replay that the gold set provides.

---

## 6. Where Hippocampus is ahead, plainly

Read against the same corpus, four things this design has that the field is now asking for:

1. **The index is a cache** (R2). Scroll's lossless event log and GPM's ledger are the same
   principle; most systems still cannot rebuild.
2. **No model in the write path** (D75), now measured elsewhere to be the right call.
3. **A closed feedback loop** (`brain.outcome`, D76). CoEvo-Mem, AdaMem, Learning-to-Retrieve and
   Mem-T are all attempts to learn retrieval from outcomes; this design has the signal wired in
   from the start, without training a policy.
4. **Explainable ranking, inspectable by a human** (§11 of the paper). Nothing in the corpus
   ships an Inspector; the closest is memorywire's proposal for a governance surface that lets a
   human review writes before they persist — which is what `pending` plus the Inspector becomes.

And four things it lacked until M12, in order of evidence: origin-bound authority (§3), state
labels and validity (§4), a retrieval-quality evaluation (§2), kind-conditioned decay (§1).

---

## 7. What to run, and what would change the plan

Nothing in this document was measured on this machine. In order:

1. Build the gold set from the owner's own episodes (M12 step 1) and run the four falsifiers of
   §14 of the paper. **If salience-boosted ranking does not beat unboosted**, the boost leaves the
   ranking (§5). **If cold-tier retrievals concentrate in one memory kind**, that kind's half-life
   is wrong and the fix is a number, not a design.
2. Run the forgetting suite. A single resurrection after a human delete is a release blocker.
3. Seed a poisoned file, a poisoned tool output and a poisoned peer batch; assert that none
   reaches a pack without its label and none can motivate a gated write.
4. Re-run the latency gates with adaptive fusion on. If the cascade does not pay on this index —
   it was measured on conversational memory, not a home directory — it is one config flag and it
   goes back off.

---

## References (2025–2026 additions; the paper's original list is unchanged)

Bhandari, K., et al. (2026). Caching for the Future: Scrub Jay Episodic Memory Principles for Agent
Memory Systems. arXiv.

Bhardwaj, V. P. (2026). SuperLocalMemory 4.0: The Governed Memory Operating System for AI Agents. arXiv.

Chen, X., et al. (2026). MemSecBench: Tracking Agent Memory Poisoning from Persistence to Consequence
and Repair. arXiv:2607.27080.

Cui, H., et al. (2026). MemTxn: A Transaction Boundary for Source-Supported Updates and Complete-State
Recovery in Agent Memory. arXiv.

Dash, P., et al. (2026). From Untrusted Input to Trusted Memory: A Systematic Study of Memory Poisoning
Attacks in LLM Agents. arXiv:2606.04329.

Feng, Z., et al. (2026). Know It, Act on It: Investigating Memory Utilization in LLM Personalization. arXiv.

Fofadiya, P., et al. (2026). Novel Memory Forgetting Techniques for Autonomous AI Agents. arXiv:2604.02280.

Huang, W.-C., et al. (2026). A Survey of Agent Memory in the Second Half. arXiv:2602.06052.

Huang, W.-C., et al. (2026). Harness the Memory: A Holistic Evaluation of Memory Substrates in Memory
Agents. arXiv:2608.15008.

Lin, Y., et al. (2026). Context as an Environment: Programmatic Context Management for Long-Horizon
Agents. arXiv:2608.21690.

Liu, S., et al. (2025). Context as a Tool: Context Management for Long-Horizon SWE-Agents. arXiv:2512.22087.

Louck, Y., et al. (2026). Securing LLM-Agent Long-Term Memory Against Poisoning: Non-Malleable,
Origin-Bound Authority with Machine-Checked Guarantees. arXiv:2606.24322.

Mahendrakar, P. M. (2026). Newer Is Not Truer: Three Presuppositions Behind the Recency Rule in Long-Term
Agent Memory. Zenodo, 10.5281/zenodo.22182349.

Miteski, S., et al. (2026). Memory as Metabolism: A Design for Companion Knowledge Systems. arXiv.

Pulipaka, S., et al. (2026). Hidden in Memory: Sleeper Memory Poisoning in LLM Agents. arXiv:2605.15338.

Seki, T. (2026). From Static Scores to Cognitive Retrieval: A Comparative Evaluation of Memory
Prioritization Algorithms for LLM Multi-Agent Systems. Zenodo, 10.5281/zenodo.19425474.

Shao, J., et al. (2026). When Stored Evidence Stops Being Usable: Scale-Conditioned Evaluation of Agent
Memory. arXiv:2605.07313.

Shi, Z., et al. (2026). A-TMA: Decoupling State-Aware Memory Failures in Long-Term Agent Memory.
arXiv:2607.01935.

Steffens, J. (2026). vstash: Local-First Hybrid Retrieval with Adaptive Fusion for LLM Agents. arXiv.

Sumida, R., Inoue, K., & Kawahara, T. (2025). Enhancing Long-term RAG Chatbots with Psychological Models
of Memory Importance and Forgetting. *Dialogue & Discourse*, 10.5210/dad.2025.203.

Sunil, B. D., et al. (2026). Memory Poisoning Attack and Defense on Memory Based LLM-Agents.
arXiv:2601.05504.

Wang, K., et al. (2026). MemDelta: Controlled Baselines and Hidden Confounds in Agent Memory Evaluation. arXiv.

Wang, Z., et al. (2026). TOKI: A Bitemporal Operator Algebra for Contradiction Resolution in LLM-Agent
Persistent Memory. arXiv:2606.06240.

Wei, L., et al. (2026). FadeMem: Biologically-Inspired Forgetting for Efficient Agent Memory.
arXiv:2601.18642.

Xiao, H., et al. (2026). TARL: Transaction-Aware Reliable Ledgers for Executable Memory Management in
Long-Term Agents. arXiv:2608.03699.

Xu, G. (2026). Governed Persistent Memory: Source-Bound State Semantics and Fail-Closed Release for
Long-Horizon Agents. arXiv:2608.12476.

Yang, D., et al. (2026). Control-Plane Placement Shapes Forgetting: An Architectural Study of Agent
Memory Across Thirteen System Configurations. arXiv.

Yang, N., et al. (2026). SF-AMS: Strategic Forgetting for Structured Memory in LLM Agent. arXiv:2607.22562.

Yuan, A., et al. (2026). AgentIR: A Workload-Adaptive Cascade Retrieval Substrate for Long-Term
Conversational Memory. arXiv:2605.25092.

Yuan, B., et al. (2026). Diagnosing Retrieval vs. Utilization Bottlenecks in LLM Agent Memory.
arXiv:2603.02473.

Zhan, Q., et al. (2026). When Memory Becomes Authority: Benchmarking Authority Collapse at the Memory
Consolidation Boundary. arXiv.

Zhang, Y., et al. (2026). When Claws Remember but Do Not Tell: Stealthy Memory Injection in Persistent
Personal Agents. arXiv:2607.05189.

Zhou, S., et al. (2026). Filesystem-Based Memory for LLM Agents: Organization, Evolution, and
Sustainability. arXiv:2607.26637.

Zhou, W., et al. (2026). Are We Ready For An Agent-Native Memory System? arXiv:2606.24775.
