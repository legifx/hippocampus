# review/ — periodic reviews of the whole against the original goal

One review per date. Each has a Markdown paper for people and agents, and a JSON file with
every number the paper cites, so the next review can diff against it rather than re-derive.

| date | commit | reviewer | paper | evidence |
|---|---|---|---|---|
| 2026-09-05 | `bbef90d` | Claude Fable 5.1 | [2026-09-05-review.md](2026-09-05-review.md) | [2026-09-05-evidence.json](2026-09-05-evidence.json) |
| 2026-09-05 (2nd pass) | `8ae1ff8` | Claude (claims-verified) | [2026-09-05-code-review.md](2026-09-05-code-review.md) | [2026-09-05-code-review-evidence.json](2026-09-05-code-review-evidence.json) |
| 2026-09-05 (3rd pass) | `45367ca` | Gemini 3.8 Flash (High) | [gemini-3.8-flash-high/2026-09-05-review.md](gemini-3.8-flash-high/2026-09-05-review.md) | [gemini-3.8-flash-high/2026-09-05-evidence.json](gemini-3.8-flash-high/2026-09-05-evidence.json) |
| 2026-09-06 (response) | `028f14b` + | Claude Fable 5.1 | [2026-09-06-response.md](2026-09-06-response.md) — what was done about each finding of the three papers above, with the numbers re-taken | (numbers inline) |

Reviews are also filed per reviewing model under `review/<model>/`, so that reviews by
different models of the same commit can be compared side by side:
`review/claude-fable-5.1/` holds the initial review;
`review/gemini-3.8-flash-high/` holds the 3rd pass (empirical deep-dive on disambiguation, memory bloat, Playwright e2e/perf, and agent-brain reality check).

The second pass of 2026-09-05 is **claims-verified**: it re-runs every command and reads the code
behind the first pass's claims. Read the first paper for what exists; read the second for whether
the evidence for it holds. Where they disagree, the second one's numbers were measured later and
on the same machine — its §1 lists three contradictions (red Rust suite, environment-dependent red
frontend suite, no CI at all).

**For an agent picking this up:** read the newest paper's §4 (gaps, ranked) and §5 (what to do
next, in order) first; re-run §6 to refresh the numbers before trusting any of them, because
they were taken on one machine on one day. The original goal the reviews measure against, in
the owner's words: a file explorer in which a human and an AI agent both work and search
perfectly, efficiently and without lag — the human finds exactly the file or path at once,
sees everything at a glance, and can verify by preview that it is the right file; the agent
uses the same explorer as its brain and database (Hippocampus).
