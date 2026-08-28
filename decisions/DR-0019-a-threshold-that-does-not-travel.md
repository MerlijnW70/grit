---
id: DR-0019
title: A threshold that does not travel
status: prose-only
---

## The Measurement

`live::COARSE_FROM` is 2048 blocks: below it `wired` dispatches `decay`, `mark` and `gather` over
every block, above it over the coarse list. `decisions/DR-0003` took that number on an RTX 4080 and
`tests/coarse.rs` has guarded it since by demanding the every-block arrangement be the cheaper one an
octave below and the dearer one an octave above.

It fails on the second card in this machine. Both arrangements, three hundred frames, the device's
own clock. The 256-to-4096 rows are three runs or more on each card and repeat to within 1.9 µs on
the Radeon and 0.2 on the 4080; the rows below that are four runs on the Radeon and one on the 4080.
The two tables agree with themselves and with each other about nothing:

```text
  NVIDIA GeForce RTX 4080, subgroup 32        AMD Radeon(TM) Graphics, subgroup 64
  blocks     cells    coarse     every        blocks     cells    coarse     every
      16     16384     21.4      13.0             16     32768     23.0      18.6
      32     32768     21.2      12.9             32     65536     27.7      27.2
      64     65536     21.5      13.4             64    131072     36.6      43.3
     128    131072     21.8      13.6            128    262144     46.1      58.0
     256    262144     22.0      14.3            256    524288     64.5      96.5
     512    524288     22.6      15.9            512   1048576     65.3     166.4
    1024   1048576     22.5      19.2           1024   2097152    102.7     306.4
    2048   2097152     22.7      25.3           2048   4194304    102.0     585.1
    4096   4194304     22.6      37.4           4096   8388608    141.2    1136
```

The 4080 crosses between 1024 and 2048, which is what 2048 says. The Radeon crosses between 32 and
64 — at 32 the two are within 3% over four runs, and from 64 up the list wins and keeps winning.

**And no rule reaches both.** At the same 131 072 cells the sixty-four-lane part prefers the list
(36.6 against 43.3 µs) and the thirty-two-lane part prefers every block (21.8 against 13.6). Same
cells, same block-to-quad ratio at the level that matters, opposite answers. The subgroup width
separates the two rows because it separates the two *cards*, not because it explains them. A
threshold keyed on it would be a guess wearing a measurement's clothes.

## The Decision

**The constant stays what it always was, and the measured choice is offered beside it.**
`live::both` builds both arrangements into one module list and hands back a stage list for each, the
one that is not running given counts of nothing — `Resident` does not record a pass told to run no
workgroups and drops a stage left empty, so the silent arrangement costs neither a dispatch nor a
barrier. One resident, one set of buffers; at 4096 blocks on a sixty-four-lane part those buffers are
the whole of what a frame owns, and two residents to answer one question is not a trade.
`live::cheaper` then pours a budget of frames each way and says which was cheaper.

`tests/crossing.rs::a_silenced_arrangement_computes_the_same_world` is what says the silencing is
sound: the same sixty frames poured four ways — each arrangement silenced beside the other, and each
alone in a resident of its own — reach one world and one work list, on both cards.

**Twenty frames is the budget, and it is measured.** `how_few_frames_name_the_winner` asks at 1, 5,
20, 60 and 300 frames, five rounds each, at seven sizes. One frame named the wrong arrangement at 64
and at 4096 blocks. Five agreed with three hundred at every size on the Radeon. Near the 4080's own
crossing at 2048 blocks, 5, 20 and 60 all split across their five rounds, which is what two
arrangements within a few percent of each other look like and is the same place `tests/coarse.rs` has
always refused to assert.

What it is worth, end to end, from nothing to 2000 frames poured, wall clock, three runs a size:

