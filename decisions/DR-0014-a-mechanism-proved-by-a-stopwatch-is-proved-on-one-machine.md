---
id: DR-0014
title: A mechanism proved by a stopwatch is proved on one machine
status: prose-only
---

## The Measurement

`tests/floor.rs` sweeps passes whose workgroup count the device reads out of a buffer, at 1, 2, 4, 8,
16 and 32 passes, and fits a line through each sweep. It then asserted that a pass the device switched
off costs less than one it left on — *"so what the count buffer holds is not reaching the dispatch"*.

Over three runs on each of the two devices in this machine, stable to a nanosecond or two:

```text
  RTX 4080, subgroup 32        46 ns off against 120 on   (intercepts 453 and 981)
  Radeon(TM) Graphics, 64      99 ns off against  89 on   (intercepts  44 and 248)
```

The assertion holds on one and fails on the other, every time. It is not noise and it is not a
threshold that wants widening: on the integrated part a dispatch of nought workgroups is the **dearer**
of the two, by about six nanoseconds a pass, and the whole shape of the sweep is different — an
intercept of 44 ns against 453.

And the mechanism it was written to check is not broken there. Asked by what it leaves behind rather
than by how long it takes, a count of nought in a buffer stops the pass on both devices and a count
of one runs it, over the same recorded command buffer with one word changed between the two frames.

## The Decision

`tests/dispatch.rs` gains `a_count_of_nought_in_a_buffer_stops_the_pass_that_reads_it`, which reads
the output buffer. `Groups::Fixed(0)` was already checked that way — `Resident` drops it before a
command buffer exists — and `Groups::From` is the opposite mechanism and the one this repository turns
on, so it now has a check of the same kind rather than a check of a different kind.

`tests/floor.rs` keeps the sweep and prints which way round it came out, saying so in the sentence
rather than in a comparison that has to be right. There is no number that is right on both devices,
and the interesting part is the sign.

## The Rejected Route

Widening the assertion until both devices pass is rejected. There is nothing to widen: the two are on
opposite sides of the comparison, so any bound that admits 99-against-89 admits a device on which the
count buffer genuinely is not read, which is the failure the assertion existed to catch. Deleting the
sweep is rejected too — it is the only place either device's dispatch floor is measured, and the
numbers above are why this was found at all.

## The Limit

Why an integrated Radeon charges more for a dispatch it does not run is NOT MEASURED, and nothing here
should be read as explaining it. Two devices, at subgroups 32 and 64; whether the sign goes the other
way again at 4, 8 or 16 is unknown, and the software driver the new CI runs at those widths keeps its
own clock that means nothing on a shared runner — `tests/floor.rs` skips itself there for that reason,
so CI will not answer this.

The new check proves that nought stops a pass and that one runs it. It does not prove that a count of
*n* runs exactly n workgroups; `a_workgroup_count_written_by_one_stage_is_read_by_the_next` is what
covers that, and it covers it at one value.
