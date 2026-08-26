---
id: DR-0002
title: A comment that has never been run is a guess, and five of them here were wrong
status: prose-only
---

**Measured on:** RTX 4080, subgroup 32.

## What happened

This repository writes down its reasoning at the point of use, at length, and that is most of what
makes it readable. It also means a claim can sit beside the code it is about for months looking
exactly like a measurement.

Five did. Every one was checked in a single session and every one was wrong — not subtly, and not in
a direction that flattered the code.

## The five

**"Naming it always costs nothing and cannot be forgotten for the one pass that needs it."**
`src/resident.rs`, about putting `DRAW_INDIRECT` in every barrier's destination mask. The second
half is true. Of the frame's ten barriers, four precede a dispatch that reads its workgroup count
out of a buffer; naming it in the other six cost **44 µs against 35**, a fifth of the frame. Now
asked per stage, and `tests/dispatch.rs` pins the asking — proved by forcing it off, which reports
*the dispatch read 1 workgroups, not 37*.

**"One test to a binary is the whole of the fix."** `tests/waiting.rs`, about a timing test that had
been given a file of its own because tests in one binary run at the same time. Cargo runs test
*binaries* in parallel too, so a file of its own bought nothing once the crate had eleven. In its own
binary, exactly as designed, **two runs in three failed**. Two things were wrong: the margin was
0.45 ms where `thread::sleep` on Windows moves by two or three tenths, and one reading is not a
measurement.

**"Asked immediately after `queue_submit` on a frame with real work in it, that is not a race."**
`tests/wedged.rs`, about a fence waited on with a zero timeout. It is a race: the host can lose its
timeslice between the submit returning and the wait being called, and a quarter of a megabyte
finishes in that window. About **one run in eight** where the file is vendored into a harness of two
hundred. A bigger frame only moves the window; an `Ok` frame poisons nothing, so it asks again.

**"Which turns out to be almost none of it."** `src/resident.rs`, about the time spent recording a
command buffer, from the commit that added the counter. It was true when a frame was a hundred
microseconds of submit and fence. Once the round trip was spread over twenty frames it was **7 µs of
35**, a fifth — and the structure never changes between calls, which the same comment said.

**"About a hundred microseconds."** In **thirteen places**, counted at `9eb4577`. It comes from
`simdr`'s `DR-0008`, which measured *"the submission, the fence and the copy back"* — and two of the
thirteen cite it faithfully as that. The other eleven use it for a bare submit and a fence, or for
reading one word back, neither of which it measured.

`DR-0008` is still about right for what it measured: `src/bin/loop_cost.rs` gives 95 µs for a frame
that copies a megabyte back and 50 for the same frame copying nothing. A bare submit and fence is
**35–42 µs** and one word back is **41–42**, by `tests/floor.rs`.

## What they have in common

Four of the five are about **the cost of something**, and none of them had a number beside it. The
fifth — the race — is about a window, and nobody had counted how wide.

None was written carelessly. Each is a plausible thing to believe, stated by somebody who understood
the mechanism; "a flag in a barrier mask costs nothing" is what one would expect, and it is what a
driver could reasonably have done. That is exactly why they lasted.

## The rule this leaves

**A comment that asserts a cost gets a number, or it says it has no number.** There is a difference
between

> naming it always costs nothing

and

> naming it should cost nothing and has not been measured

and the second is worth writing, because it is an invitation rather than a full stop. The first is
what stopped anybody looking for four of these.

Where a number is given, it says what produced it. `tests/floor.rs` exists so that the ones about
Vulkan have somewhere to come from.

## The sweep, once the rule existed

The five above were found by walking into them. A rule that is only applied where somebody happens
to look is not a rule, so the source was swept for the shape — *costs nothing*, *is free*, *cheap*,
*negligible*, *for nothing* — and every hit read.

Most were noise: `free_diagonal`, `right_free`, a variable name. Of what was left:

* **One was a plain error, and mine.** `sand::PATIENCE` said the cell word "uses seven of its
  thirty-two" bits. It is eight — two of material, two of direction, one of memory, three of
  fatigue. It had already been corrected in `DR-0004` and not in the source, which is the same
  failure this record is about wearing different clothes.
* **One overreached.** `crate::gate` said the gather's gate "costs nothing at all" because the count
  it votes on was already loaded. The load is free; the vote and the branch are not, and nothing has
  measured them. It now says that.
* **One is the same claim as the worst of the five, about the same feature, in the same file.**
  `Resident` declares `INDIRECT_BUFFER` on every buffer because any of them may end up holding a
  workgroup count, and said doing so "costs nothing". It is a creation-time flag and cannot reach a
  recorded dispatch — but it can reach `vkGetBufferMemoryRequirements`, and that has not been looked
  at. It now says *that*, rather than either measuring it or claiming it.
* **The rest were true in context.** `live::tally`'s "adding nought is free" is about the *value*
  being added, and the paragraph under it already prices the instruction at one atomic a workgroup.

So the sweep found one error and two overreaches, and no second `DRAW_INDIRECT`. That is the useful
outcome either way: the five were the ones that mattered, and now it is known rather than hoped.

## The cost, stated

None of the five changed a design. `compact::prepare` still exists so a workgroup count never comes
back to the host — 41 µs to learn one number is more than the whole frame that produced it, which is
a **stronger** argument than the comment was making. The wake set, the partition, the two
hierarchies: untouched.

What they cost was 22 µs of frame and two tests that failed for reasons that had nothing to do with
what they were testing.