```text
  AMD Radeon(TM) Graphics       by constant           by asking the device
       64 blocks             110.1-111.0 ms every    115.0-117.0 ms list
      128                    149.2-153.2    every    153.0-164.7    list
      256                    236.0-245.3    every    229.6-232.9    list
      512                    410.2-427.4    every    350.7-361.5    list
     1024                    739.5-768.7    every    604.8-625.1    list
     2048                    848.9-880.3    list     864.2-896.0    list
     4096                      1.331-1.468 s list      1.360-1.495 s list
```

Taken run against run, it is worth 128.9–143.6 ms at 1024 blocks and 56.1–65.9 at 512, and it costs
3.7–11.5 ms at 64 and 128 blocks, where it picks the arrangement that is cheaper on the device and dearer on the
wall, and 14.0–15.7 at 2048. **Offered, not taken**: a caller that knows its device keeps `wired`.

`tests/coarse.rs` now reports two things and asserts neither. It names the sizes at which the device
running it wants the other arrangement by more than a tenth of the frame — on the Radeon that is 256,
512 and 1024; on the 4080 it is none — and it puts `live::cheaper`'s twenty frames beside its own
three hundred at every size where the three hundred are more than a quarter apart.

**Asserting the second one was tried and measured out.** Held to it at a gap of a tenth the file
passed alone and failed inside the whole suite; at a quarter, one full-suite run in three still
flipped a size. Twenty frames on a queue a dozen other tests are also using prices the machine, which
is the argument this file has made about timing assertions since it was written and which turns out
to apply to its own. What it still asserts is the canary it always had — that both arrangements
stepped the same number of quads — and the mechanism behind `cheaper` is gated by the world
comparison in `tests/crossing.rs`. With that settled the whole suite ran clean three times on each
card, 64 tests a run.

## The Rejected Route

**Keying the threshold on the subgroup width** is rejected on the 131 072-cell row above: the two
cards answer opposite questions from the same shape, so width is a name for the device and not a
cause.

**Lowering the constant for everyone** is rejected on the 4080's own table, where the list is 22.0
against 14.3 µs at 256 blocks and 22.5 against 19.2 at 1024 — a constant low enough for the Radeon
sends that card to the dearer arrangement over most of the range.

**A calibration that sets the world up three times**, so that its two timings start from the same
place, was built first and measured: at 2000 frames and 1024 blocks on the Radeon it came to
679.6–692.7 ms against 601.3–625.1 for the one that spends the first two submissions of the run
itself, and at 2048 blocks it was 1.009–1.060 s against 848.0–880.3 for the constant. Two extra seeds is most of what
it cost. The calibration in the tree pours its frames into the run.

**Picking the winner on the host's clock instead of the device's** was built and rejected. The
thought was that it would fix 64 and 128 blocks on the Radeon, where the list is cheaper on the card
and dearer over 2000 frames. It does not: over two runs it chose the list at both sizes exactly as
the device clock did. **Warming only the arrangement timed first** was found the same way — the
instrument was handing the second arrangement its own first-use inside the window being measured,
which was a real flaw and is fixed in `live::cheaper`, and fixing it did not change the choice
either.

## The Limit

**The calibration decides on a colder world than it runs in.** Twenty frames after the seed the work
list is far shorter than it is at frame three hundred — in one run of `tests/coarse.rs` on the Radeon
at 256 blocks `cheaper` read 25.5 µs against 88.4 where the same run's settled table read 64.8
against 95.2. On both cards, at every size
where the long measurement is not close, the two agree anyway; whether a workload that changes shape
later would be better served by asking again is not measured here.

**The 4080's wall clock did not settle.** Six runs of the end-to-end comparison put the same row
anywhere between 37 and 235 ms, both paths moving together, on the card that also drives this
machine's display. What is stable there is the choice: the calibration picked what the constant
picked in all forty-two rows. What asking costs on that card in wall clock is not a number this
session can give.

**Two cards, one shape of workload.** Every figure here is one pour into one room, at the sizes
listed, on the two devices this machine holds. The software driver CI runs is a processor and prices
these arrangements differently again — `tests/coarse.rs` has said so since it was written, and still
declines to check the threshold when `CI` is set.
