---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, `tests/floor.rs` fits a line through six submission sizes over eight
passes that write one word each: a submission is 35–42 µs, one more frame inside it 0 µs, a dispatch
210–245 ns, a barrier 996–1009 ns, one word read back 41–42 µs. In `src/bin/scale.rs`'s loop, five
passes that write one word cost +7.5 µs as five stages and +1.1 µs as one stage, which is 1.28 µs a
barrier and 0.22 µs a dispatch, against a frame that five runs of that file put at 27–29 µs at
262 144 cells and 29–32 µs at 16 777 216.

## The Decision

`tests/floor.rs` was added to separate a submission's cost from a frame's by varying how many frames
go into one submission, and cost differences are taken by adding passes to a loop rather than by
removing passes from it.

## The Rejected Route

Fusing `hot_scan_counts` and `hot_scan_totals` into one kernel with a workgroup barrier was rejected
at one barrier plus one dispatch, 1.2 µs of 27.

## The Limit

These figures do not measure device execution: the 11 µs that is left after subtracting ten barriers,
seventeen dispatches and a twentieth of a submission from 27 µs is a subtraction, and no timestamp
query was issued.
