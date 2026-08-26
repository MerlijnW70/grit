---
id: DR-0005
title: What is ordered against what is the caller's to say, and the recording is not
status: prose-only
---

**Measured on:** RTX 4080, subgroup 32.

## The decision

`Resident` records a frame into one command buffer, with a barrier between passes and one submission
around the lot. Three things about that were decided in this session, and they are one decision seen
three times: **the caller knows things about its own frame that `Resident` cannot work out, and every
one of them was being paid for in barriers.**

## One: a barrier goes between stages, not between passes

Passes were a flat list and every one got a barrier after it. A barrier is a full pipeline flush; at
the time it cost about 3.8 µs, and sixteen of them was very nearly the whole frame.

Two passes may share a stage exactly when neither reads what the other writes, and that is a question
about the *frame*, not about `Resident`. `crate::live::wired` answers it — sixteen passes in eleven
stages, with the argument for every seam beside them:

```text
  tick    -> PARAMS[0], which settle reads for the partition's phase
  decay   -> AWAKE and HOT, which brush writes too and must not race with
  brush   -> STATE, which settle reads
  settle  -> STATE, AWAKE, HOT, which everything below reads
  the two hierarchies, each tier reading the tier above it
```

`wipe` and `wipe_counts` join `tick` and `decay` because the four touch no buffer in common;
`prepare` joins `gather` at both tiers because one writes a list and the other a workgroup count.
`paint` joins the coarse mark: it reads the state the settle wrote, writes pixels nothing else
touches, and at one invocation a pixel is the largest fixed dispatch in the loop.

**The machinery already existed.** `Resident::submit` had taken stages since `1c38b88` and
`tests/dispatch.rs` had tested the semantics since. Nothing used it.

44 µs, from 60.

## Two: `DRAW_INDIRECT` belongs in four of the ten barriers

A workgroup count is read at that stage rather than in a shader, so a barrier naming only
`COMPUTE_SHADER` orders the buffer's contents and not the command that reads them. Real hazard, and
why the flag was there in all ten. Four of the ten precede such a dispatch. The other six were paying
for an ordering nothing needed.

The question is one `matches!` over the stage about to run, asked while the command buffer is being
recorded — so it cannot go stale, because the stage list *is* what is being recorded.

35 µs, from 44. `DR-0002` has the comment that had said this cost nothing.

## Three: a recording is a function of three things, and none of them is data

Nothing that changes between frames is in a recording. A dispatch names a pipeline and a count; a
descriptor set names buffers. What a frame differs by is what those buffers *hold*, which a command
buffer does not carry — and `Groups::From` is the sharpest case, since even the count is read out of
a buffer at execution.

So a recording is a function of the passes, their grouping, and how many frames were asked for.
`Recorded` is those three, compared on every call. `ONE_TIME_SUBMIT` had to go, because that flag says
the recording may be invalid after one submission; `SIMULTANEOUS_USE` is not needed in its place,
because two submissions never overlap — there is one fence and `start` refuses a second while the
first is outstanding.

**It bought nothing at first, and the counter said why in one line:**

```text
  recorded 16 times, reused 0
```

A staged write is a recorded copy and it went into the *frame's* command buffer, so the four brush
words `scale` writes between batches threw the frame away every time. Transfers have their own buffer
now — the pool allocates two — and the frame survives them: `recorded 2 times, reused 14`, the two
being the warm-up and the first batch, which differ in how many frames they hold.

25 µs, from 35.

`Resident::reused` exists so that the next person does not have to wonder. A caller that hands over
the same passes every frame and measures no reuse has a bug rather than a slow frame: something in
its stage list is changing that it did not mean to change.

## What checks all three

`tests/live.rs`, and it is the right shape for it. A stage merged that should not have been corrupts
the *listed* run's compaction and not the whole-grid run's, so it shows as a divergence — six
thousand frames, cell for cell.

Two things it does not reach got tests of their own in `tests/dispatch.rs`, which had no
`Groups::From` in it at all before this:

* a count written by one stage is the count the next is dispatched over. 37 asked for, 1 seeded
  underneath, and the tally says which was read. Forcing the flag off reports *the dispatch read 1
  workgroups, not 37*.
* a frame is recorded once and a write between two does not undo it. Sharing the command buffer again
  does not merely re-record — it submits the *transfer's* commands as the frame, and the test reports
  a stale answer. Never keeping the recording reports two recordings where one was asked for.

## The cost, stated

`Resident` now holds a `Vec<Vec<Groups>>` and compares it on every call. Seventeen comparisons
against 7 µs of recording is not a trade worth thinking about, but it is not nothing, and a caller
whose stages differ every call pays the comparison and records anyway.

And the grouping is a *claim by the caller* that `Resident` cannot check. A wrong one is a race, not
an error. What makes that acceptable here is that `tests/live.rs` is a different algorithm rather
than a second opinion; in a crate without such a test it would not be.
