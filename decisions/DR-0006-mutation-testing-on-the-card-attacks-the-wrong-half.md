---
id: DR-0006
title: Mutation testing on the card attacks the wrong half, and the numbers to say so were already written down
status: prose-only
---

**Measured on:** the figures below are `hyperia`'s, from its own `DR-0132` and `DR-0136`, on
`990-pro-nvme`. Nothing here was re-measured for this record.

## The proposal

This crate's shape — a work list of what is live, compacted every frame, dispatched over rather than
swept — was put to a different problem: **mutation testing.** Mutants as active cells. Kill a mutant
and it leaves the list; the run gets cheaper as it proceeds. Thirty-two or sixty-four mutants share
a subgroup, executing one instruction stream until the mutated line pulls them apart. State as
deltas rather than a copy of memory each.

It is a good analogy and the architecture is sound in the abstract. Mutants are independent in
space, which is the shape a card wants.

## Why it does not pay, on the codebase it was proposed for

`hyperia`'s gate had already taken a mutant apart, twice. Post-`DR-0136`:

```text
  the build a mutant pays              7.88 s
  one focused test, matrix knows       0.72 s
  the whole suite, matrix does not    14.75 s
  the matrix answers for about 3 in 4
```

A mean mutant is `0.75 × 8.60 + 0.25 × 22.63` = **12.11 s**, of which the **build is 65%**. The check
that this is the right arithmetic: 400 mutants a file × 12.11 s is 81 minutes, and `DR-0136` reports
an hour and a half for `src/card.rs`.

So the premise — *you run the program a thousand times on the processor* — is not where the time is.
The time is a thousand compiles and links.

```text
  if test execution went to nothing    12.11 s -> 7.88 s    1.54x
  if the build went to nothing         12.11 s -> 4.23 s    2.86x
```

An infinitely fast test runner takes an hour and a half to fifty minutes.

## Two harder obstacles, on that codebase

**The engine does syscalls.** `src/lib.rs`, `src/source/mod.rs` and `src/sys.rs` carry
`extern "system"` — file mappings, virtual reservations, unbuffered reads. That does not run on a
card. The pure arithmetic `hyperia/DR-0010` deliberately pushed down so the gate could reach it is
`spread.rs` plus `pattern.rs`: **274 lines of 78 files in the gate's surface**.

**And the 0.72 s is not computation.** That crate's library harnesses report in hundredths of a
second — the arithmetic is integers over small arrays. What the 0.72 s is, is cargo deciding what to
run and Windows starting a process. Neither parallelises across lanes.

## Where the proposal's own model is wrong

Two of its three mechanisms hold. One does not, and it is worth writing down because the mistake is
natural from this repository's own success:

**A killed mutant cannot leave its wavefront.** `crate::compact` wins because a *dispatch* is
replanned between frames — the list is rebuilt and the next dispatch is smaller. Within a subgroup
there is no such thing: a lane that has finished holds its slot until every lane retires. So "the run
gets cheaper as it proceeds" is true between dispatches and false inside one, and a batch of
thirty-two costs the longest mutant of the thirty-two rather than the mean.

The other two hold with qualifications. Lanes holding different *values* is free; lanes taking
different *branches* is not, and an interpreter is the worst possible case because its opcode switch
diverges on every instruction. Per-mutant state wants `state[address][lane]` and not
`state[lane][address]`, or every access is uncoalesced — which is a transformation of the whole
program under test, not a wrapper around it.

## What was declined, and what was not

**Declined:** running mutants of `hyperia` on the card.

**Not declined, and not this repository's to do:** the technique that removes the 65% needs no card
at all — compile the mutation set into one binary where the mutant is a runtime selector rather than
a source edit. That is where the work-list-of-survivors idea genuinely belongs, and it belongs on the
CPU.

**Left open, honestly:** `simdr` emits SPIR-V *and* decodes it back — `decode.rs` is how its tests
inspect what was emitted. A SPIR-V mutator is therefore reachable in this ecosystem in a way it is
not in general: mutate the word stream, build sixty-four variants, run them in one submission through
`Resident`, compact the dead away. No compiler in the loop.

Whether that is worth building depends on how much SPIR-V there is worth mutating, and **that has not
been measured.** It is not estimated here, because guessing at it is the exact habit `DR-0002` is
about.

## The lever that does exist

`hyperia/DR-0132`, in its own *what is not claimed*: the way past the remaining build is *"overlapping
a mutant's build with the previous mutant's kill"*. That is a pipelining change in the prober, it
attacks the 65%, and that record already names it.
