---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

**Measured on:** RTX 4080, subgroup 32, as `Host::name` reports it.

## The decision

**Nothing here is optimised until it has been priced, and a price is obtained by putting something
*in* rather than taking something out.** Taking a pass out of a working loop changes what the loop
computes, so the timing that comes back belongs to a different program.

Two removals in this session produced a better number and a different program:

* **`DRAW_INDIRECT` dropped from every barrier.** Reported 28.6 / 29.1 / 29.9 / 34.0 µs against a
  baseline of about 44 — a third off. It was doing less work: the settle was dispatching over a
  stale count, and `listed` fell from 768 to 468 and 471.
* **Five passes set to `Groups::Fixed(0)`,** which removes their dispatches and their barriers.
  Reported 41.2 / 45.9 / 65.1 / 141.1 µs against a baseline of 60.2 / 59.0 / 59.8 / 60.7. The
  smallest size was the signal being looked for; the largest had **got worse**, because without
  those five passes the hot-block list is never rebuilt and three passes fall back to a workgroup a
  block.

`src/bin/scale.rs` prints `listed`, how many quads the settle actually stepped, and it caught the
first of those two on sight. **It did not catch the second** — the fallback changes how many
workgroups are dispatched, not how many quads are listed. So the canary is necessary and not
sufficient, and a removal has to be reasoned about as well as watched.

## The apparatus

Two of them, and their agreeing is most of what makes the numbers believable.

**One inside the real loop.** Add N passes that write one word nobody reads. The simulation is
untouched, `listed` stays at 768, and the difference is the passes:

```text
  five idle passes, one stage each     +7.5 us      five dispatches and five barriers
  five idle passes, all in one stage   +1.1 us      five dispatches and no barriers
```

which is 1.28 µs a barrier and 0.22 µs a dispatch.

**One with no simulation in it at all.** `tests/floor.rs` holds eight passes that write one word and
varies the only thing that separates a submission's cost from a frame's — how many frames go into
one — then fits a line and reads the intercept and the slope. Four runs:

```text
  a submission, holding nothing          35-42 us
  one more frame in it, holding nothing        0
  a dispatch                            210-245 ns
  a barrier                            996-1009 ns
  asking the device for one word         41-42 us
```

The two give 1.28 µs against about 1.00 for a barrier and 0.22 against 0.23 for a dispatch, by
different means over different code. That agreement is the point of having two.

**The spreads are a finding too.** Across those four runs a barrier came back within 13 ns of itself
— 996, 1004, 1005, 1009 — while a submission moved by six and a half microseconds, 35.2 to 41.7.
That is what a pipeline flush looks like next to a queue and a fence.

## What a frame is made of

Three of these four lines are the price list multiplied out. The fourth is what is left when they
are subtracted from the measured 25 µs, and it is a **residual rather than a measurement** — nothing
here has timed a kernel on the device's own clock.

```text
  ~10 us   ten barriers            measured: 10 x ~1.00 us
  ~ 4 us   seventeen dispatches    measured: 17 x ~0.23 us
  ~ 2 us   the submission          measured: ~37 us, over twenty frames
  ~ 9 us   everything else         a subtraction, not a reading
```

Four separate changes took the frame from 60 µs to 25, and the order they happened in is itself the
lesson: each became visible only once the one before it was gone.

```text
        cells     ungated       gated      listed      staged     narrowed        kept
      262 144       66 us       79 us       57 us       44 us        35 us       25 us
    1 048 576       92 us       79 us       57 us       44 us        35 us       26 us
    4 194 304      161 us       79 us       59 us       44 us        36 us       26 us
   16 777 216      323 us      144 us       60 us       46 us        36 us       27 us
```

## The one that was priced and declined

`hot_scan_counts` and `hot_scan_totals` are two dispatches of **one workgroup** with a barrier
between them — `Plan::over` refuses past `per_block²` blocks, so there are at most `per_block` tiles
and `hot_tiles` is always one. They could be a single kernel with a workgroup barrier inside.

Against the price list that buys one barrier and one dispatch: **about 1.2 µs of 25**, for a
coarse-only kernel that breaks the arrangement where `compact::scan` serves both tiers identically.
Declined, and written here rather than left as an idea somebody has again.

## The cost, stated

`tests/floor.rs` asserts **shapes and not numbers** — that a submission costs more than a frame
inside it, that a barrier costs more than a dispatch, that neither is free. A number from this
machine on this day is not a thing to assert on, and a test that pinned one would fail on somebody
else's card with nothing broken.

So the price list above is guarded by nothing. It is dated, and the way to learn whether it still
holds is to run the test and read what it prints.
