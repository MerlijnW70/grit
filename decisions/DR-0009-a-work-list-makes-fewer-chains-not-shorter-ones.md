---
id: DR-0009
title: A work list makes fewer chains, not shorter ones
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, `src/bin/ticket.rs` ran one pass over the organisms whose masks
changed and then over all of them, and read 1254 against 1254 µs at 4096 masks of 513 connections,
5273 against 5819 at 65 536 of them, 22 365 against 22 770 at 16 384 masks of 1281 connections, and
245 895 against 261 225 at 65 536 of those — 56 to 69% of the work taken away for 0 to 9% of the
time. That pass is 96% of a generation, because with its workgroup count set to nought a generation
reads 884 µs against 22 365, and one of its invocations walks 256 inputs by 1281 connections in a
loop carrying a running total, which is a single chain of dependent steps.

## The Decision

`crate::compact` is reached for where a pass is many short independent steps, and a saving is
claimed for it only where the pass it shortens was filling the device.

## The Rejected Route

Compacting that evaluation was rejected at 0 to 9% of the time for 56 to 69% of the work, the card
holding fewer chains rather than running shorter ones.

## The Limit

The apparatus is `src/bin/ticket.rs`, a demonstration that is not part of the crate and is not kept,
and one card at one width is all that was measured.
