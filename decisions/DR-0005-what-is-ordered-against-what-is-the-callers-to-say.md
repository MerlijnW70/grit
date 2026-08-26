---
id: DR-0005
title: What is ordered against what is the caller's to say, and the recording is not
status: prose-only
---

## The Measurement

`Resident` put a barrier after every pass, named `DRAW_INDIRECT` in all of them, and reset and
re-recorded the command buffer on every call: 60 µs a frame at 262 144 cells, of which
`Resident::recording` reported 7 µs. `Resident::submit` had taken stages since `1c38b88` and
`tests/dispatch.rs` had covered their semantics since, and `crate::live::wired` passed one pass a
stage.

## The Decision

`crate::live::wired` returns sixteen passes in eleven stages, `Resident` names `DRAW_INDIRECT` in
the four barriers whose next stage holds a `Groups::From`, and the command buffer is kept when the
stages, the copy-out list and the frame count match the last call — taking 58–62 µs to 43–44,
to 34, to 27–28 at 262 144 cells.

## The Rejected Route

Recording transfers into the frame's command buffer was rejected at `recorded 16 times, reused 0` in
`src/bin/scale.rs`, which writes four brush words between batches; the pool allocates two buffers and
reports `recorded 2 times, reused 14`.

## The Limit

The grouping is a claim the caller makes and `Resident` does not check: these figures do not prove
any grouping correct, and what stands in for that is `tests/live.rs`, which is a second algorithm
rather than a second opinion.
