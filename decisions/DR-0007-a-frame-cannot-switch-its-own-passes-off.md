---
id: DR-0007
title: A frame cannot switch its own passes off, because the barrier costs more than the dispatch
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, five runs of `tests/floor.rs` put a pass whose count the device set to
nought at 48–51 ns against 120–123 ns for one it left on, and a stage before a count read from a
buffer at 2408–2443 ns against 1085–1099 ns for one before a count the host wrote. Gating a pass
moves it from the second pair to the first, which is 69–75 ns saved against 1309–1358 ns spent.

## The Decision

`crate::live::wired` emits a fixed workgroup count for every pass whose count the device does not
already write, and a frame runs the same passes whether anything moved or not.

## The Rejected Route

A frame reading `layout::TALLY` and switching its own passes off for the rounds after the world came
to rest was rejected at 1309–1358 ns of barrier for 69–75 ns of dispatch a pass.

## The Limit

Whether the barrier has to widen in front of every gated pass was not measured: Vulkan makes a write
visible to every later indirect read, and `Resident` names `DRAW_INDIRECT` before each reader rather
than after each writer.
