# grit

Sixteen million cells of falling sand and water, simulated in **27 µs a frame**, on a work list
that never learns how long it is.

Built on [`simdr`](../simdr) — a SPIR-V emitter written in stable Rust with an empty dependency
table. `grit` adds two of its own: `ash`, for the Vulkan a simulation needs and a measurement
harness does not, and `minifb`, for the one thing that has to leave the device.

```
cargo run --release --bin pour       # the demo: a 4096×4096 world, seen through a window
cargo run --release --bin scale      # what a frame costs at four sizes
cargo run --release --bin loop_cost  # what looking costs, which is why there is a window
cargo run --release --bin spike      # the first thing that worked, kept
cargo test  --release                # every claim below, checked
```

Left button pours sand, right pours water, **WASD** flies, **F** follows the action, Escape leaves.

---

## What is actually being demonstrated

Not "sand on a GPU". The demo is a vehicle for one number: **how much of the world a frame has to
touch.** A 4096×4096 grid holds 4 194 304 quads. A frame steps about **seven hundred** of them.

```
       cells      quads      frame     stepped    of grid
      262 144     65 536      25 µs        768        85×
    1 048 576    262 144      25 µs        768       341×
    4 194 304  1 048 576      26 µs        768      1365×
   16 777 216  4 194 304      27 µs        768      5461×
```

The last column is the ratio between what was stepped and what exists. It is the number this
repository was written to print. The frame column had to be earned five times over: sixty-four times
the world for two microseconds more, with **nothing in the frame proportional to the grid** — not the
settle, not the passes that build the list it runs on, and not what was left once neither of those
grew, which turned out to be the barriers between them, then one flag named in all of them, and then
the host recording the whole thing afresh every frame.

**The size of that list never crosses the bus.** A pass called `prepare` divides the count and
writes three words into a buffer; Vulkan reads them at dispatch time. The host does not learn how
much work there was unless it asks, which the title bar does twice a second.

---

## How it is put together

Seventeen passes in **eleven stages**, one submission, six words from the host. A barrier sits
between stages and not inside one, and at about 3.8 µs each that grouping is a fifth of the frame —
`live::wired` is where it is decided, and the argument for every seam is beside it. Four of the ten
barriers are followed by a dispatch that reads its workgroup count out of a buffer, and only those
four name `DRAW_INDIRECT`; naming it in the other six cost another fifth.

```
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

## What the tests are for

Every one of them is aimed at a failure that a convincing picture survives.

| file | the claim |
| --- | --- |
| `sweep.rs` | the first tier's offsets are the ones the host computes |
| `compact.rs` | the work list is every awake quad, once, in block order |
| `hot.rs` | and the hot-block list is every hot block, once, in order, across several tiles |
| `sand.rs` | conservation, the host step for step, and that it actually falls |
| `sand.rs` | water reaches all 62 columns of a basin where sand heaps at 22 |
| `sand.rs` | the window shows the cells it is pointed at, from four corners, two wrapping |
| `live.rs` | **the same pour, through the list and over the whole grid, agrees cell for cell** |
| `tally.rs` | the reduction says what a count done by hand says |
| `dispatch.rs` | a workgroup count one stage wrote is the one the next stage is dispatched over |
| `dispatch.rs` | a frame is recorded once, and a write between two of them does not undo that |

`live.rs` is the one that matters. Everything else compares a device against a host computing the
same thing a different way; that cannot work for a wake set, because a host reference would derive
it from the same rule and be wrong in the same place. So it runs the same pour twice — once through
the work list, once by stepping every quad whether it needs it or not — and demands the two grids be
identical every frame.

> *The list is allowed to be shorter. It is not allowed to be different.*

Sand proves the list can reach nothing. Water proves the list is never wrong: a runner crosses the
world sideways through cells that are not changing, which is exactly what a wake rule keyed to
movement would lose. **Both are poured in the same run** — sand, then water, then nothing — because
`sand::TIRED` made the rule depend on a cell's own history, and history is the thing a listed run and
a whole-grid run can disagree about. Six thousand frames, cell for cell, ending at a list of nought.

Every test is run at subgroup **32** (RTX 4080), **64** (integrated Radeon) and **8** and **4**
(Mesa lavapipe). `GRIT_DEVICE` picks by substring.

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
* **A frame's wall clock is a submit and a fence**, about a hundred microseconds on this machine
  whatever is inside it. The table above was measured with `Resident::frames`, which records twenty
  frames into one command buffer and divides. Measured the other way, the coarse level *appeared to
  make things slower* and nearly went in the bin on that evidence — and the hot-block list that came
  after it would have been invisible too, because what it saves at the largest size is 84 µs and a
  round trip costs more than that.
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
