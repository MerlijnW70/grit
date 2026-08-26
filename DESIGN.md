# How grit is put together

The [README](README.md) is what it does and how to run it. This is why it is shaped this way.

Each answer here is short on purpose: the full argument lives beside the code that makes it, and the
last column says where.

---

## The frame

Seventeen passes, **eleven stages**, one submission, six words from the host.

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

---

## What Vulkan charges

`tests/floor.rs` prices it on an apparatus with no simulation in it — kernels that write one word,
varying only how many frames go into one submission, so the intercept and the slope come apart.

```text
  a submission, holding nothing        35–42 µs
  one more frame in it, holding nothing     0
  a dispatch                          210–245 ns
  a barrier                           996–1009 ns
  asking the device for one word       41–42 µs
```

Four runs. The barrier returns the same number every time and the submission does not — a pipeline
flush next to a queue and a fence.

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

About 130 workgroups to remove 49 152, and 144 µs to 60. It needs no third tier and never will:
`Plan::over` already bounds the tiles at `per_block`, so one workgroup scans them all.

**Read:** `layout::HOT_BUCKETS`, `decisions/DR-0003`.

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
| **A smaller world does not get a smaller frame.** Ten barriers and seventeen dispatches are paid whatever the grid is. Fusing the coarse `scan ×2` would buy one of each — 1.2 µs of 27 — for a kernel that breaks the symmetry where `compact::scan` serves both tiers. Priced and declined. | `DR-0001` |
| **`paint` is the largest fixed dispatch left**, at one invocation a pixel — 16 384 workgroups for a 1024×512 window. Priced by the window rather than by the world, so it does not break the claim above; `scale` does not measure it. | — |
| **A column of sand takes a long time to fall.** A full 2×2 block has nowhere for anything to move, so a wide column erodes from its two edges, a cell a frame each side, and drains from the outside in. That is the rule rather than a fault. | — |
| **The frame was measured one way and could have been abandoned on it.** `Resident::frame` is a submit and a fence whatever is inside it, and measured that way the coarse level *appeared to make things slower*. `Resident::frames` records twenty frames into one submission and divides. | `DR-0001` |

---

## Where the arguments live

| | |
| --- | --- |
| `decisions/` | six records: measurement, decision, rejected route, limit — four headings, no prose |
| `src/live.rs` | `wired`, the one place the loop's order and grouping are decided |
| `src/resident.rs` | stages, the barrier mask, the kept recording |
| `src/sand.rs` | the Margolus rule and what a cell word holds |
| `tests/live.rs` | the same pour through the list and over the whole grid, six thousand frames |
