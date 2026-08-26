---
id: DR-0003
title: The compaction is applied to itself, above the size where it pays for itself
status: prose-only
---

## The Measurement

At subgroup 32 on an RTX 4080, three runs of `tests/coarse.rs` read two arrangements on the device
clock at 256, 512, 1024, 2048 and 4096 blocks, both of which `tests/live.rs` passes: `decay`, `mark`
and `gather` over the coarse list, at 24.29–24.40, 24.76–24.83, 24.78–24.85, 24.53–24.86 and
24.49–24.87 µs; the same three over every block with the five passes that build the list left out,
at 14.37–14.41, 15.62–15.69, 19.16–19.20, 24.81–25.44 and 36.84–37.61. A third arrangement, the
list built and not used, put building it at 6.2–6.7 µs at every size, and was measured on a
patched build that is not in the tree.

## The Decision

`crate::compact` runs a second time over `layout::HOT` — seven further bindings, `hot_mark`, two
scans, `hot_gather` and `hot_prepare` — and `crate::live::wired` emits those five passes, and
dispatches `decay`, `mark` and `gather` over the list they write, only at `crate::live::COARSE_FROM`
blocks or more.

## The Rejected Route

Building the list at every size was rejected at 24.29–24.40 µs against 14.37–14.41 at 256 blocks,
and dispatching those three over every block at every size at 36.84–37.61 against 24.49–24.87 at
4096.

## The Limit

`COARSE_FROM` is where the two crossed on one card at subgroup 32, no other width was measured, and
every figure here is one spout for 300 frames, so the awake set does not grow with the grid.
