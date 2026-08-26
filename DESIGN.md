# How grit is put together

The [README](README.md) is what it does and how to run it. This is why it is shaped this way.

Each answer here is short on purpose: the full argument lives beside the code that makes it, and the
last column says where.

---

## The frame

Seventeen passes, **eleven stages**, one submission, six words from the host — or twelve in eight,
below the size where five of them stop paying for themselves.

```text
  tick     1              the frame number, and the partition's phase
  decay    the hot list   every wake counter, one closer to nothing        indirect
  wipe     1              both tile totals and the tally, back to nothing
  wipe     a tile         every block's count, back to nothing
  brush    a box          what the pointer holds, and the quads it touched
  settle   the work list  one Margolus step over what the last frame woke  indirect
  hot ×5   a tile         which blocks hold anything, compacted into a list
  mark     the hot list   the counters, scanned into buckets and counts    indirect
  scan ×2                 the blocks against each other
  gather   the hot list   the buckets into one contiguous list             indirect
  prepare  1              how many workgroups the *next* settle runs
  paint    the window     the visible cells, as colours
```

A barrier sits between stages and not inside one. Which passes may share a stage is a question about
the frame, not about `Resident`, so `live::wired` answers it and names every seam.

**Five of those passes are conditional.** `hot ×5` builds the list that `decay`, `mark` and `gather`
are then dispatched over. Below `live::COARSE_FROM` blocks those three run over every block instead
and the five are not emitted, which leaves twelve passes in eight stages. `tests/coarse.rs` measures
both sides; `decisions/DR-0003` carries the figures.

---

## What Vulkan charges

`tests/floor.rs` prices it on an apparatus with no simulation in it — kernels that write one word,
varying only how many frames go into one submission, so the intercept and the slope come apart.

```text
                                             host clock    device clock
  a submission, holding nothing            41.1–43.9 µs       32–40 ns
  one more frame in it, holding nothing         0.05 µs
  a dispatch                                 224–232 ns
  a barrier                                  981–1013 ns
  eight passes in eight stages                              8.32–8.36 µs
  a pass the device switched off               46–47 ns
  a pass it left on                          117–118 ns
  a stage that wrote a workgroup count      2373–2401 ns
  a stage that did not                      1097–1098 ns
  asking the device for one word           39.7–41.7 µs
```

**The two clocks do not quite agree, and this page used to say they did.** Eight dispatches and
seven barriers is 8.7–9.0 µs at the host's own prices against 8.32–8.36 on the card: no overlap,
about 5% apart. What both ends do agree on is the round trip — a submission holding nothing is
32–40 ns on the card against 41–44 µs of host clock, measured from both sides at once.

Five runs. Nothing in the table returns the same number twice; the barrier moves by 3% between runs
and the submission by 2%.

**Two more lines say what this cannot become.** A pass the device switched off costs 46–47 ns, so a
frame could record every path it might take. But a stage that wrote a workgroup count costs about
1300 ns more than one that did not, and a gate is a kernel writing counts — 490–504 ns saved across
seven gated passes for 1275–1304 spent on the one barrier behind it. `decisions/DR-0007`.

**Two lines of that table are the whole design.** Asking the device for one number costs more than
the frame that produced it, so `prepare` writes the count into a buffer and Vulkan reads it at
dispatch time. And ten barriers at ~1 µs are most of what is left, which is why the stages exist.

---

## Why there is no race

| | |
| --- | --- |
| **The rule** | No invocation writes outside its own 2×2 block. The partition's origin alternates between frames, so material crosses a boundary on the next step. |
| **What it buys** | Conservation falls out of the arrangement instead of being enforced on top of it — no atomics, no priority pass, no double buffering. |
| **The corollary** | *Reading* outside a block is a race too. `tests/live.rs` caught it as one presents: frame nought, disagreement at subgroup 8, agreement at 32. |
| **The fix** | One bit. `sand::HELD` — the cell writes down what it is standing on, refreshed every second frame by the invocation entitled to write it. |
| **Read** | `src/sand.rs` module doc |

---

## How the work list is built

A quad's offset is a sum over three tiers, and each is a different thing in `simdr`.

```text
  serial   earlier cells in this lane      a Rust loop over registers      free
  before   earlier lanes of this subgroup  prefix_sum_exclusive            one instruction
  base     earlier blocks and tiles        two later dispatches            one load
```

The fusion is not in merging kernels. It is that the step and the running count **share a register**,
so the first tier costs no memory traffic — the state is read once, stepped, written once, and the
number deciding where it goes was added on the way.

The third tier cannot exist until every block has run, so it is a second dispatch — and because there
are more blocks than one block can scan, a third:

```text
  sweep        blocks × per_block cells   →  counts[block]  and a bucket each
  scan_counts  tiles  × per_block counts  →  bases[block]   and totals[tile]
  scan_totals  1      × per_block totals  →  totals[tile] exclusive, and the grand total
  gather       blocks × per_block slots   →  dense[], the work list, contiguous
```

