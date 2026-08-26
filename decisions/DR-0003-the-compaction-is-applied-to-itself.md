---
id: DR-0003
title: The compaction is applied to itself, because a skipped workgroup is still a launched one
status: prose-only
---

## The Measurement

At 16 777 216 cells `src/bin/scale.rs` ran 144 µs a frame with `decay`, `mark` and `gather`
dispatched one workgroup a block and gated on `layout::HOT` — 3 × 16 384 workgroups that load one
word, vote, and exit. Nothing else in that file scales with the grid: the settle reads its count from
a buffer, `sand::brush` is 32 × 32 cells, and `wipe` and `prepare` are one workgroup.

## The Decision

`crate::compact` was run a second time over `layout::HOT` instead of `layout::AWAKE` — seven further
bindings, `hot_mark`, two scans, `hot_gather` and `hot_prepare`, about 130 workgroups — and `decay`,
`mark` and `gather` are dispatched over the list it writes, which took 144 µs to 60 and 57 µs to 57
at 262 144 cells.

## The Rejected Route

Keeping the vote as the only filter was rejected at 144 µs against 60 at 16 777 216 cells, the vote
being retained in `gather` where the count it reads was loaded for other reasons.

## The Limit

These figures are one spout in one place for 300 frames at every size, so the awake set is about 768
quads whatever the grid is: they do not show what the frame costs when the awake set is a large
fraction of the world.
