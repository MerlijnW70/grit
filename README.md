# grit

**Sixteen million cells of falling sand and water, stepped in 27 µs a frame.**

A frame touches about seven hundred of the world's four million quads, and the host never learns how
many — the device writes the count into a buffer and Vulkan reads it at dispatch time. That is what
this repository is: a sparse-grid stream compaction engine, built on
[`simdr`](../simdr), with a sand simulation on top of it to prove the claim.

```text
       cells      quads      frame     stepped    of grid
      262 144     65 536      25 µs        768        85×
    1 048 576    262 144      25 µs        768       341×
    4 194 304  1 048 576      26 µs        768      1365×
   16 777 216  4 194 304      27 µs        768      5461×
```

**Sixty-four times the world for two microseconds more.** The last column is the ratio between what
was stepped and what exists, and it is the number this repository was written to print.

---

## Run it

```text
cargo run --release --bin pour       # the demo: 4096×4096, seen through a window
cargo run --release --bin scale      # the table above, measured on your card
cargo run --release --bin loop_cost  # what looking costs, which is why there is a window
cargo test  --release                # every number below, checked
```

In `pour`: **left button** pours sand, **right** pours water, **WASD** flies, **F** follows the
motion, **Escape** leaves. Watch the count beside the frame time — pour, and it climbs into the
hundreds; stop, and it falls to nought while the grid it is not looking at stays four million quads.

---

## Use it

Eleven stages, one submission, and the only thing crossing the bus per frame is six words.

```rust,no_run
use grit::compact::Plan;
use grit::layout::{PARAMS, STATE};
use grit::sand::{AIR, Grid};
use grit::{Groups, Host, sweep};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let Some(host) = Host::open()? else { return Ok(()) };
let width = host.width();
let grid = Grid::of(4096, 4096);
let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width)))?;

// Sixteen passes, grouped into the eleven stages the dependencies allow.
// `None` asks for no picture; `Some(view)` puts `paint` in the fourth stage.
let (passes, stages) = grit::live::wired(width, grid, &plan, None)?;
let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();

// Allocate once. Everything after this is a submission.
let mut resident = host.resident(&plan.sizes(), &modules)?;
resident.write(STATE as usize, &vec![AIR; grid.cells()])?;
resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
grit::live::seed(&mut resident, grid, &plan)?;

loop {
    resident.staged(&groups)?;
}
# }
```

`src/lib.rs` pulls this file in under `cfg(doctest)`, so `cargo test` compiles the block above and a
snippet that stops working stops the build.

**Want the Vulkan without the sand?** [`host`] and [`resident`] name no buffer of the simulation's —
`hyperia` vendored those two files and nothing else. See `decisions/DR-0005`.

---

## Where things are

| | |
| --- | --- |
| `src/resident.rs` | buffers that outlive a frame, stages, one submission, the kept recording |
| `src/host.rs` | a device, opened once, and what it will say about itself |
| `src/compact.rs` | the scan tiers, the gather, and the count the host never sees |
| `src/sweep.rs` | the fused pass: step a cell and count it in the same register |
| `src/live.rs` | the loop — `wired` is the one place its order is decided |
| `src/sand.rs` | the Margolus rule, four ordered swaps, and what a cell word holds |
| `src/layout.rs` | which buffer is which |
| `decisions/` | six records, four headings each: measurement, decision, rejected route, limit |

Start at `src/live.rs`'s `wired` if you want the loop, `src/resident.rs` if you want the Vulkan, and
`decisions/DR-0002` if you want to know which of this repository's claims have been checked.

---

## What the tests are for

Each is aimed at a failure that a convincing picture survives.

| file | the claim |
| --- | --- |
| `live.rs` | **the same pour, through the list and over the whole grid, agrees cell for cell** |
| `sand.rs` | conservation, the host step for step, and that water reaches all 62 columns |
| `compact.rs` | the work list is every awake quad, once, in block order |
| `hot.rs` | and the hot-block list is every hot block, once, across several tiles |
| `sweep.rs` | the first tier's offsets are the ones the host computes |
| `tally.rs` | the reduction says what a count done by hand says |
| `dispatch.rs` | a stage's workgroup count is read by the next, and a frame is recorded once |
| `floor.rs` | a submission costs more than a frame in it, and a barrier more than a dispatch |
| `direct.rs`, `overlap.rs`, `waiting.rs`, `wedged.rs` | the host layer: direct writes, tickets, the fence bound |

`live.rs` is the one that matters. A host reference for a wake set would derive it from the same rule
the kernel does and be wrong in the same place — so the comparison is against **a different
algorithm**: the same pour, stepped over every quad whether it needs it or not, for six thousand
frames.

> *The list is allowed to be shorter. It is not allowed to be different.*

Every kernel is built for subgroup **4, 8, 16, 32 and 64** — one arm each, and `LaneError::NoMapping`
for anything else. `GRIT_DEVICE` picks a device by substring. Every figure on this page is from an
RTX 4080 at subgroup 32.

---

## Read further

**[`DESIGN.md`](DESIGN.md)** — why the partition has no race, how the compaction adds up in three
tiers, why there is a second hierarchy over the first, and what is still true and unflattering.

**[`decisions/`](decisions/)** — what was decided, what it measured, what was rejected at what price,
and what the numbers do not prove.
