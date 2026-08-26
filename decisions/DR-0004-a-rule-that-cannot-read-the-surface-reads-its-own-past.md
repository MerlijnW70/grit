---
id: DR-0004
title: A rule that cannot read the surface may read its own past
status: prose-only
---

**Measured on:** RTX 4080, subgroup 32.

## The problem, stated exactly

`sand::start` gives still water a direction when it is standing on water with air beside it. That
stands in for "there is somewhere lower to go", and almost everywhere it is right.

Where it fails is the remainder. A hundred and twenty-eight cells do not divide into sixty-two
columns, so four of them sit on a level surface with air beside them — which is **indistinguishable,
to a rule reading one 2×2 block, from water that ought to flow**. They took a direction, ran until
something stopped them, and were handed the same direction again next frame. For ever.

Whether there is somewhere lower is a property of the whole surface, and no block-local rule can see
it. That much was already written down. What was not is that it is not a cosmetic fault: the claim
this repository exists to make is that the work list reaches nought when nothing is happening, and a
few dozen quads kept awake by cells pacing about is that claim being false in the one case anybody
would check.

## The decision

**A rule that cannot read the surface can read its own history.** `sand::TIRED` is three bits in the
same word, above `HELD`: how many times this cell has been given a direction without ever
descending. At `sand::PATIENCE` `start` refuses it, the word stops changing, and the quad sleeps.

Nothing had to be added to clear it. `fall` already stripped a landing cell to bare material — *what
lands is bare* — and moving down is exactly the progress the counter measures the absence of. That
rule was written that way before there was anything here to strip.

## What it took

**`held` had to become what it claimed to be.** It was `word >> CARRIED_BITS` — the whole of the word
above the direction — which was the same thing right up until there was something above it. Left
alone, every cell that had ever tried to run would have read as standing on water.

**Three places rebuild a word from its low bits and had to carry the counter through:** `hold`,
`course`'s halt, and `start` itself. `hold` is the sharpest, because it is called every second frame:
a rebuild that dropped the counter would have made it count nothing at all.

**The counter saturates rather than wraps,** and does so by never being incremented past the limit —
`start` refuses a cell that has reached it, so the add can only ever see a value one short. A counter
that wrapped would hand the cell its patience back.

## The size was measured, not chosen

The first version of this record said two bits were "measured to be too few". They had not been
measured. On the basin in `tests/sand.rs` — a hundred and twenty-eight cells poured into sixty-two
columns:

```text
  bits  patience   water lies      still running
     1         1   1 to 7 deep                 0     not level, and the test says so
     2         3   2 to 3 deep                 0
     3         7   2 to 3 deep                 0
```

**Two is the measured minimum.** Three is what is there, for headroom that nothing measures: a cell
restarts when something stops it, and how often that happens over a basin sixty-two columns wide says
little about one four thousand wide. The word is then two bits of material, two of direction, one of
memory and three of this — **eight of its thirty-two**.

## What the tests say now

`tests/sand.rs` demanded `moving * 16 < poured` and demands nought. Then, because no direction is a
snapshot and a cell may be about to be given one, it steps the settled grid again on the host **in
both phases** and demands it come back unchanged. Proved by taking the gate out: four of a hundred
and twenty-eight cells run for ever and the test names them.

`tests/live.rs` pours water after the sand, which it never did. The rule now depends on a cell's
history, and history is the thing a listed run and a whole-grid run can disagree about. Six thousand
frames, cell for cell, ending at a list of nought — the list fell to nine at four thousand frames and
to nought between four and four and a half, so six thousand is that with margin.

## The cost, stated

**Fatigue clears on falling and on nothing else.** Pour new water into a basin that has come to rest
and the cells that actually descend reset, but a tired cell far from the disturbance stays tired. The
surface can therefore keep a step that an infinitely patient rule would have removed.

`tests/sand.rs` bounds it — `deepest - shallowest <= 1` — and in that scene it does not happen. It is
not proved not to happen in another.
