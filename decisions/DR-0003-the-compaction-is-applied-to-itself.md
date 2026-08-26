---
id: DR-0003
title: The compaction is applied to itself, above the size where it pays for itself
status: prose-only
---

## The Measurement

At subgroup 32 on an RTX 4080, and with `tests/live.rs` passing in all three, three arrangements of
`src/bin/scale.rs` read on the device clock over three runs each at 262 144, 1 048 576, 4 194 304 and
16 777 216 cells: `decay`, `mark` and `gather` dispatched over the coarse list, 24.40–24.49,
24.95–25.03, 24.95–25.04 and 25.13–25.18 µs; the same three over every block with the list still
built, 20.81–20.85, 25.61–25.64, 43.63–43.66 and 115.44–115.50; and the same three over every
block with the five passes that build it removed, 14.38–14.45, 19.16–19.19, 37.41–49.02 and
108.81–108.86. Building the list costs 6.2–6.7 µs at every size, and the two arrangements cross at
2048 blocks, where `tests/coarse.rs` reads them at 25.0 and 25.4 µs and the order changes between
runs.

## The Decision

`crate::compact` runs a second time over `layout::HOT` — seven further bindings, `hot_mark`, two
scans, `hot_gather` and `hot_prepare` — and `crate::live::wired` emits those five passes, and
dispatches `decay`, `mark` and `gather` over the list they write, only at `crate::live::COARSE_FROM`
blocks or more.

## The Rejected Route

Building the list at every size was rejected at 24.40–24.49 µs against 14.38–14.45 at 256 blocks,
and keeping the vote as the only filter at 108.81–108.86 against 25.13–25.18 at 16 384.

## The Limit

`COARSE_FROM` is where the two crossed on one card at subgroup 32, no other width was measured, and
every figure here is one spout for 300 frames, so the awake set does not grow with the grid.
