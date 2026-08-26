---
id: DR-0003
title: The compaction is applied to itself, because a skipped workgroup is still a launched one
status: prose-only
---

**Measured on:** RTX 4080, subgroup 32.

## What was there before

`layout::HOT` holds one word a block, written by exactly the code that writes the per-quad wake
flags, so a block reading nought provably has no awake quad in it. `decay`, `mark` and `gather` each
loaded it, voted on it with `any_uniform`, and exited early where it was cold.

That flattened the middle of the table and left the largest size paying:

```text
        cells     ungated       gated
      262 144       66 us       79 us
    1 048 576       92 us       79 us
    4 194 304      161 us       79 us
   16 777 216      323 us      144 us
```

A gate makes a workgroup cheap. It does not stop it being launched. At sixteen million cells that is
three passes over 16 384 blocks — **49 152 workgroups** doing one load and a vote before they exit —
and it was the whole of what still grew with the world. `src/bin/scale.rs` runs no picture, and
nothing else in it is the size of the grid: the settle is list-driven, the brush is a fixed box, and
`wipe` and `prepare` are one workgroup each.

## The decision

**The same four passes, over `HOT` instead of `AWAKE`.** What comes out is a list of the blocks that
hold something, and `decay`, `mark` and `gather` are dispatched over that.

```text
  hot_mark     a tile x per_block blocks  ->  hot_counts[tile]  and a bucket each
  hot_scan x2                                 the tiles against each other
  hot_gather   a tile x per_block slots   ->  hot_dense[], the hot-block list
  hot_prepare  1                          ->  how many workgroups the three run
```

About a hundred and thirty workgroups to remove forty-nine thousand.

**It needs no third tier, and never will.** `Plan::over` already refuses past `per_block²` blocks, so
there are at most `per_block` tiles, and one workgroup scans `per_block` entries. The level above
this one is a single dispatch by construction rather than by luck.

**A tile and a coarse block are the same partition.** The fine hierarchy's tile is `per_block`
blocks, and so is what one coarse workgroup sweeps. `Plan` carries one field for both, because they
are the same division of the same array rather than two numbers that happen to agree.

```text
        cells       gated      listed
      262 144       79 us       57 us
    1 048 576       79 us       57 us
    4 194 304       79 us       59 us
   16 777 216      144 us       60 us
```

Lower than the gated floor at *every* size, because the blocks being launched and dismissed were
never free, only cheap.

## Three things had to move with it, and each was a way to be wrong

**`mark` could no longer zero what it skipped.** It used to write `counts[block] = 0` before its gate,
so a skipped block said nothing rather than last frame's something. A block that is never dispatched
cannot write that — and `scan_counts` reads *every* block, because a prefix has to see the zeros. One
stale count inflates every base after it and the grand total with them. So `wipe_counts` is a pass of
its own, one workgroup a tile.

**`HOT` had to be padded to whole tiles,** because the coarse sweep reads a tile at a time and would
otherwise read past the end. Its dump slot moved past the padding with it: inside, it would have been
a hot block that no quad ever woke, listed every frame for ever.

**`HOT_TOTALS` is scanned in place like `TOTALS` is,** so `wipe` clears both. Without it the grand
total climbs for ever — the same failure that cost this crate a work list once already, arriving by a
different road.

## What proves it

`tests/live.rs` runs the same pour twice, once through the work list and once over every quad, and
demands the two grids be identical cell for cell. A block wrongly left off the hot list is a grain
that stops in one run and keeps falling in the other.

That test could not reach the coarse tier's upper tiers on its own. Its world is 512×256, which is
32 768 quads; on a thirty-two-lane part a block is 256 quads, so that is 128 blocks and **one tile**
— and the coarse scan's second and third terms are a base of nought added to a base of nought there.
A bug in either would go straight past. On an eight-lane part a block is 64 quads, so the same world
is 512 blocks in eight tiles, which makes the gap a property of the device rather than of the test.

`tests/hot.rs` closes it: a thousand blocks, which is more than one tile at every width this crate
builds for and not a whole number of them, so the padding is exercised too. Two frames, because a
scan that writes over its own input only goes wrong on the second — proved by removing `wipe`, which
reports **46 336** hot blocks on frame two where there are 181.
