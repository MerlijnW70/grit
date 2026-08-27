---
id: DR-0016
title: A stage can be timed without waiting for it
status: prose-only
---

## The Measurement

`Resident` has always written two timestamps a submission — one before the first pass, one after the
last — and `on_device` reports their difference. Everything finer than that was a subtraction: run the
frame, run it again with a pass taken out, call the difference that pass.

The pool now holds a stamp at every stage boundary as well. A frame of four passes in three stages,
on an RTX 4080 at subgroup 32:

```text
  the whole submission   8.960 µs
    stage 0              3.200 µs
    stage 1              2.048 µs
    stage 2              2.112 µs
```

And the same thing asked for from a Python client over `quay`'s socket, over 4096 words:

```text
  waited                171.4  µs      the socket, the submission and the fence
  on the card's clock    22.75 µs
    stage 0               8.160 µs
    stage 1               6.144 µs     two passes, and no dearer than the one below it
    stage 2               6.144 µs
    the three together   20.448 µs
```

The 2.30 µs the stages do not account for is the two barriers between them, which is consistent with
the 996–1009 ns `tests/floor.rs` prices a barrier at, and is not a new measurement of one.

**Stage 1 holds two passes and costs what stage 2 costs with one.** That is what a profile is for and
it is not visible from a submission's total.

## The Decision

The query pool is sized `2 + STAGES + 1` rather than 2, and the per-stage stamps are written on the
**last repetition only**. A recording of `times` frames replays the same commands, and a query written
twice without a reset between is undefined; a reset inside the loop would have to be ordered against
writes that may still be running. Writing them once needs neither, and the last frame round is the
warm one.

`Resident::stage_times` hands back one duration a stage. `quay`'s `Spent` carries them over the wire,
so a client that cannot link against this crate can profile a frame it did not record.

`STAGES` is 64. A frame with more stages than that still runs and still reports its whole cost, and
`stage_times` answers with nothing rather than with a profile of the first sixty-four.

## The Rejected Route

A host clock around each stage is rejected and was never plausible: it needs a fence between stages,
and `tests/floor.rs` puts a submit and a fence at 41.1–43.9 µs before any work happens — five times
the whole frame above. It would not be a slow way to measure the frame; it would be a measurement of
something else.

Writing the per-stage stamps on every repetition, resetting the pool between them, is rejected on the
ordering: `vkCmdResetQueryPool` inside the loop would sit between one repetition's writes and the
next's, and nothing in the recording orders it against writes still in flight.

## The Limit

**A timestamp says when a pass ran, not what it wrote**, and nothing here would have found the race in
`decisions/DR-0013`. That hunt was perturbed by reading *buffer contents* back between frames — a copy
and a fence — and comparing what two runs left in `AWAKE` still needs exactly that. This removes the
wait from timing a stage and not from inspecting one.

The profile is the last frame of a submission and not an average of them; `on_device` is the average
and the two are different numbers. The timestamp period is 1 ns on the RTX 4080 and 10 ns on the
integrated Radeon, so a stage of a few hundred nanoseconds is a handful of ticks on the second and
should not be read as though it were the first.

Whether the boundaries land where a reader assumes is NOT MEASURED beyond what the numbers above
imply: a stamp is written at `TOP_OF_PIPE` before a stage and the last at `BOTTOM_OF_PIPE`, so what a
stage's figure holds is its dispatches and whatever the implementation lets overlap them.
