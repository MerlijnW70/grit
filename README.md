# grit

Sparse-grid stream compaction on Vulkan compute. Built on `simdr` (SPIR-V emitter, path dependency).

## Commands

```sh
cargo build --release
cargo test                       # whole suite, default device
cargo test --test <name>         # one file
cargo clippy --all-targets
cargo fmt

GRIT_DEVICE=AMD cargo test       # select device by name substring

cargo run --release --bin scale      # frame cost against world size
cargo run --release --bin loop_cost  # per-stage device time
cargo run --release --bin pour       # windowed, needs default feature `window`
cargo run --release --bin spike      # windowed, needs default feature `window`
```

Features: `window` (default) pulls in `minifb`; `default-features = false` drops it.

## Modules

| | |
| --- | --- |
| `host` | device open, memory, queue. `Host::open`, `Host::matching`, `Host::width`, `Host::heap`, `Host::sparse` |
| `resident` | buffers, descriptor set, recording, submission. `Resident`, `Groups`, `Run`, `Ticket` |
| `layout` | binding indices |
| `sweep` | one Margolus step over every quad, and the block sweep |
| `sand` | cell word, the fall rules, `Grid` |
| `compact` | `Plan`, `Tier`, `Over`, `scan`, `gather`, `prepare`, `dense_reference` |
| `live` | the frame loop: `wired`, `arranged`, `both`, `cheaper`, `seed`, and one function a pass |
| `gate` | `if_any`, a workgroup-uniform branch |
| `netlist` | `Netlist`, `Gate`, `Chip`, `Exam`, `drawn`, `turned`, `answered` |

## Bindings

`layout::BINDINGS` = 20. Every kernel declares all of them, in one descriptor set.

| | | | |
| --- | --- | --- | --- |
| 0 `PARAMS` | 5 `TOTALS` | 10 `PIXELS` | 15 `HOT_BASES` |
| 1 `STATE` | 6 `DENSE` | 11 `TALLY` | 16 `HOT_TOTALS` |
| 2 `BUCKETS` | 7 `GRAND` | 12 `HOT` | 17 `HOT_DENSE` |
| 3 `COUNTS` | 8 `INDIRECT` | 13 `HOT_BUCKETS` | 18 `HOT_GRAND` |
| 4 `BASES` | 9 `AWAKE` | 14 `HOT_COUNTS` | 19 `HOT_INDIRECT` |

`PARAMS`: 0 frame, 1 dump base, 2..6 brush (`layout::BRUSH`, 4 words), 6..8 window
(`layout::LOOKING`). `TALLY` is `layout::TALLY_WORDS` = 6.

## Resident

```
host.resident(&sizes, &modules)      // sizes: [usize; BINDINGS], modules: &[&[u32]]
host.resident_direct(..)             // host-visible buffers, no staging
host.resident_near(..)               // device-local preferred
host.resident_vast(..)               // sparse binding; needs Host::sparse
```

| | |
| --- | --- |
| write | `write`, `write_at`, `write_many`, `read` |
| sparse | `bind`, `unbind`, `page`, `bound` |
| run | `frame`, `frames`, `staged`, `staged_frames` |
| run and read | `frame_reading`, `staged_reading`, `staged_reading_both` |
| async | `submit`, `submit_reading`, `submit_reading_both`, `wait`, `wait_reading`, `wait_reading_both` |
| numbers | `on_device`, `stage_times`, `recording`, `reused`, `pushing`, `copying` |
| shape | `bindings`, `passes`, `cached`, `direct`, `patience`, `set_patience` |

`Groups::Fixed(n)` dispatches n workgroups; `Groups::From { binding, at }` reads the count off the
device; `Groups::Counting(n)` is `Fixed(n)` and declares that the stage wrote a count, which puts
`DRAW_INDIRECT` in the barrier behind it. `Fixed(0)` is not recorded, and a stage left empty is
dropped.

`staged` takes `&[&[Groups]]`: one slice a stage, a barrier between stages and none inside one.

## Plan

`compact::Plan::over(subgroup, blocks)` — refuses past `per_block²`, where
`per_block = sweep::cells(subgroup) = subgroup * 8`.

| | |
| --- | --- |
| fields | `subgroup`, `per_block`, `blocks`, `tiles`, `hot_tiles`, `quads`, `grid` |
| sizes | `sizes()`, `showing(view)`, `padded()`, `cold()`, `dump()`, `hot_dump()` |
| counts | `groups()`, `listed()`, `hot_listed()` |

## Live

`live::wired(subgroup, grid, plan, picture)` picks an arrangement by `live::coarse_pays(blocks)`,
which is `blocks >= live::COARSE_FROM`. `live::arranged(.., coarse)` says which one out loud.

`live::both(..)` builds both into one module list with a stage list each, the one not running given
`Fixed(0)`. `live::cheaper(resident, both, budget)` times both on the device and returns
`Cheaper { coarse, listed, every, on_device }`.

Passes: `tick`, `decay`, `wipe`, `wipe_counts`, `settle`, `mark`, and `sand::brush`, `sand::paint`,
`compact::scan`, `compact::gather`, `compact::prepare`.

## Sand

Cell word: material in the low bits, then `RUNNING_LEFT`/`RUNNING_RIGHT`, `HELD`, and a tired
counter to `PATIENCE`. Materials `AIR` 0, `WATER` 1, `SAND` 2, `WALL` 3.

Readers: `material_of`, `running_of`, `held_of`, `tired_of`, `hold_of`. Lane forms: `material`,
`running`, `hold`, `held`, `tired`, `fall`, `course`, `start`.

`Grid::of(wide, deep)` — rounds each side up to a power of two. `cells()`, `quads()`, `across()`, `down()`.

## Netlist

Sources are `inputs + gates`: source `n < inputs` is input `n`, source `inputs + n` is gate `n`.
A gate may name only a source below its own.

A gate is one word: `KIND_BITS` = 8 kind, `SOURCE_BITS` = 12 each reference. `SOURCES` = 4096,
`KINDS` = 256. A kind is the four-bit truth table of the gate: `0b1000` and, `0b1110` or, `0b0110`
xor, `0b1010` left wire.

| | |
| --- | --- |
| shape | `Netlist::over(inputs, outputs, gates, kinds)` → `TooWide` |
| shape | `sources()`, `below(at)`, `words()`, `sizes()` |
| rule | `checked(at, gate)` → `Fault`, `faults(packed)` |
| draw | `drawn(netlist, at, roll)`, `turned(netlist, at, gate, roll)` — legal for any roll |
| run | `Chip::of(shape, gates, outputs)` → `NotAChip`; `sourced`, `answers`, `table` |
| score | `Exam::of(shape, answers)` → `NotAnExam`; `asked`, `marks`, `scored` |
| gate | `Gate::pack`, `Gate::unpack`, `answered(gate, left, right)` |

## Tests

| | |
| --- | --- |
| `live` | list arrangement against whole grid, cell for cell |
| `sand`, `sweep`, `compact`, `hot`, `tally` | the rules and the hierarchy |
| `coarse`, `crossing` | the two arrangements priced, and the device asked which it prefers |
| `floor` | submission, dispatch and barrier cost |
| `dispatch`, `direct`, `overlap`, `waiting`, `wedged`, `vast`, `saying` | `Resident` behaviour |
| `hunt`, `wander`, `counting` | wake bookkeeping, and the flag against its counters |
| `netlist` | the packing, the rule, the draw, the chip and the exam — no device needed |
