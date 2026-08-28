---
id: DR-0020
title: The last word two lanes could disagree about
status: prose-only
---

## The Measurement

`decisions/DR-0018` took a read-modify-write out of `live::decay` — one word, read and written back
by every lane of the workgroup, justified by the rule `Kernel::store_at` states about *identical*
writes. That rule covers a store of a constant. It does not cover a value that depends on when the
read happened, and measured, the flag took a second decrement in one block of every hundred thousand.

That is a shape, not one line, so the crate was swept for it: every `load_at` and `store_at` in
`src/live.rs`, `src/compact.rs`, `src/sweep.rs` and `src/sand.rs`. Every other store is either at a
per-lane address or a value that does not depend on a read of the same word — `compact::scan`'s
aggregate and `live::mark`'s count are reductions stored by every lane, which is the identical write
the rule really is about.

**One was left, and it is the frame counter.** `live::tick` loads `PARAMS[0]`, adds one and stores it
back, from every invocation of its single workgroup, and said so in its own doc with the same
argument the flag's did. What a wrong count would cost is not a wrong number on a screen: the **low
bit of that word is the Margolus partition's phase**, so a counter that jumped by two would leave the
origin where it was for two frames running and nothing would cross a block boundary in between.
`tests/live.rs` cannot see that — both arrangements read the same word and would agree about the same
wrong phase.

**It does not miscount.** `tests/counting.rs` dispatches the pass alone, a barrier behind each, and
compares the counter with the number of dispatches: fifty submissions of ten thousand, on each of two
cards, every one of them exact. And in the loop it actually runs in — where it shares a stage with
`decay`, `wipe` and `wipe_counts` and three other dispatches are in flight — four rounds of two
thousand frames under each arrangement on each card, every one exact.

## The Decision

**It is a workgroup of one invocation now.** `Shape::new` takes the workgroup size apart from the
subgroup width, so `Shape::new(subgroup, 1, BINDINGS)` is the same pass with nothing left to race —
the shape is gone by construction rather than by a measurement that would have to be repeated on the
next card.

**And it costs nothing that can be separated from the machine.** One tick and the barrier behind it,
`Resident::on_device`, five submissions a size, at 100 frames a submission:

```text
                          a workgroup's worth                       one invocation
  Radeon, five and        750-767  731-772  760-784                 730-786  760-772  734-752
  three runs              734-762  747-790                          ns
  4080, five and          1059-1065  1068-1074  1057-1064           1204-1213  1200-1212
  three runs              1062-1216  1206-1220                      1206-1217
```

Five runs of the old shape and three of the new, and the fourth and fifth runs of the old are why:
the first three read 1057-1074 on the 4080 and the next two read 1062-1216 and 1206-1220. **The
spread appears inside the old shape alone**, so the 140 ns that three-against-three seemed to show is
the card drifting and not the change — a conclusion two more repetitions overturned. On the Radeon
the two are indistinguishable in every run. This is not a claim that the new shape is free to a
nanosecond; it is a claim that nothing here can tell the two apart.

Most of either figure is the barrier rather than the pass. `tests/floor.rs`, three runs a card for
this record: on the 4080 a dispatch is about 111-215 ns and a barrier about 962-1009, which adds to
1073-1224 against the 1057-1220 above; on the Radeon 116-121 and 444-453, adding to 560-574 against
730-790. The same order, and not a decomposition this claims to have taken apart.

`tests/counting.rs` stays as the check: it is what would notice if the counter ever stopped counting
frames, and it costs two seconds.

## The Rejected Route

**Leaving it and correcting only the doc** was the other option, and the measurements above support
it — half a million dispatches a card with no miscount is not a weak result. It is rejected because
it would have to be re-measured on every part the crate is run on, where a workgroup of one is an
argument that travels. Nothing was traded for it.

**Making one lane do it under a select** does not work and is worth saying: a `select` chooses a
*value*, and the hazard here is the *load*. Every lane still reads.

## The Limit

**Two cards.** Both figures are the RTX 4080 at subgroup 32 and the Radeon at 64. Whether a
workgroup's worth of invocations would have miscounted on a third part is exactly the question this
change makes not worth asking, and it is not answered here.

**The barrier's price is a 4080 number too.** `src/resident.rs` prints 996–1009 ns for a barrier and
`src/live.rs` 981–1013 against 224–232 for a dispatch. Run for this record, `tests/floor.rs` says
about 962 and 260 on the 4080 — close — and about 467 and 108 on the Radeon, which is half. Nothing
here depends on which, and the stage grouping those figures argue for is not re-opened by it; it is
noted because `decisions/DR-0019` is about exactly this and the same caveat belongs on these.

**And a number this repository printed wrong.** While pricing the tick, `Resident::on_device` turned
out to be *already per frame* — `timed` divides the two stamps by how many frames the submission held
— and `tests/wander.rs` had been dividing again. DR-0018's cost table was a hundred times too small
and is corrected there. `tests/counting.rs::what_a_tick_costs` asks at 1, 10, 100 and 1000 frames a
submission, and what comes back settles it: on the 4080, 3808-4608 ns at one frame, 1436-1555 at ten,
1062-1220 at a hundred and 2339-2979 at a thousand. A total would have been ten times larger at each
step and it is not — it falls and then rises, which is a warm-up and a long submission on that card,
not a sum. `tests/coarse.rs` and
`tests/crossing.rs` read `on_device` straight and were never wrong.
