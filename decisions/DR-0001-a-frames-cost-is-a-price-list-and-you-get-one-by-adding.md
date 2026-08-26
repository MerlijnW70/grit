---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, five runs of `tests/floor.rs` put a submission at 43.2–44.3 µs of
host clock and 32 ns on the device, one more frame inside it at 0.05 µs, a dispatch at 221–244 ns,
a barrier at 980–1014 ns, one word read back at 40.1–42.4 µs, and eight passes in eight stages at
8.32–8.34 µs on the device against 8.6–9.1 µs at those prices from the host. Nineteen runs of
`src/bin/scale.rs` put its sixteen passes in eleven stages at 24.8–26.2 µs on the device at
4 194 304 and 16 777 216 cells, and the eleven passes in seven stages it runs below
`live::COARSE_FROM` at 14.3–19.3 at 262 144 and 1 048 576.

## The Decision

Cost differences are taken by adding passes to a loop and differencing, never by removing passes from
it, with `tests/floor.rs` and `Resident::on_device` as the apparatus.

## The Rejected Route

Fusing `hot_scan_counts` and `hot_scan_totals` into one kernel with a workgroup barrier was rejected
at one barrier plus one dispatch, 1.23 µs of 24.8–26.2.

## The Limit

These figures do not say what any one pass costs: the 11 µs left after subtracting ten barriers and
sixteen dispatches from 25 µs is a subtraction over sixteen passes, and no per-pass timestamp was
written.
