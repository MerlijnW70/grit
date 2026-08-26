---
id: DR-0008
title: A barrier belongs behind the pass that wrote the count, not in front of the one that reads it
status: prose-only
---

## The Measurement

Eighteen runs of `src/bin/scale.rs` put a frame of sixteen passes in eleven stages at 22.28–22.42 µs
on the device at 4 194 304 cells and 22.38–22.59 at 16 777 216, against 24.82–26.24 and 24.75–25.20
for the same frame with `DRAW_INDIRECT` named in front of every stage holding a `Groups::From`. Five
runs of `tests/floor.rs` put a stage that wrote a count at 2373–2401 ns against 1097–1098 for one
that did not, and the loop holds two of the first where it held four.

## The Decision

`Groups::Counting` marks a pass that writes a workgroup count, and `Resident` names `DRAW_INDIRECT`
in the barrier behind each stage that holds one.

## The Rejected Route

Leaving `Resident` to work it out was rejected because a `Groups::From` names the binding it reads
and nothing in a recording says which binding a kernel writes.

## The Limit

`Resident` does not check the claim: a caller that marks no pass gets a dispatch reading the seeded
count rather than the written one, which `tests/dispatch.rs` shows as 32 invocations against 1184,
three runs out of three.
