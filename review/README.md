# review/ — periodic reviews of the whole against the original goal

One review per date. Each has a Markdown paper for people and agents, and a JSON file with
every number the paper cites, so the next review can diff against it rather than re-derive.

| date | commit | paper | evidence |
|---|---|---|---|
| 2026-09-05 | `bbef90d` | [2026-09-05-review.md](2026-09-05-review.md) | [2026-09-05-evidence.json](2026-09-05-evidence.json) |

**For an agent picking this up:** read the newest paper's §4 (gaps, ranked) and §5 (what to do
next, in order) first; re-run §6 to refresh the numbers before trusting any of them, because
they were taken on one machine on one day. The original goal the reviews measure against, in
the owner's words: a file explorer in which a human and an AI agent both work and search
perfectly, efficiently and without lag — the human finds exactly the file or path at once,
sees everything at a glance, and can verify by preview that it is the right file; the agent
uses the same explorer as its brain and database (Hippocampus).
