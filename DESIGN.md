# How grit is put together

The [README](README.md) is what it does and how to run it. This is why it is shaped this way.

Seventeen passes in **eleven stages**, one submission, six words from the host. A barrier sits
between stages and not inside one, and `tests/floor.rs` prices one at 996–1009 ns against 210–245 ns
for a dispatch — so which passes are ordered against which is worth stating, and `live::wired` is
where it is stated, with the argument for every seam beside it.

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

Four of the ten barriers are followed by a dispatch that reads its workgroup count out of a buffer,
and only those four name `DRAW_INDIRECT`; naming it in the other six cost a fifth of the frame.

`tests/floor.rs` prices what is underneath all of it, on an apparatus with no simulation in it —
kernels that write one word, and the only thing varied is how many frames go into one submission, so
that the intercept and the slope come apart:

```text
  a submission, holding nothing        35–42 µs
  one more frame in it, holding nothing     0
  a dispatch                          210–245 ns
  a barrier                           996–1009 ns
  asking the device for one word       41–42 µs
```

Over four runs, and the spreads are the point: the barrier is the same number every time and the
submission is not, which is what a queue and a fence look like next to a pipeline flush. **The last
line is why `prepare` exists** — asking the device for one number costs more than the whole frame
that produced it, so the count is written into a buffer and Vulkan reads it at dispatch time.

---

### The partition, and why there is no race to resolve

Two cells want to move into the same empty one. On a CPU the loop order decides; on a GPU nothing
does, and the usual answers — atomics, a priority pass, double buffering with a tie-break — all cost
more than the physics.

This has no race because **no invocation writes outside its own 2×2 block**. The grid is partitioned
into blocks, each is read and rewritten by one invocation, and the partition's origin alternates
between frames so material can cross a boundary on the next step. That is the Margolus
neighbourhood, and conservation falls out of the arrangement rather than being enforced on top of
it.

*The corollary bit me anyway.* Reading outside a block is not free either: that row belongs to
another invocation and is being rewritten at the same moment. `tests/live.rs` caught it exactly as a
race presents — frame nought, disagreement on an eight-wide part, agreement on a thirty-two-wide
one. See `sand::HELD` for the fix, which is one bit.

### The compaction, in three tiers

A quad's offset in the work list is the sum of three terms: how many awake quads sit before it in
its own block, how many its earlier blocks hold, and how many earlier *tiles* hold. The first is a
subgroup prefix sum and is free — it rides along with the pass that reads the flags. The second and
third are two more dispatches over a hundred and a thousandth of the data.

```
  sweep        blocks × per_block quads  →  counts[block]  and a bucket each
  scan_counts  tiles  × per_block counts →  bases[block]   and totals[tile]
  scan_totals  1      × per_block totals →  totals[tile] exclusive, and the grand total
  gather       blocks × per_block slots  →  dense[], the work list, contiguous
```

Two levels reach `per_block²` blocks — 16.7 million quads on a 32-lane part. Past that `Plan::over`
refuses **by name** rather than scanning the first tier and silently dropping the rest.

### Two levels of "is anything happening here"

The work list makes the settle proportional to what is moving. It does nothing for the passes that
*produce* it, and at sixteen million cells those were most of the frame. So the same trick one level
up: `layout::HOT` holds one word a block, written by exactly the code that writes the per-quad
flags, so a block reading nought provably has no awake quad in it.

**Skipping a block was not enough, because a skipped workgroup is still a launched one.** `decay`,
`mark` and `gather` each ran one per block — 3 × 16 384 of them at the largest size, doing a load and
a vote before they exited — and that was the whole of the frame's growth from four million cells to
sixteen. So the compaction is applied to itself: the same four passes over `HOT` that they run over
`AWAKE`, producing a list of the blocks that hold something, and the three of them are dispatched
over *that*.

```
  hot_mark     a tile × per_block blocks  →  hot_counts[tile]  and a bucket each
  hot_scan ×2                                the tiles against each other
  hot_gather   a tile × per_block slots   →  hot_dense[], the hot-block list
  hot_prepare  1                          →  how many workgroups the three run
```

This level needs no third tier and never will: `Plan::over` already refuses past `per_block²` blocks,
so there are at most `per_block` tiles and one workgroup scans them all. The whole hierarchy costs
about a hundred and thirty workgroups to remove forty-nine thousand.

