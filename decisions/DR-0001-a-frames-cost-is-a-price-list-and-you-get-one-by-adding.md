---
id: DR-0001
title: A frame's cost is a price list, and the way to get one is to add rather than remove
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, `tests/floor.rs` fits a line through six submission sizes over eight
passes that write one word each and puts a submission at 35–42 µs of host clock and 33 ns on the
device, one more frame inside it at 0, a dispatch at 210–245 ns, a barrier at 996–1009 ns, one word
read back at 41–42 µs, and those eight passes in eight stages at 8.84 µs on the device against
8 × 0.23 + 7 × 1.00 from the host. Three runs of `src/bin/scale.rs` put its sixteen passes in
eleven stages at 24–26 µs on the device and 27–31 µs of host clock at every size from 262 144 to
16 777 216 cells.

## The Decision

Cost differences are taken by adding passes to a loop and differencing, never by removing passes from
it, with `tests/floor.rs` and `Resident::on_device` as the apparatus.

## The Rejected Route

Fusing `hot_scan_counts` and `hot_scan_totals` into one kernel with a workgroup barrier was rejected
at one barrier plus one dispatch, 1.23 µs of 24–26.

## The Limit

These figures do not say what any one pass costs: the 11 µs left after subtracting ten barriers and
sixteen dispatches from 25 µs is a subtraction over sixteen passes, and no per-pass timestamp was
written.