Two levels reach `per_block²` blocks. Past that `Plan::over` refuses **by name** rather than scanning
the first tier and dropping the rest.

**Read:** `src/sweep.rs`, `src/compact.rs`.

---

## Why there is a second hierarchy

The work list makes the settle proportional to what moves. It did nothing for the three passes that
*produce* it — `decay`, `mark` and `gather` ran one workgroup a block, gated on `layout::HOT`.

**A gate makes a workgroup cheap. It does not stop it being launched.** At 16 777 216 cells that was
3 × 16 384 workgroups doing one load and a vote, and it was the whole of the frame's growth.

So the compaction is applied to itself — the same four passes over `HOT` that they run over `AWAKE`:

```text
  hot_mark     a tile × per_block blocks  →  hot_counts[tile]  and a bucket each
  hot_scan ×2                                the tiles against each other
  hot_gather   a tile × per_block slots   →  hot_dense[], the hot-block list
  hot_prepare  1                          →  how many workgroups the three run
```

About 130 workgroups to remove 49 152 — 6.2–6.7 µs on the device, the same at every size. That buys
37 µs down to 22 at 4096 blocks and costs 7.5 µs of 22 at 256, so it is not run at every size: the
two cross between 1024 and 2048 blocks, and `live::COARSE_FROM` is 2048. It needs no third tier and never will: `Plan::over` already bounds the tiles at `per_block`,
so one workgroup scans them all.

**Read:** `layout::HOT_BUCKETS`, `live::COARSE_FROM`, `tests/coarse.rs`, `decisions/DR-0003`.

---

## The one reduction

Everything else here compacts — a prefix sum, to know *which* and in what order. The telemetry wants
the opposite: not which, but how many, and one answer for the whole device.

Six numbers a frame, folded into the settle because it already has the cells in registers. One
`reduce_sum` and one atomic a workgroup. **The telemetry is not a pass.**

Press **F** in `pour` and the window steers to the centre of the motion — the same reduction,
weighted. Nothing else in the loop knows where anything is.

**Read:** `layout::TALLY`.

---

## Writing for an engine with no subtraction

`simdr`'s lane API has no exclusive-or, no subtraction, no division and no bitwise and on vectors.
That is not an omission — it is what makes every operation map to something a subgroup can do.

| wanted | written |
| --- | --- |
| `a − b` | `add(a, mul(b, −1))` |
| `v mod 2ᵏ` | `add(v, mul(shift_left(shift_right(v, k), k), −1))` |
| `v ÷ 2ᵏ` | `shift_right_logical(v, k)` |
| `p && q` | `mul(select(p,1,0), select(q,1,0))` — a `Predicate` is not a value |

The grid's sides are powers of two for exactly this reason, and a material's **number is its weight**
so that "heavier above lighter swaps" is one comparison rather than one per pair of materials. Sand
sinking through water falls out of that.

---

## What is still true and unflattering

| | |
| --- | --- |
| **Water takes thousands of frames to stop.** A runner moves one cell a frame, so levelling a pool 500 columns wide is thousands of them. It does reach nought: cells with nowhere to be give up after `sand::PATIENCE` tries. | `DR-0004` |
| **Fatigue clears on falling and on nothing else.** Pour into a basin that has come to rest and a tired cell far from the disturbance stays tired, so the surface can keep a step. Bounded at one cell in the scene `tests/sand.rs` runs; not proved in another. | `DR-0004` |
| **A frame has a floor the world's size does not reach.** At 262 144 cells `scale` runs eleven passes in seven stages for 14.3–14.4 µs on the device, of which one wide barrier, six narrow ones and eleven dispatches are 11.5 at `tests/floor.rs`'s prices. Fusing the coarse `scan ×2` would buy one barrier and one dispatch — 1.2 µs of 22 — for a kernel that breaks the symmetry where `compact::scan` serves both tiers. Priced and declined. | `DR-0001` |
| **`paint` is the largest fixed dispatch left**, at one invocation a pixel — 16 384 workgroups for a 1024×512 window. Priced by the window rather than by the world, so it does not break the claim above; `scale` does not measure it. | — |
| **A column of sand takes a long time to fall.** A full 2×2 block has nowhere for anything to move, so a wide column erodes from its two edges, a cell a frame each side, and drains from the outside in. That is the rule rather than a fault. | — |
| **The frame was measured one way and could have been abandoned on it.** `Resident::frame` is a submit and a fence whatever is inside it, and measured that way the coarse level *appeared to make things slower*. `Resident::frames` records twenty frames into one submission and divides. | `DR-0001` |

---

## Where the arguments live

| | |
| --- | --- |
| `decisions/` | eight records: measurement, decision, rejected route, limit — four headings, no prose |
| `src/live.rs` | `wired`, the one place the loop's order and grouping are decided |
| `src/resident.rs` | stages, the barrier mask, the kept recording |
| `src/sand.rs` | the Margolus rule and what a cell word holds |
| `tests/live.rs` | the same pour through the list and over the whole grid, six thousand frames |
