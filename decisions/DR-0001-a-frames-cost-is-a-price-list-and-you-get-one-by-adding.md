---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, `tests/floor.rs` fits a line through six submission sizes over eight
passes that write one word each: a submission is 35–42 µs, one more frame inside it 0 µs, a dispatch
210–245 ns, a barrier 996–1009 ns, one word read back 41–42 µs. The same file's device clock puts a
submission holding nothing at **33 ns** and those eight passes in eight stages at **8.84 µs**, which
is 8 × 0.23 + 7 × 1.00 to three digits — the host clock and the device clock, by different means,
agreeing on the same price list.

## The Decision

`tests/floor.rs` was added to separate a submission's cost from a frame's by varying how many frames
go into one submission, and cost differences are taken by adding passes to a loop rather than by
removing passes from it.

## The Rejected Route

Fusing `hot_scan_counts` and `hot_scan_totals` into one kernel with a workgroup barrier was rejected
at one barrier plus one dispatch, 1.2 µs of 27.

## The Limit

These figures do not say what any one pass costs: `Resident` writes two timestamps a submission and
puts the frame at 24–26 µs on the device against 27–31 µs of host clock, but the 11 µs left after
subtracting ten barriers and seventeen dispatches from that is still a subtraction over seventeen
passes, and no per-pass query was issued.
