---
id: DR-0003
title: The compaction is applied to itself, above the size where it pays for itself
status: prose-only
---

## The Measurement

At subgroup 32 on an RTX 4080, three runs of `tests/coarse.rs` read two arrangements on the device
clock at 256, 512, 1024, 2048 and 4096 blocks, both of which `tests/live.rs` passes: `decay`, `mark`
and `gather` over the coarse list, at 21.88–21.89, 22.30–23.06, 22.37–22.40, 22.33–22.39 and
22.30–22.40 µs; the same three over every block with the five passes that build the list left out,
at 14.34–14.35, 15.58, 19.08–19.11, 25.32–25.42 and 37.37–37.50. A third arrangement, the list built
and not used, put building it at 6.2–6.7 µs at every size, and was measured on a patched build that
is not in the tree.

## The Decision

`crate::compact` runs a second time over `layout::HOT` — seven further bindings, `hot_mark`, two
scans, `hot_gather` and `hot_prepare` — and `crate::live::wired` emits those five passes, and
dispatches `decay`, `mark` and `gather` over the list they write, only at `crate::live::COARSE_FROM`
blocks or more.

## The Rejected Route

Building the list at every size was rejected at 21.88–21.89 µs against 14.34–14.35 at 256 blocks,
and dispatching those three over every block at every size at 37.37–37.50 against 22.30–22.40 at
4096.

## The Limit

The two cross between 1024 and 2048 blocks and no size lies between them at this width, because
`crate::sand::Grid` takes powers of two and a block count between the two is not one.
