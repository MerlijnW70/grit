---
id: DR-0021
title: The floor is measured on a card something else is using
status: prose-only
---

## The Measurement

`tests/floor.rs` puts a submit and a fence at 41.1–43.9 µs and prices a dispatch and a barrier by
fitting a line through six submission sizes. Four of its assertions are on that fit. One of them
failed inside a full-suite run, and running it alone did not reproduce it — three times.

Twenty runs a card, back to back, nothing else asked of the machine by this session:

```text
  NVIDIA GeForce RTX 4080     9 of 20 runs failed, and 9 of 20 again
  AMD Radeon(TM) Graphics     0 of 20 runs failed, between those two
```

Two further sets of twenty on the 4080, taken to record the figures below, failed 5 and 4 — so the
rate moves between a fifth and a half over the afternoon and the Radeon stays at nothing.

It is not the fit and it is not the code. A failing run's own table says what it is:

```text
  frames     no passes     one stage  eight stages
       1       585.0 us       587.8 us       599.4 us
       2       586.6 us       590.0 us       597.9 us
       5       187.5 us       595.3 us       618.5 us
      10       581.5 us       600.2 us       677.6 us
      20       581.1 us       618.0 us       765.9 us
      40       579.7 us       253.4 us       934.4 us

  a submission costs 481.9 us before it holds anything
  a frame of nothing adds 2.69 us, of eight dispatches -8.01 us
```

A submission holding **nothing** costs 585 µs where a passing run of the same test on the same card
reads 45.6, and two cells are wild the other way. The line fitted through that says eight dispatches
cost −8.01 µs a frame, and the assertion that a dispatch is not free is the one that fires. The 4080
in this machine drives the display and is the card another session on it runs against; the Radeon,
measured in the same minutes, does not do this at all.

The one assertion in the file that did **not** fail is the one that is not a fit: the device's own
clock read 64 ns a frame for nothing and 9.286 µs for eight dispatches in eight stages, in the same
run whose host fit had gone to pieces.

### Two gates for it, both measured and both rejected

The file already prints how far its three intercepts disagree — "which is how much to trust the
fit", 5.2 µs in a good run and 163.0 in the one above. Using it as the gate looked obvious. Twenty
runs with the figure recorded beside the outcome:

```text
  failed at    41.5  106.9  108.2  138.3  174.3 us
  passed at     4.9    5.2    7.3    8.8    9.5   13.6   43.1   83.8   99.2  102.6
                111.5  116.5  118.5  147.2  168.7
```

**It does not separate them.** A run passed at 168.7 µs and one failed at 41.5, so the two
populations overlap over four fifths of the range.

Nor does the table's own shape. More frames in one submission cannot cost less, so a column that
steps down is a corrupted sample; counting those steps over another twenty runs, of which four
failed, gives failures at 4, 5, 5 and 7 dips against passes at 1, 2, 3, 4 and 6. Rejected on the same
grounds.

## The Decision

**Nothing is changed in `tests/floor.rs`.** Two gates were measured and neither works, and the third
thing available — widening the assertions until they stop firing — is not a gate, it is turning the
test off slowly.

What is recorded instead is where to run it: on this machine `tests/floor.rs` is a measurement of
the Radeon and, while the 4080 is shared, not one of the 4080. That is the same shape as
`decisions/DR-0019`, where a constant taken on one card did not describe the other, and the same
shape as `tests/coarse.rs`, which stopped asserting a timing under load for exactly this reason and
says so in its own doc.

## The Rejected Route

**Gating on the fit's own trust figure** is rejected on the twenty runs above: the two populations
overlap from 41.5 µs to 168.7.

**Gating on the table being non-decreasing** is rejected the same way: a run passed at six dips and
one failed at four.

**Asserting on the device clock instead of the host fit** is not rejected and is not done. It is the
one figure that held up in the failing run, and moving the four assertions onto it would be a real
change to what the file measures — a dispatch and a barrier priced by the device rather than by the
round trip, which is a different claim from the one `decisions/DR-0001` built the frame's price list
on. It is worth doing and it is worth measuring first, and neither happened here.

## The Limit

**Why the 4080 is busy is inferred and not measured.** It drives this machine's display and another
session works on it, and the Radeon does not do this — that is what the numbers say. Which process
holds the queue was not looked at, and nothing here reads the card's own utilisation.

**A hundred runs, one afternoon.** Five sets of twenty, and the rate moves between 4 and 9 of 20 on
the 4080 while the Radeon stays at 0 of 20. It is not a constant of anything. What travels is the
direction: the failure follows the card, not the test.

**Which of the four assertions fires was not tallied.** The one caught in full was "eight dispatches
cost −8009 ns and none cost 2690 ns, so a dispatch is free". Whether the other three ever fire is NOT
MEASURED.
