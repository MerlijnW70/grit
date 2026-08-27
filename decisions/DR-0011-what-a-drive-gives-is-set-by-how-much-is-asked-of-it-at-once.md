---
id: DR-0011
title: What a drive gives is set by how much is asked of it at once
status: prose-only
---

## The Measurement

On this machine's F: drive, reading 1 MiB chunks at scattered offsets out of a 256 GiB file into an
RTX 4080, driven by what a compute pass wrote into a buffer saying which chunks it could not draw
through — `src/bin/vista.rs` over a fixed flight.

One handle a thread, with the number of chunks allowed to be on their way at once held at 96: 0.38
GiB/s sustained at 4 threads, 0.72 at 8, 0.90 at 16 and 0.95 at 32.

Holding the threads at 32 and moving that in-flight cap instead, against how much of the picture was
still waiting on a chunk it had asked for: at a haze horizon of 1600 voxels, 31.98% of the frame
waiting at 96 and 2.99% at 256; at a horizon of 1000 voxels, 0.03% at 256 and the same 0.03% at 512.
The sustained rate over the same flight went from 0.70 GiB/s to 2.12. **The cap was the rein, not the
drive and not the threads** — it left thirty-two handles idle while there was more to fetch than it
would let be asked for.

## The Decision

The pump asks for as many chunks as the working set needs rather than as many as it can carry, and
what bounds the working set is the haze: a ray that reaches a missing chunk further away than the
distance at which haze has swallowed it whole does not ask for it, because what would be drawn there
is the colour of the sky either way. The slot given up when the card is full is the one furthest from
where the eye is looking, by the same score that orders what is fetched first.

## The Rejected Route

Reading the counter first and then only the entries it names, rather than the whole quarter-megabyte
list, measured no faster: three runs each with nothing else on the card gave 84.27, 80.57 and 73.92 µs
a frame for the whole list against 81.04, 79.97 and 74.60 for the counter and then its entries. That
cost is waiting for the card to be finished and not the bytes, so the simpler one read is kept.
Raising the in-flight cap past 256 is rejected at 0.03% either way.

## The Limit

One drive, one file, one access pattern; whether a sequential read of the same file would show the
same shape is NOT MEASURED here, and `hyperia`'s `bin/pipe` puts drive to card at 3.22–3.28 GiB/s
sequentially, which this does not reach. The share of the frame still waiting is a property of the
flight and the scene as much as of the pump — flown by hand at speed the same build left 2.804% of
its last frame waiting where the fixed flight left 0.03%. The two sweeps above were taken under
different builds of the walk and are not two points on one curve, and both were taken over a world
that was a bare height field, before it was given water, trees and cliffs. The haze horizon the
demonstration now runs at is 900 voxels rather than either of the two swept, because the camera was
moved down onto the ground in between and what a camera can see from there is not what it could see
from five hundred voxels up. The apparatus was a demonstration that was never part of the crate, and it has been removed: `src/bin/vista.rs` and the world it read are gone, so nothing here can be re-run as it stands. What is above is the record of what it measured while it existed.
