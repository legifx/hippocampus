# Papers

Four documents, written to be read in this order and published together.

| file | what it is | who it is for |
|---|---|---|
| [`FORGETTING-IS-THE-FEATURE.md`](FORGETTING-IS-THE-FEATURE.md) | The design paper. Why memory systems that only accumulate fail, what a language model actually needs from a memory, the mathematics of forgetting, what was measured, and what would falsify the design. | somebody deciding whether this architecture is right |
| [`A-DIRECTORY-IS-JUST-ONE-QUERY.md`](A-DIRECTORY-IS-JUST-ONE-QUERY.md) | The explorer paper. Why the file manager should be a memory browser that can show a directory rather than the other way round, what that changes in the architecture, and four shippable steps to get there. | somebody working on the window |
| [`EXPLORER-KICKOFF-PROMPT.md`](EXPLORER-KICKOFF-PROMPT.md) | A session prompt for an agent picking up that work: what to read, in what order, and which rules outrank the paper. | paste it into a fresh session |
| [`BUILD-PROMPT.md`](BUILD-PROMPT.md) | The specification. Self-contained: hard rules, vocabulary, schema, formulas with their failure modes, eleven milestones with executable acceptance criteria, and the tests to write before the code. | somebody — human or agent — building it |

The papers are the argument. The build prompt is what you build. Where they disagree, the build
prompt wins, because it is the one with acceptance criteria — and the explorer paper's plan is
milestone **M10** in it.

**On the numbers.** Every measurement in §9 of the paper came from this repository on
2026-08-23 and is reproducible: `cargo run --release -p brain-sim -- --days 60` for the
simulator, `cortex consolidate --dry-run` and `cortex consolidate` for the real-index timings.
Claims that are arguments rather than measurements are marked as such, and §13 of the paper
lists what has not been evaluated at all.
