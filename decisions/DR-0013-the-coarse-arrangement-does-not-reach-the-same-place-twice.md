---
id: DR-0013
title: The coarse arrangement does not reach the same place twice
status: prose-only
---

## The Measurement

The machine this is written on holds two devices — `["NVIDIA GeForce RTX 4080", "AMD Radeon(TM)
Graphics"]`, subgroup 32 and 64 — and `tests/dispatch.rs` said in prose that the whole suite passes
on both. Setting `GRIT_DEVICE=AMD` and running it is one line and had not been run.

On subgroup 64, at 1024 blocks, **the coarse arrangement run twice in one process does not reach the
same place**: 1128 quads and then 1130, 1130 and then 1128, 1125 and then 1130. The every-block
arrangement beside it answers 1130 every time. Across separate processes, 5 of 8 runs of
`tests/coarse.rs` ended with the two arrangements disagreeing; on subgroup 32 it is 0 of 8, and the
new equality above is quiet in 4 runs of 4 there.

It needs the settling, not the pour. The brush stops at frame 200, and over six runs at each length:

```text
  200 frames    0 of 6 disagreed
  250 frames    4 of 6
  300 frames    6 of 6
```

`tests/floor.rs` fails on the same device for a different reason: a pass the device switched off cost
115 ns against 99 for one it left on, which is the check that what the count buffer holds reaches the
dispatch.

## The Decision

`best` in `tests/coarse.rs` asserts that one arrangement, run twice, reaches the same place. It kept
the faster of two runs and threw the other count away, so a run that was not reproducible looked like
one that was — and the disagreement only surfaced later against the *other* arrangement, where it
reads as two programs differing rather than as one of them wandering.

`tests/live.rs` gains a run of the coarse arrangement at 1024 blocks. Before it, the strongest check
in this repository ran that hierarchy only at 128 blocks, a size at which it is not worth using, and
being slower was taken as the only reason not to grow it.

`tests/floor.rs` skips itself when `CI` is set: every check in it is a wall-clock comparison a few
hundred nanoseconds wide, and a shared runner is contended by somebody else's job.

And `.github/workflows/ci.yml` exists, which it did not. It runs the suite on Mesa's lavapipe at
subgroup 4, 8 and 16 — three widths every kernel here has an arm for and neither device in this
machine can report — and it checks out `simdr` beside this repository, because `Cargo.toml` names it
by path and a clone of this alone does not build.

## The Rejected Route

The larger run added to `tests/live.rs` was written to catch this and does not. Six runs on the same
Radeon, at the same 1024 blocks and 400 frames, agree cell for cell every time — so whatever wanders
is **not the state that check compares**. The coarse arrangement computes the same sand, frame for
frame, at the size and the length where its own quad count will not sit still. It is kept for the
coverage it adds on its own terms and not as a guard for this, and its doc comment says so.

## The Limit

**The cause is NOT LOCALISED.** What is measured is that the count wanders on one device and not on
the other, and that the state it wanders over does not. Whether that is a race in the compaction, a
count read before the pass that writes it has landed, or something the driver does with an indirect
dispatch is not known, and nothing here should be read as saying which. Which of the two arrangements
is right is inferred from every-block being the one that does not vary, not proven.

Two devices and two widths. Subgroup 4, 8 and 16 have kernel arms and no device here; the CI above is
the first thing that will run them, and it has never run — its first run is the measurement, not a
confirmation of one.

Why `tests/floor.rs` failed on subgroup 64 is settled and was a different thing: `decisions/DR-0014`.
