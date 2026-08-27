---
id: DR-0015
title: A threshold above what a narrow device can plan
status: prose-only
---

## The Measurement

The first workflow run that got as far as a device ran the suite on Mesa's `llvmpipe` at subgroup 4,
8 and 16 — three widths every kernel here has an arm for and neither card in the author's machine can
report. Everything passed at all three except `tests/coarse.rs`, and it failed differently at the
narrow end than at the other two.

At subgroup 4, `Plan::over` refused before anything ran: `Blocks { blocks: 2048, limit: 1024 }`. The
limit is `sweep::cells(subgroup)` squared, which is `(subgroup × 8)²`:

```text
  subgroup     4        8        16        32        64
  blocks    1024     4096     16384     65536    262144
```

`live::COARSE_FROM` is 2048. **So at subgroup 4 every plan that can be built is below the size at
which the coarse hierarchy would be chosen**, and that hierarchy is unreachable there — not
misbehaving, unreachable. Nothing said so; the constant's own documentation is about where it stops
paying for itself, which is a different question from where it can exist.

At subgroup 8 and 16 the plan built and the equality checks passed. What failed was the threshold
itself: at 1024 blocks every-block took 551.74 µs and the list 473.375 µs, so the list was the
*cheaper* of the two below the size at which it is supposed to become worth using. On a device that
runs on the processor, which is what `llvmpipe` is.

Separately, the job before it: `minifb`'s build script panics on anything that is not Windows —
*"At least one of the x11 or wayland features must be enabled"* — because `Cargo.toml` turns its
default features off and those go with them. `pour` and `spike` did not build on Linux at all, and
nothing said so until a Linux runner tried.

## The Decision

`tests/coarse.rs` skips a size its width cannot plan, prints which, and carries on with the rest,
rather than expecting a plan it has no reason to get. Its threshold assertions do not run when `CI`
is set, for the reason `tests/floor.rs` skips itself entirely: which of two arrangements is faster is
a property of the device, and a shared runner running a software driver is not the device the number
is about. **The equality checks above them do run there** — those are the half of the file that found
something.

`Cargo.toml` names `minifb`'s `x11` feature for `cfg(unix)`. One back end is what a window needs, and
`dialogs` is the feature the default-off was for in the first place.

## The Rejected Route

Lowering `COARSE_FROM` until a four-wide device could reach it is rejected. It is 2048 because that
is where the hierarchy started paying for itself on this machine's hardware — `decisions/DR-0009` and
the sweep in `tests/coarse.rs` — and moving a measured threshold to accommodate a software driver
would make it a number about nothing. What is wrong is not the constant but that its unreachability
at the narrow end was undocumented.

Asserting the threshold on the runner anyway, with a wider margin, is rejected for the same reason
`tests/floor.rs` stopped asserting its own comparison: the two devices are on opposite sides of it,
so any margin that admits one admits the failure the assertion exists to catch.

## The Limit

No four-wide or eight-wide hardware was involved: `llvmpipe` is a Vulkan implementation running on the
processor, and what it says about *timing* is worth nothing here — which is why none of it is asserted.
What it is good for is that the kernels ran and agreed at three widths that had never been executed at
all, and that is what these figures are.

Whether the coarse hierarchy being unreachable at subgroup 4 matters to anybody is NOT MEASURED,
because no such device has been in front of this. Whether `COARSE_FROM` is anywhere near right at
subgroup 8 or 16 on real hardware is unknown for the same reason.

`tests/coarse.rs` still fails on the integrated Radeon at subgroup 64, on the reproducibility check
rather than on any of this — `decisions/DR-0013`, and that cause is still not localised.
