---
id: DR-0013
title: The coarse arrangement does not reach the same place twice
status: prose-only
---

## The Measurement

The machine this is written on holds two devices — `["NVIDIA GeForce RTX 4080", "AMD Radeon(TM)
Graphics"]`, subgroup 32 and 64 — and `tests/dispatch.rs` said in prose that the whole suite passes on
both. Setting `GRIT_DEVICE=AMD` and running it is one line and had not been run.

On subgroup 64, at 1024 blocks, **the coarse arrangement run twice in one process does not reach the
same place**: 1128 quads and then 1130, 1130 and then 1128, 1125 and then 1130. Every-block answers
1130 every time, and on subgroup 32 the coarse arrangement does too, in four runs of four.

`tests/hunt.rs` takes that apart. Five runs of each arrangement, every buffer read **once at the end**
so that the instrument is not part of what it measures, on the same 1024 blocks over 300 frames:

```text
                                    lists                world   AWAKE     HOT       coarse list
  subgroup 64, as arranged groups   1130 1127 1127 …     same    differ    differ    230 229 229 …
  subgroup 64, one pass a stage     1130 1128 1128 …     same    differ    differ    230 229 229 …
  subgroup 32, either grouping      1130 ×5              same    same      same      230 ×5
```

Four things follow, and none of them is what this looked like from `tests/coarse.rs`.

**The world is right.** All 2 097 152 cells, hashed, identical across every run of every grouping on
both devices. What wanders is the wake bookkeeping — `AWAKE`, the counters, and `HOT`, the coarse
flags — and the work list that is derived from them. A quad that is stepped when it had nothing to do
leaves the same cell behind, which is why `tests/live.rs` compares cells for four hundred frames and
finds nothing.

**It is not a missing barrier.** Run with every pass in a stage of its own — every barrier the
ordering could want, sixteen instead of eleven — it wanders exactly as much. A stage grouping that was
wrong would have been fixed by that and was not.

**It is timing-sensitive, and reading changes it.** Comparing one word after each frame, the two
arrangements part at frame 1; comparing two megabytes after each frame, they never part in three
hundred and twenty. The first instrument written for this was measuring itself.

**And there are two sites, not one.** Dispatching `decay` over every block instead of over the coarse
list — one line in `live::arranged`, put back afterwards — makes `AWAKE` identical across runs and
leaves `HOT` wandering, and the lists wander still. So the list-driven decay is one contributor and
something in the coarse flags is another.

## The Decision

`best` in `tests/coarse.rs` asserts that one arrangement, run twice, reaches the same place. It kept
the faster of two runs and threw the other count away, so a run that was not reproducible looked like
one that was — and the disagreement only surfaced later against the *other* arrangement, where it
reads as two programs differing rather than as one of them wandering.

`tests/hunt.rs` is kept as the instrument, not as a check: it prints what each grouping did and
asserts nothing, because what to assert is not yet known. `tests/live.rs` gains a run of the coarse
arrangement at 1024 blocks, which is coverage worth having and which does **not** catch this.

## The Rejected Route

The larger run added to `tests/live.rs` was written to catch this and does not. Six runs on the same
Radeon, at the same 1024 blocks and 400 frames, agree cell for cell every time — and the measurement
above says why: it compares the world, and the world is not what is wrong. It is kept for the coverage
it adds on its own terms, and its doc comment says so.

Blaming the stage grouping is rejected on the one-pass-a-stage run. Blaming a duplicate in the coarse
list is rejected too: the list is ascending and free of duplicates in every run, which
`tests/hunt.rs` checks — only its length moves.

## What Answered It

`decisions/DR-0018`. The cause is `decay`'s own countdown on the coarse flag — a read-modify-write on
one word by every lane of the workgroup — and everything in **The Limit** below stands as it was
written except the first line: the cause is now localised to those three lines, without the tool that
section says the next step wanted. `tests/wander.rs` is the instrument that did it, and
`tests/hunt.rs` now reads the same on every line.

## The Limit

**The cause is NOT LOCALISED to a line.** What is measured is the shape: two counter buffers, both
written by `live::wake` as a plain store over a three-by-three neighbourhood that crosses block
boundaries, and both read-modify-written by `decay`, which is the only read-modify-write on either.
That is where a race would live and it is not proof that one does.

The tool for the next step is Vulkan's synchronization validation layer, which is **not installed on
this machine** — no Vulkan SDK, no explicit layers registered — so it was not run. Nothing here should
be read as a finding that layer would have made.

Whether the every-block arrangement is the correct one is inferred from its being the one that does
not vary, not proven. Two devices and two widths; whether subgroup 4, 8 or 16 wander is unknown — CI
runs them on a software driver whose timing is nothing like either card's, and it has not shown this.
