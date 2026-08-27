---
id: DR-0010
title: A walk that can cross a chunk does not need a longer budget
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, one ray a pixel over 800 by 448 through a sparse voxel world in
`src/bin/vista.rs`, reading the card's own clock over a fixed flight. Stepping one voxel a turn, the
cost is flat in the budget: 381 µs a frame at 256 turns, 705 at 512, 1.37 ms at 1024 and 2.66 at
2048 — the loop's length is fixed, so every ray pays for every turn whether it is still travelling or
not.

Given a map saying which chunks hold nothing, so that a turn crosses a whole 64-voxel chunk where
there is nothing in it: 498 µs at 96 turns, 654 at 128, 797 at 160, 1.04 ms at 256 and 2.49 at 1024.
Given a second map inside each chunk, one bit for each of its 8³ blocks of 8³ voxels, so that a turn
crosses a block too: 738 µs at 96, 922 at 160, 1.25 ms at 256 and 2.18 at 512.

What that buys is read off the picture rather than the clock. From high above a landscape looking
steeply down — the case a level flight does not reach — the share of the frame that is sky, and so
the share a ray failed to reach the ground through, is 68.6% at 128 turns, 51.4% at 256 and 50.1% at
512 with chunk skipping alone. With block skipping as well it is 9.3% at 96 turns, 8.5% at 160 and
8.4% at 256: **the budget that closes the picture falls from 512 turns to 160**, and with it the card
from about 2.3 ms to 922 µs.

## The Decision

`src/bin/vista.rs` walks at three sizes in one turn and takes the largest that applies: a chunk where
the world map says the chunk is empty, a block where the chunk is held but that corner of it is
empty, and one voxel otherwise. The two larger steps are the same routine under a size argument.
Putting a ray down again at a distance needs a float turned into a word, which `simdr::module::op`
did not name, so `CONVERT_F_TO_U` and `CONVERT_F_TO_S` were added there.

## The Rejected Route

Raising the budget instead was measured and is the thing being rejected: 512 single-voxel turns costs
705 µs and reaches 512 voxels, where 160 turns of the walk above costs 922 µs and reaches as far as
the haze. Letting the loop stop early was not attempted — `simdr`'s `decisions/DR-0003` fixes a trip
count when the module is built, because one that varied by lane would diverge.

## The Limit

That the larger steps draw the same thing was checked at one camera and one budget: the same view at
1024 turns with chunk skipping on and forced off gives 339 585 of 351 116 pixels identical and 11 531
different, of which 11 529 are the plain walk running out of turns and calling the rest sky where the
skip had already reached the ground. **Two pixels differ for any other reason**, both a shade
brighter, which is a haze bucket either side of a boundary. Pixels either side was holding a chunk it
had not been given were set aside, and no other camera was diffed. Whether a third level below the
block would pay is NOT MEASURED. Every figure above was taken over a world that was a bare height
field; the same apparatus has since been given water, trees and cliffs, which puts more in each chunk
and fewer chunks entirely empty — 172 287 of 262 144 against 177 680 — so the numbers describe the
walk rather than the state the demonstration is in. The apparatus was a demonstration that was never part of the crate, and it has been removed: `src/bin/vista.rs` and the world it read are gone, so nothing here can be re-run as it stands. What is above is the record of what it measured while it existed. One card at one width is all that was measured.
