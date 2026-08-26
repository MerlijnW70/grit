---
id: DR-0003
title: The compaction is applied to itself, because a skipped workgroup is still a launched one
status: prose-only
---

## The Measurement

At subgroup 32 on an RTX 4080, and with `tests/live.rs` passing in all three, three arrangements of
`src/bin/scale.rs` read on the device clock over three runs each at 262 144, 1 048 576, 4 194 304 and
16 777 216 cells: `decay`, `mark` and `gather` dispatched over the coarse list, 24.40–24.49,
24.95–25.03, 24.95–25.04 and 25.13–25.18 µs; the same three over every block with the list still
built, 20.81–20.85, 25.61–25.64, 43.63–43.66 and 115.44–115.50; and the same three over every block
with the five passes that build it removed, 14.38–14.45, 19.16–19.19, 37.41–49.02 and
108.81–108.86. Building the list costs 6.2–6.7 µs at every size.

## The Decision

`crate::compact` runs a second time over `layout::HOT` — seven further bindings, `hot_mark`, two
scans, `hot_gather` and `hot_prepare` — and `decay`, `mark` and `gather` are dispatched over the list
it writes.

## The Rejected Route

Keeping the vote as the only filter was rejected at 108.81–108.86 µs against 25.13–25.18 at
16 777 216 cells.

## The Limit

Below 4 194 304 cells the arrangement costs more than it saves — 10 µs of 24 at 262 144 and 6 µs of
25 at 1 048 576 — no size in between was measured, and every figure here is one spout for 300 frames,
so the awake set is about 768 quads whatever the grid is.
