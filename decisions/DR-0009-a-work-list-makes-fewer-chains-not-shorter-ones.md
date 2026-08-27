---
id: DR-0009
title: A work list makes fewer chains, not shorter ones
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, a pass dispatched over what changed rather than over everything took
73.6 µs against 155.6 and 185.6 against 381.5 in `src/bin/chip.rs`, where an invocation is a few
loads, four gates and a store — 44% of the work for 47 and 49% of the time — and 22 365 against
22 770 and 245 895 against 261 225 in `src/bin/ticket.rs`, where an invocation walks 256 inputs by
1281 connections in a loop carrying a running total: 31 and 43% of the work for 98 and 100% of the
time. That second pass is 96% of its generation, because with its workgroup count set to nought a
generation reads 884 µs against 22 365, so what separates the two is not which pass was shortened.

## The Decision

`crate::compact` is reached for where a pass is many short independent steps, and a saving is
claimed for it only where the pass it shortens was filling the device with them.

## The Rejected Route

Compacting the evaluation in `src/bin/ticket.rs` was rejected at 0 to 9% of the time for 56 to 69%
of the work, the card holding fewer chains rather than running shorter ones.

## The Limit

Both apparatus were demonstrations that were never part of the crate, and both have since been
removed: `src/bin/chip.rs` and `src/bin/ticket.rs` are gone, so the figures above cannot be re-run
as they stand. What is here is the record of what they measured while they existed. One card at one
width is all that was measured.
