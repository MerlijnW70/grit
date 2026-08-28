---
id: DR-0018
title: A flag that counts is not a flag that marks
status: prose-only
---

## The Measurement

`decisions/DR-0013` left the coarse arrangement wandering and said what it had ruled out: not the
world, not the stage grouping, not a duplicate in the list. What it could not say was **where**, and
its instrument could only see the wander by running the same thing twice and finding two answers.

There is an invariant that needs one run. [`live::wake`] writes the same two to a quad's counter in
`AWAKE` and to its block's flag in `HOT`, in one invocation; `decay` takes one off both, in one
invocation. So **a flag is at least the highest counter in its block, always**. `tests/wander.rs`
checks that, and everything below is one of its numbers on the AMD Radeon(TM) Graphics at subgroup
64, 1024 blocks, 2048×1024 cells.

**It is violated, and where it is violated is where the runs disagree.** Over 150 frame counts of
eight runs each, eight frame counts held a run whose flag was below its counters — and those same
eight, and no others of the 150, are the frame counts at which the eight runs' wake counters
disagreed. The violation is not a symptom of the wander; it is the wander, seen without a second run
to compare against.

**It is not the coarse list.** Run with no coarse hierarchy at all — `decay`, `mark` and `gather`
over every block, which is the other half of `live::arranged` and what `tests/live.rs` runs — the
flag still fell below its counters, at two frames, in blocks 120, 121 and 123 — 40 of 128 runs in
one measurement of it and 42 of 128 in another.
The list does not cause it. What the list does is keep it: a block whose flag reads nought is not
listed, so it is never dispatched for again, its counters freeze where they are, and its quads never
reach a work list. Over every block the same block is visited next frame regardless and the slip
heals in one frame, which is why the every-block arrangement looks steady and is not.

**It is not a barrier.** One pass a stage — every barrier the ordering could want — holds it at 33 of
128 against 42 of 128. Naming `DRAW_INDIRECT` in every barrier rather than only behind the stage
that wrote a count: `tests/hunt.rs` wanders the same either way. A barrier behind the **last** stage
of a recording, so that a frame's writes are ordered against the next submission's first stage:
2 of 400 runs before it and 3 of 400 after, at a frame a submission.

**It is one pass, and it is one line.** `decay` alone, dispatched over a list naming each of 1024
blocks exactly once, nothing else running. Four hundred rounds, every second one over that list, so
204 800 blocks a measurement:

```text
  counters and flag start at   right      flag one lower     anything else
  2                            204 800    0                  0
  2                            204 794    6                  0
  5                            204 798    2                  0
  5                            204 798    2                  0
```

Five rather than two because at two a second decrement and a stale read leave the same nought. At
five the flag came back at three where the counters came back at four, so it is **a second
decrement**, not a read that saw an older value. The line it comes from is the flag's own countdown:

```rust
let held = kernel.load_at(HOT, block)?;   // by every lane of the workgroup,
let lower = step(&mut kernel, held)?;     // at one address,
kernel.store_at(HOT, block, lower)?;      // read-modify-write
```

A quad's counter is read and written by one lane. The block's flag was read and written by all
sixty-four, and one of them read what another had already written back.

## The Decision

**The flag is computed from the counters instead of stepped beside them.** `decay` already steps
every counter in the block; it now keeps the highest as it goes, reduces that across the subgroup,
and stores it. There is no load of `HOT` left in the pass, so there is nothing for a second read to
disagree with, and every lane stores the same number — which is the case `layout::HOT` documents as
safe, and which a read-modify-write never was.

The two are the same number by construction, so this is not a new rule: `wake` writes the same two to
both, and nothing else raises either. Saying it out loud is what makes it safe.

What it is worth, on the same 1024 blocks: a hundred `decay` dispatches a submission, ten
submissions a measurement, three measurements a side, timed on the device rather than the host.

```text
  AMD Radeon(TM) Graphics, subgroup 64     before  3623-3643 ns    after  2426-2457 ns
  NVIDIA GeForce RTX 4080, subgroup 32     before    69-92   ns    after    69-91   ns
```

**It is cheaper.** Sixty-four lanes loading one word and storing to it serialise; a reduction and one
store do not. On the 4080 nothing moved.

And `tests/hunt.rs`, which is the whole of DR-0013, now reads the same on every line: four
groupings, five runs each, work list 1130 every time, coarse list 230 every time, `AWAKE`, `HOT` and
the world identical across all twenty runs. `tests/wander.rs` finds nothing on either device — the
150-frame scan, 400 runs at each of three batch sizes, 768 runs over the first six frames, and
204 800 blocks of the isolated pass.

## The Rejected Route

`reduce_max::<U32, 1>` was the first thing written and it is not a reduction. A one-wide vector on a
sixty-four-lane subgroup maps to *clusters of one*, and a clustered reduction over a cluster of one
hands back what it was given — so every lane kept its own highest counter and the flag was whichever
lane's store landed last. It passed the probe that gives each block a ramp of counters, because
there the highest counter is in the last lane and the last lane's store is the one that lands; and
it failed 128 runs of 128 the moment the awake quads sat anywhere else. The width the reduction
folds over has to be the width the workgroup is, which is what `decayed`'s `LANES` already is.

The trailing barrier described above is not in the tree. It is defensible on the specification —
nothing orders one submission's last stage against the next submission's first — but it was measured
and it moved nothing, and this repository does not carry a barrier that has not paid for itself.

## The Limit

**Both cards, one shape.** Everything here is 1024 blocks at subgroup 64 and subgroup 32, one pour
into one room. Whether the same slip existed at subgroup 4, 8 or 16 is unmeasured: those widths run
on a software driver whose timing is nothing like either card's, and it never showed this.

**Why the world never moved.** `tests/live.rs` compares cells and finds nothing, before or after,
and DR-0013 already said why: a quad that is stepped when it had nothing to do leaves the same cell
behind. The frozen block is a latent hazard rather than an observed wrong pixel — nothing in this
measurement shows a grain stopping because of it. What it did show is a work list that is not a
function of the world, and that was enough to be worth taking apart.

**One test still fails on the Radeon, and it is not this.** `tests/coarse.rs` asserts that below
`live::COARSE_FROM` the every-block arrangement is the cheaper of the two, and at 1024 blocks on
this part it is not. Over five attempts a side, the ones that reached the row before the assertion
stopped them: 141.1-143.5 µs for the list against 524.7-530.2 µs for every block before this change,
and 102.0-103.7 against 306.5-310.5 after. It fails at `HEAD` without this change too, so it is a
threshold taken on the other card and not a consequence of anything here. It is left where it is.
