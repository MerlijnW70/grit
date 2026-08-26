---
id: DR-0007
title: A frame cannot switch its own passes off, because the barrier costs more than the dispatch
status: prose-only
---

## The Measurement

On an RTX 4080 at subgroup 32, five runs of `tests/floor.rs` put a pass whose count the device set to
nought at 46–47 ns against 117–118 ns for one it left on, and a stage that wrote a workgroup count at
2373–2401 ns against 1097–1098 ns for one that did not. Gating the seven passes of
`crate::live::wired` whose counts the host now fixes saves 490–504 ns, and the one barrier behind the
kernel that would write those counts costs 1275–1304.

## The Decision

`crate::live::wired` emits a fixed workgroup count for every pass whose count the device does not
already write, and a frame runs the same passes whether anything moved or not.

## The Rejected Route

A frame reading `layout::TALLY` and switching its own passes off for the rounds after the world came
to rest was rejected at 1275–1304 ns of barrier against 490–504 ns of dispatch.

## The Limit

The kernel that writes the gate is a further pass, and a further stage wherever nothing else can
share one with it, and neither of those was measured.
