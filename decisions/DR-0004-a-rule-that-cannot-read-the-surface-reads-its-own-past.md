---
id: DR-0004
title: A rule that cannot read the surface reads its own past
status: prose-only
---

## The Measurement

In `tests/sand.rs`, 128 cells poured into a basin 62 columns wide left 4 cells carrying a direction
after `SETTLING` frames, and the bound in that file was `moving * 16 < poured`. `sand::start` reads
one 2×2 block, and whether there is somewhere lower to go is a property of the whole surface.

## The Decision

`sand::TIRED` is three bits above `sand::HELD` in the cell word counting how many times the cell has
taken a direction without descending, `sand::start` refuses a cell at `sand::PATIENCE`, `sand::fall`
clears the count because a landing cell is already stripped to its material, and `sand::held` was
narrowed from `word >> CARRIED_BITS` to one bit.

## The Rejected Route

One bit was rejected at water lying 1 to 7 deep against 2 to 3, which `tests/sand.rs` fails on; two
bits reached 2 to 3 deep and 0 cells running, and three is what is in the word.

## The Limit

The count clears on falling and on nothing else, so these figures do not show what a basin does when
water arrives beside cells that have already stopped: `tests/sand.rs` bounds the surface at one cell
of step in the scene it runs, and no other scene was run.