The gate is still there in `gather`, where it is free — the count it votes on was loaded anyway — and
`crate::gate` still carries the argument for why a branch is allowed at all: the condition is one word
loaded at the *workgroup's* index, so every lane has the same answer by construction, and
`any_uniform` is how a program says so in a form the branch will accept.

Two things had to move with it. `mark` used to write `counts[block] = 0` before its gate so a skipped
block said nothing rather than last frame's something — a block that is never dispatched cannot write
that, so it is a pass of its own now, one workgroup a tile. And `HOT` is padded to whole tiles,
because the coarse sweep reads a tile at a time and a dump slot inside the padding would be a hot
block that no quad ever woke.

### The reduction

Everything above compacts: a prefix sum, to know *which* and in what order. The telemetry is the
other half of the lane model — a reduction, which does not care which, only how many, and hands one
answer back for the whole device. Six numbers a frame, summed inside the settle because the settle
already has the cells in registers. One `reduce_sum` and one atomic a workgroup.

Press **F** and the window steers itself to the centre of the motion. Nothing else in the loop knows
where anything is.

---

## Working against an engine with no subtraction

`simdr`'s lane API has no exclusive-or, no subtraction, no division, and no bitwise and on vectors.
That is not an omission — it is what makes every operation map to something a subgroup can do. It
means the code says what it means:

| wanted | written |
| --- | --- |
| `a − b` | `add(a, mul(b, −1))` |
| `v mod 2ᵏ` | `add(v, mul(shift_left(shift_right(v, k), k), −1))` |
| `v ÷ 2ᵏ` | `shift_right_logical(v, k)` |
| `p && q` | `mul(select(p,1,0), select(q,1,0))` — a `Predicate` is not a value |

The grid's sides are powers of two for exactly this reason, and a material's *number is its weight*
so that "heavier above lighter swaps" is one comparison rather than one per pair of materials. Sand
sinking through water falls out for free.

---
---

## Things that are true and unflattering

* **Water takes a long time to stop, and a cell has to be allowed to give up.** A pool levels to what
  fits in a whole layer, and the remainder — 128 cells do not divide by 62 columns — used to pace
  about looking for somewhere to be, for ever. No rule reading one 2×2 block can know where it
  belongs: whether there is somewhere lower is a property of the whole surface. What a rule *can*
  read is the cell's own past, and `sand::TIRED` is three bits of it — how many times this cell has
  taken a direction without ever descending. At `sand::PATIENCE` it stops asking, and `fall` clears
  the count because *what lands is bare*. `tests/sand.rs` now demands nought rather than bounding it,
  and demands a further step in both phases change nothing at all.
* **A frame's wall clock is a submit and a fence**, 37 µs on this machine
  whatever is inside it. The table above was measured with `Resident::frames`, which records twenty
  frames into one command buffer and divides. Measured the other way, the coarse level *appeared to
  make things slower* and nearly went in the bin on that evidence — and the hot-block list that came
  after it would have been invisible too, because what it saves at the largest size is 84 µs and a
  round trip is a third of that again. **The hundred microseconds this repository quoted for years
  was a round trip that moves data**, inherited from `simdr`'s `DR-0008` and used for a bare submit
  and fence; `tests/floor.rs` prices them apart. Nothing in the design changes for it — 41 µs to ask
  the device for one number is still more than the whole frame that produced it, which is why
  `prepare` exists.
* **What is left of the frame is mostly the barriers, and there are ten of them.** Priced by adding
  idle passes rather than removing real ones, so the simulation is unchanged: five of them in five
  stages costs 7.5 µs and five inside one stage costs 1.1 µs, which is **1.3 µs a barrier and 0.2 µs
  a dispatch**. The ten are a genuine dependency chain — the two hierarchies are serial by
  construction — so the obvious remaining merge, fusing the coarse `scan ×2` into one kernel, buys
  one barrier and one dispatch: 1.5 µs of 25, for a coarse-only kernel that breaks the symmetry
  where `compact::scan` serves both tiers. Priced and declined. A smaller world still does not get a
  smaller frame.
* **`paint` is the largest fixed dispatch left**, at one invocation a pixel — 16 384 workgroups for
  a 1024×512 window. It is priced by the window rather than by the world, so it does not break the
  claim above; it is simply what `pour` pays and `scale` does not measure.
* **A column of sand takes a long time to fall**, and that is the rule rather than a fault. A full
  2×2 block has nowhere for anything to move, so a wide column erodes only from its two edges, a
  cell a frame each side, and drains from the outside in.
