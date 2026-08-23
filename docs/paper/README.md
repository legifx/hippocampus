# Papers

Two documents, written to be read in this order and published together.

| file | what it is | who it is for |
|---|---|---|
| [`FORGETTING-IS-THE-FEATURE.md`](FORGETTING-IS-THE-FEATURE.md) | The design paper. Why memory systems that only accumulate fail, what a language model actually needs from a memory, the mathematics of forgetting, what was measured, and what would falsify the design. | somebody deciding whether this architecture is right |
| [`BUILD-PROMPT.md`](BUILD-PROMPT.md) | The specification. Self-contained: hard rules, vocabulary, schema, formulas with their failure modes, ten milestones with executable acceptance criteria, and the tests to write before the code. | somebody — human or agent — building it |

The paper is the argument. The build prompt is what you build. Where they disagree, the build
prompt wins, because it is the one with acceptance criteria.

**On the numbers.** Every measurement in §9 of the paper came from this repository on
2026-08-23 and is reproducible: `cargo run --release -p brain-sim -- --days 60` for the
simulator, `cortex consolidate --dry-run` and `cortex consolidate` for the real-index timings.
Claims that are arguments rather than measurements are marked as such, and §13 of the paper
lists what has not been evaluated at all.
