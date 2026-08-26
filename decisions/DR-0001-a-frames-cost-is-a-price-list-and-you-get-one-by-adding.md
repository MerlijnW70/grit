---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, five runs of `tests/floor.rs` put a submission at 41.1–43.9 µs of
host clock and 32–40 ns on the device, one more frame inside it at 0.05 µs, a dispatch at
224–232 ns, a barrier at 981–1013 ns, one word read back at 39.7–41.7 µs, and eight passes in eight
stages at 8.32–8.36 µs on the device against 8.7–9.0 µs at those prices from the host. Eighteen runs
of `src/bin/scale.rs` put its sixteen passes in eleven stages at 22.3–22.6 µs on the device at
4 194 304 and 16 777 216 cells, and the eleven passes in seven stages it runs below
`live::COARSE_FROM` at 14.3–19.4 at 262 144 and 1 048 576.

## The Decision

Cost differences are taken by adding passes to a loop and differencing, never by removing passes from
it, with `tests/floor.rs` and `Resident::on_device` as the apparatus.

## The Rejected Route

Fusing `hot_scan_counts` and `hot_scan_totals` into one kernel with a workgroup barrier was rejected
at one barrier plus one dispatch, 1.22 µs of 22.3–22.6.

## The Limit

These figures do not say what any one pass costs: the 4 µs left after subtracting two wide barriers,
nine narrow ones and sixteen dispatches from 22.4 µs is a subtraction over sixteen passes, and no
per-pass timestamp was written.
