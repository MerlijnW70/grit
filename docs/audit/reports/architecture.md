# Grit audit: architecture against the state of the art

Grit at commit `8419acf` and simdr at `9dec3b7`, cloned side by side into this job directory. All runs: Windows 11, NVIDIA GeForce RTX 4080 (driver 610.88, Vulkan 1.4.341, subgroup 32) and AMD Radeon(TM) Graphics iGPU (AMD proprietary driver 2026.04.17, Vulkan 1.4.315, subgroup 64, min 32), Rust release profile. Device picked with `GRIT_DEVICE=NVIDIA` or `GRIT_DEVICE=AMD`. Every probe is in `evidence/` and also lives in the clone as `grit/examples/audit_*.rs`.

Setup used for every command below (PowerShell, from the job directory):

```
git clone F:\grit grit ; git clone F:\simdr simdr
git -C grit checkout 8419acf ; git -C simdr checkout 9dec3b7
New-Item -ItemType Directory grit\examples ; Copy-Item evidence\audit_*.rs grit\examples\
cd grit ; $env:CARGO_TARGET_DIR = "$PWD\target"
cargo build --release --examples
```

## Verdict

As a sparse-activity *execution core*, Grit is already strong. The frame bill follows activity: the RTX 4080 steps a mean of 6 547 of 65 536 quads in a settling 512x512 room. The result is bit-identical between the RTX 4080 at subgroup 32, the AMD iGPU at subgroup 64 and the CPU reference after 2 000 frames, and it is mirror-symmetric with no left/right bias. Determinism this strong is rarer than speed in this field. As an *engine* for this lens, though, it is not near top tier:

- The sparse pipeline is wired to one built-in rule, so no other rule can run on it without editing library code.
- That rule has four materials whose ids are also their density order, a 2x2 Margolus neighbourhood, a top speed of one cell a frame, and no temperature or pressure fields.
- A rule is written twice: once as SPIR-V emitter calls, once as a CPU reference.
- There is no save, load or replay API.
- The world is a single power-of-two torus whose largest size depends on the device's subgroup width (8192x8192 at 32, 2048x2048 at 16).
- The code asks only for Vulkan 1.1, has no subgroup-size control, and no run here or in CI has ever been checked by a validation layer.

Each gap has a clear fix, and the determinism and compaction core means the fixes build on solid ground.

## Ranked findings

| Rank | Title | Severity / gain | One-line evidence |
|---|---|---|---|
| 1 | The sparse pipeline is hard-wired to the sand rule: no pluggable rule | Critical / highest gain | `src/live.rs:536-545` calls `sand::hold/start/course/fall` inside the only listed-step kernel; `src/live.rs:233` hard-codes `sand::brush` |
| 2 | Rule model too narrow for a top-tier material sim | High | 2-bit material id = density order (`src/sand.rs:8-18,264-268`); a single grain falls 253 rows in 254 frames (`audit_cpu`) |
| 3 | No save, load or replay API, though the state needed is tiny | High gain, low cost | `audit_gpu`: resume from STATE + frame word = 0 differing cells, frame off by one = 17; no save/load code in `src/` |
| 4 | World size depends on the subgroup width; one power-of-two torus; no chunking or streaming | High | `audit_reach`: reach 32 768 quads at subgroup 4, 2 097 152 at 16, 16 777 216 at 32; 4096x4096 refused at 16 |
| 5 | Rules are written twice, as SPIR-V emitter calls and as a CPU reference | Medium-high | sand rule: 248 non-blank lines of emitter code + 111 of pipeline kernel vs 93 of CPU reference |
| 6 | Portability: Vulkan 1.1 only, no subgroup-size control, subgroups 4-64 only, no validation layers, CI pins nothing | Medium | `src/host.rs:147` asks for 1.1; `settle(128)` returns `NoMapping`; no `VK_LAYER_KHRONOS_validation` installed; CI checks out simdr `main` |
| 7 | No counter-based random primitive for rules | Medium gain, low cost | no RNG in `src/sand.rs` or `src/live.rs`; a hash exists only in `src/bin/spike.rs:107-115` |
| 8 | Public API is raw binding numbers and hand-sized buffers; the engine crate also holds unrelated subsystems | Medium | `src/layout.rs` exposes 20 binding indices; `src/bin/pour.rs` takes 226 lines to set up a demo; `src/lib.rs` exports `netlist`, `machine`, `serve` |
| 9 | Strength to protect: bit-exact determinism across vendors, subgroup widths and the CPU reference | Strength (keep, add to CI) | `audit_gpu` hash `d5326afd461cd507` on RTX 4080, AMD iGPU and CPU; mirror test 0 cells |

Baseline suite, run in `grit/` with `$env:GRIT_DEVICE="NVIDIA"` (or `"AMD"`) and then `cargo test --release --no-fail-fast`:

- RTX 4080: 327 passed, 0 failed, 2 ignored across 46 test binaries, 686.9 s wall. Log in `evidence/suite-nvidia.txt`.
- AMD iGPU: 327 passed, 0 failed, 2 ignored across 46 test binaries, 1 138.3 s wall. Log in `evidence/suite-amd.txt`.

The wall times are for the whole `cargo test` command, measured with `Measure-Command`.The suite's `SKIPPED` lines go to stderr, which libtest captures for passing tests, so these logs cannot show whether any test skipped itself. The validation layers could not be enabled for these runs, because they are not installed on this machine (finding 6).

---

## 1. The sparse pipeline is hard-wired to the sand rule

**What is wrong.** The only kernel that steps listed quads is `live::listed`, built by `live::settle` (`src/live.rs:478-616`).

- Its body calls the sand rule's functions in a fixed order (`src/live.rs:536-545`: `sand::hold`, `sand::start`, `sand::course`, then `sand::fall` four times).
- It then counts `sand::WATER` and `sand::SAND` cells for the tally (`src/live.rs:556-570`).
- The frame schedule `live::arranged` (`src/live.rs:139-270`) puts `sand::brush` (`:233`) and `sand::paint` (`:167`) into every frame.

There is no trait, closure or kernel parameter through which a user can supply a rule. The board rules in `src/rule.rs` are a closed `enum Kind` of seven variants (`src/rule.rs:33-42`), and they run on a separate dense path, not on the sparse list. The README says "what those bits mean belongs to the rule rather than to the engine". That is true of the cell word but not of the pipeline. To run a new rule sparsely, a user has to fork `live.rs`.

**Evidence.** In the clone, `grep -n "sand::" src/live.rs` lists lines 167, 233, 536-545 and 561-563. `grep -rn "pub trait" src` returns nothing.

**Why it matters.** Every engine in the comparison set separates the stepping machinery from the rule:

- The Powder Toy gives each element a property block and an `Update` function pointer ([WATR.cpp](https://github.com/The-Powder-Toy/The-Powder-Toy/blob/master/src/simulation/elements/WATR.cpp)).
- Sandspiel calls one update function per species through a `SandApi` with relative `get` and `set` ([Making Sandspiel](https://maxbittker.com/making-sandspiel)).
- Golly loads rules from `.rule` files without recompiling ([Golly file formats](https://golly.sourceforge.io/Help/formats.html)).
- CAX makes perceive and update separate modules ([CAX, arXiv:2410.02651](https://arxiv.org/abs/2410.02651)).

Grit's sparse scan-and-gather is its distinctive asset, and today only sand can use it.

**Recommendation.**

- Make a block rule a value. A trait `BlockRule` has a lane-level `step` and a scalar `reference`. `step` maps the four corner vectors and the frame to the next four corners plus a "changed" count, which feeds `wake`. `reference` maps `(frame, [u32; 4])` to `[u32; 4]`.
- Make `listed`/`settle` and `arranged` generic over it. Make the brush, picture and tally passes optional stages that the caller supplies.
- Parametrise `tests/live.rs::against_the_whole_grid` over two rules (sand and, for example, a Margolus billiard-ball or Critters rule), so the existing sparse-equals-dense proof covers any rule.

*Expected effect:* any 2x2 block rule runs on the sparse list, with the same proof of equivalence. *Cost:* about 3-5 engineer-days, an estimate that no command produced. Most of it is moving existing code, since `corners`, `wake` and `tally` already work on generic `Vector`s.

## 2. Rule model too narrow for a top-tier material sim

**What is wrong.**

- *Materials are four, and their ids are their weights.* `MATERIAL_BITS = 2` (`src/sand.rs:18`) gives AIR=0, WATER=1, SAND=2, WALL=3 (`:8-11`). `fall` moves a cell down when `above > below` and `WALL > above` (`src/sand.rs:264-268`), so the id *is* the density order. The test `the_materials_are_in_the_order_the_rule_needs` (`tests/sand.rs:370`) pins that coupling. A fifth material such as oil or gas needs a new bit layout: `CARRIED_BITS` and `TIRED` shift (`:19-24`), and so does every accessor.
- *No per-material property table.* Colours are literals in the paint kernel (`src/sand.rs:790-800`), and density is the id.
- *No fields.* The live pipeline has no temperature, pressure or velocity binding. The `FIELD` binding (`src/rule.rs:17`) belongs only to the board rules.
- *Locality and speed.* A step sees one 2x2 block (`src/live.rs:691-725`), and `wake` marks only the 3x3 quads around a change (`src/live.rs:750-751`). A cell therefore moves at most one cell a frame, and no rule can look beyond its block.

**Evidence.** `target\release\examples\audit_cpu.exe` printed `single grain: fell 253 rows in 254 frames = 0.996 rows per frame` (`evidence/audit_cpu.out.txt`). The probe uses `sand::settle_reference`, which `audit_gpu` (finding 9) shows is bit-identical to the GPU path. At that rate a grain dropped from the top of the README's 4096-row grid needs about 4 095 frames, which is 68 s at 60 frames a second. That figure is arithmetic on the measured rate.

**Why it matters.** Users expect hundreds of materials with properties and state changes.

- The Powder Toy's water carries `Weight`, `HeatConduct`, temperature and pressure thresholds, and transitions to ice and vapour ([WATR.cpp](https://github.com/The-Powder-Toy/The-Powder-Toy/blob/master/src/simulation/elements/WATR.cpp)). The game also simulates air pressure, velocity and heat ([The Powder Toy, fork README](https://github.com/SpikeViper/The-Powder-Toy)).
- Noita adds rigid bodies made of pixels on top of its per-pixel rules ([80.lv on Noita](https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation/)).
- GPU block-CA projects such as sands-of-rust use 2x2 neighbourhoods too ([sands-of-rust](https://github.com/wg-romank/sands-of-rust)). The Margolus choice is sound, but those projects are not the top tier.

**Recommendation.**

- (a) Widen the material id to 8 bits, and move properties (density, flags, colour, transition targets) into a read-only table binding that rules index. *Effect:* up to 256 materials without changing the bit layout again. *Cost:* about 2 days on top of finding 1.
- (b) Add optional per-cell field bindings, such as a 16-bit temperature, with a diffusion pass that reports "changed" to `wake` the way the settle pass does. *Effect:* heat and phase changes, while quiet fields still sleep. *Cost:* about 1 week.
- (c) For speeds above one cell a frame, run k sub-steps per frame over the same list (k `settle` dispatches between scans), or carry a velocity field. *Effect:* falling at k cells a frame, at k times the settle cost per active quad. *Cost:* the sub-step schedule is about 1 day, and the wake radius must grow to k quads.

All costs are estimates.

## 3. No save, load or replay API, though the state needed is tiny

**What is wrong.** Nothing in `src/` writes or reads a world: `grep -rniE "save|snapshot|File::create|fs::write" src` finds only unrelated `serial` variables. A user can call `Resident::read(STATE, n)`. Nothing tells them that the frame counter in `PARAMS[0]` is also part of the world, because the Margolus phase is `frame & 1` (`src/live.rs:701`).

**Evidence.**

```
$env:GRIT_DEVICE="NVIDIA"; target\release\examples\audit_gpu.exe 2000
...
checkpoint_frame_word 999
resume_from_state_plus_frame_differing_cells 0
resume_with_frame_off_by_one_differing_cells 17
```

The AMD run prints the same three lines (`evidence/audit_gpu.nvidia.txt`, `evidence/audit_gpu.amd.txt`). The probe runs 1 000 frames, then saves STATE and `PARAMS[0]`. It builds a fresh `Resident`, writes both back, re-seeds the work list as "everything awake" with `live::seed`, and runs 1 000 more frames. The result equals the uninterrupted 2 000-frame run cell for cell. Resuming with the frame word one off changes 17 cells. So a checkpoint is exactly STATE plus the frame word, and an all-awake re-seed is safe.

**Why it matters.** Sandspiel saves worlds as lossless PNGs and shares them ([Making Sandspiel](https://maxbittker.com/making-sandspiel)). Golly has a macrocell format for universes too large for RLE ([Golly file formats](https://golly.sourceforge.io/Help/formats.html)). With Grit's determinism (finding 9), the initial world plus a log of brush strokes would give exact replay, which most engines cannot offer.

**Recommendation.**

- Add `World::snapshot()`, returning grid, rule id and version, frame and cells, and `World::restore`, which writes STATE and `PARAMS[0]` and calls `live::seed`.
- Store snapshots on disk as a header plus run-length-encoded cells.
- Keep a replay log of `(frame, brush words)`.
- Move the probe's resume check into `tests/`.

*Expected effect:* save, load, undo and exact replay. *Cost:* about 150-300 lines and 1-2 days (estimate). The probe already holds the core of it.

## 4. World size depends on the subgroup width; one torus; no streaming

**What is wrong.**

- *Size ceiling.* The work-list scan has two levels, so `Plan::over` refuses more than `cells(subgroup)^2` blocks, with `cells(s) = 8s` (`src/compact.rs:91-119`, `src/sweep.rs:14-16`). The largest world is a property of the GPU's subgroup width, not of its memory.
- *Shape.* `Grid::of` rounds both sides up to a power of two (`src/sand.rs:131-137`).
- *Edges.* Every coordinate is masked, so the world is a torus (`src/sand.rs:442-454`). A comment in `src/serve.rs` says that without walls "everything leaves by the bottom", and every demo builds a wall frame for this reason.
- *Memory.* The whole grid is one storage buffer. `Resident` can make sparse-residency buffers (`resident_vast`, `src/resident.rs:134-144`, tested in `tests/vast.rs`), but the live pipeline never uses them.

**Evidence.** `target\release\examples\audit_reach.exe` (`evidence/audit_reach.out.txt`):

```
subgroup reach_quads largest_square_grid state_binding_bytes_at_reach
       4       32768    256x256    1048576
       8      262144   1024x1024   8388608
      16     2097152   2048x2048   67108864
      32    16777216   8192x8192   536870912
      64   134217728  16384x16384  4294967296
subgroup 32 grid 16384x16384: refused TooBig 262144 65536
subgroup 16 grid 4096x4096: refused TooBig 32768 16384
subgroup 8 grid 2048x2048: refused TooBig 16384 4096
Grid::of(5000, 3000) -> 8192x4096 = 33554432 cells
```

What the output shows:

- A subgroup-16 or subgroup-8 device cannot plan the README's own 4096x4096 demo.
- CI's lavapipe width-4 leg can plan at most 256x256.
- A 5000x3000 request costs 2.2 times the cells it asked for.
- At subgroup 64 the reach is 16384x32768 cells, not square. Its STATE binding would be 4 294 967 296 bytes, one more than the `maxStorageBufferRange` of 4 294 967 295 that both GPUs report (`evidence/vulkaninfo-full.txt`). `Plan::over` accepts that plan, and nothing in `buffers_in_with` (`src/resident.rs:156-301`) compares binding sizes with `Host::ceiling()`. This case is derived from the printed size and was not run.

**Why it matters.**

- Noita splits its world into 64x64 chunks, each with a dirty rect, and "the engine does allow you to go very very very deep" ([80.lv](https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation/)).
- NanoVDB now rebuilds dynamic GPU sparse grids of hundreds of millions of voxels in milliseconds ([Dynamic GPU Sparse Grids in NanoVDB](https://research.nvidia.com/labs/prl/publication/sifakis2026dynamic/); [NanoVDB](https://research.nvidia.com/labs/prl/publication/nanovdb)).
- Taichi's pointer and bitmasked SNodes activate blocks on write, and its loops visit only active cells ([Taichi sparse docs](https://docs.taichi-lang.org/docs/sparse); [Taichi paper](https://yuanming.taichi.graphics/publication/2019-taichi/)).
- Golly's hashlife stores universes too large for RLE in its macrocell format ([Golly file formats](https://golly.sourceforge.io/Help/formats.html)).

**Recommendation.**

- (a) Add a third scan level, or loop the existing `scan` pass over levels until one group remains, so reach becomes `cells(s)^L` for any L. *Effect:* 4096x4096 plans at subgroups 16 and 8, and no ceiling tied to the subgroup. *Cost:* about 1-2 days, since `compact::scan` already scans `TOTALS` into `GRAND` in a second dispatch. Keep this multi-pass form rather than a single-pass decoupled look-back scan ([Merrill & Garland 2016](https://research.nvidia.com/publication/2016-03_single-pass-parallel-prefix-scan-decoupled-look-back)). Look-back spin-waits across workgroups and so depends on forward-progress behaviour that the Vulkan spec does not promise across vendors (not verified on these GPUs).
- (b) Check every binding against `Host::ceiling()` when building a `Resident`, so an over-range plan is refused instead of being undefined. *Cost:* under an hour.
- (c) Make the edge behaviour a choice between wrapping and walls, and accept any even side length in `Grid`. *Cost:* about 1 day.
- (d) Longer term, tile the world: each coarse-tier block becomes a residency page, bound with the existing `bind`/`unbind` when the coarse list first names it. *Effect:* memory follows activity the way cost already does. *Cost:* about 2-3 weeks.

All costs are estimates.

## 5. Rules are written twice

**What is wrong.** The GPU rule is a sequence of emitter calls (`lanes.splat_bits`, `lanes.select`, and so on).

- `src/sand.rs` makes 232 such calls. The rule functions take 248 non-blank lines (`src/sand.rs:31-122` and `:255-440`), plus 111 for the listed kernel (`src/live.rs:492-616`).
- The same rule is written again as plain Rust in `settle_reference`: 93 non-blank lines (`src/sand.rs:471-573`).

Each new rule needs both, and only tests keep them in step.

**Evidence.** Counts of non-blank lines (`grep -vc '^\s*$'`) over each range:

| Range | Command | Lines |
|---|---|---|
| `src/sand.rs:31-122` | `awk 'NR>=31&&NR<=122' src/sand.rs` | 80 |
| `src/sand.rs:255-440` | `awk 'NR>=255&&NR<=440' src/sand.rs` | 168 |
| `src/live.rs:492-616` | `awk 'NR>=492&&NR<=616' src/live.rs` | 111 |
| `src/sand.rs:471-573` | `awk 'NR>=471&&NR<=573' src/sand.rs` | 93 |

`grep -c "lanes\.\|kernel\." src/sand.rs` gives 232. All were run in Git Bash in the clone.

**Why it matters.** In the comparison set a rule has one source:

- Taichi kernels are plain Python compiled to each backend ([Taichi paper](https://yuanming.taichi.graphics/publication/2019-taichi/)).
- Golly rules are data ([Golly formats](https://golly.sourceforge.io/Help/formats.html)).
- Sandspiel rules are one Rust function per species ([Making Sandspiel](https://maxbittker.com/making-sandspiel)).

Grit's emitter style is precise, but the rule plus its kernel takes 3.9 times the lines of the reference (359 against 93), and every rule has to be written twice.

**Recommendation.** In simdr, put the lane operations behind a trait with two implementations: the SPIR-V emitter, and a scalar `u32` evaluator. A rule written once against that trait yields both the kernel and the CPU reference, and the existing sparse-versus-dense tests compare them for free. *Expected effect:* about half the code per rule, and no hand-kept second copy. *Cost:* about 1-2 weeks in simdr (estimate). An alternative route is a scalar interpreter of the emitted module, since simdr already has a decoder (`simdr/src/decode.rs`).

## 6. Portability

**What is wrong.**

- *API level.* The instance asks for Vulkan 1.1 (`src/host.rs:147`), though both GPUs here offer 1.4 (`evidence/vulkaninfo-summary.txt`).
- *No subgroup-size control.* No pipeline sets `requiredSubgroupSize` or `REQUIRE_FULL_SUBGROUPS`: `grep -rn "RequiredSubgroupSize\|REQUIRE_FULL\|ALLOW_VARYING" src` is empty. simdr emits SPIR-V 1.3 (`simdr/src/kernel/binding.rs:37`). So the kernels run at the device's default width, which is 64 on the AMD iGPU although it supports 32 (`minSubgroupSize = 32` in `evidence/vulkaninfo-full.txt`).
- *Fixed set of widths.* Every kernel is instantiated only for 4, 8, 16, 32 and 64 lanes (for example `src/live.rs:478-490`). `audit_reach` prints `settle at subgroup 128: NoMapping { lanes: 128, width: 128 }`.
- *No validation.* The host never enables a layer or a debug messenger, and this machine has no `VK_LAYER_KHRONOS_validation`. `vulkaninfo --summary` lists six layers, none from Khronos, and the registry key `HKLM:\SOFTWARE\Khronos\Vulkan\ExplicitLayers` does not exist. No run here or in CI has been checked by the validation layers.
- *CI.* CI checks out `MerlijnW70/simdr` with no `ref:` (`.github/workflows/ci.yml`, all three jobs). A CI run builds whatever simdr `main` is at that moment, not a pinned commit.
- *Single API.* Vulkan only, through `ash`.

**Evidence.** The points above, plus the full suite passing on both GPUs at their native widths of 32 and 64 (see the suite line at the top). That shows the width-generic code works wherever it is instantiated.

**Why it matters.** Subgroup size control is core in Vulkan 1.3. The extension's reference page says "Previously Vulkan exposed a single subgroup size per physical device" ([VK_EXT_subgroup_size_control](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_subgroup_size_control.html)). Requiring full subgroups guarantees that every invocation is active ([Vulkan spec, pipelines chapter](https://docs.vulkan.org/spec/latest/chapters/pipelines.html)). WebGPU now exposes subgroups, and tells authors to check `subgroupMinSize` and `subgroupMaxSize` when an algorithm assumes a width ([Chrome 134 WebGPU notes](https://developer.chrome.com/blog/new-in-webgpu-134)). Grit's maximum world size also depends on the width (finding 4).

**Recommendation.**

- (a) Ask for Vulkan 1.3 where available, and chain `VkPipelineShaderStageRequiredSubgroupSizeCreateInfo` with `REQUIRE_FULL_SUBGROUPS` for the width Grit compiled for. *Effect:* one width per plan on every device, and the AMD iGPU can run at 32 like the RTX 4080. *Cost:* about 1 day.
- (b) Add a 128-lane instantiation, or derive all kernels from one const-generic list. *Cost:* hours.
- (c) Install `vulkan-validationlayers` in the lavapipe CI job, run the suite with `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`, and fail on any message. *Effect:* catches synchronisation and descriptor-range errors such as the 4 GiB case in finding 4. *Cost:* about half a day.
- (d) Pin simdr in CI with `ref: <sha>`. *Cost:* minutes.

All costs are estimates. A Metal or WebGPU backend would need simdr's SPIR-V translated, for example by naga or SPIRV-Cross. Whether those handle its subgroup operations is not verified here.

## 7. No counter-based random primitive

**What is wrong.** No rule has access to randomness. `grep -rniE "random|rng" src/sand.rs src/live.rs` is empty. A position-and-frame hash exists only inside the demo `src/bin/spike.rs:107-115` (`GOLDEN`, `STIR`). The sand rule gets its symmetry from fixed logic, and that works (finding 9). Stochastic behaviour has no primitive to build on: fire, erosion, gas diffusion, or just breaking up regular slopes.

**Why it matters.** Sandspiel's rules use `rand_dir()` ([Making Sandspiel](https://maxbittker.com/making-sandspiel)). The way to keep randomness *and* determinism on a GPU is a counter-based generator keyed by (cell, frame, seed), such as Philox or Threefry ([Random123, Salmon et al., SC11](https://www.thesalmons.org/john/random123); [paper PDF](https://users.cs.utah.edu/~hari/teaching/bigdata/random123sc11.pdf)). With that, replays (finding 3) stay exact.

**Recommendation.** Add a `random(cell_index, frame, seed)` lane operation to simdr or Grit, using Philox-2x32 or the existing `spike.rs` hash, with the seed in `PARAMS` and a matching scalar version for references. *Effect:* stochastic rules stay bit-exact across devices. *Cost:* about 1 day (estimate).

## 8. Public API: binding numbers, hand sizing, mixed scope

**What is wrong.** To run the shipped rule, a user must:

- pick raw binding numbers (`src/layout.rs` exports 20 `u32` binding constants);
- size all 20 buffers from `Plan::sizes` and zero each one;
- write `PARAMS` as `[u32::MAX, plan.dump()]`;
- call `live::seed`;
- write brush words at fixed offsets.

`src/bin/pour.rs` takes 226 lines for that. The crate also exports `netlist`, `machine`, `marking`, `board` and `serve` (`src/lib.rs`), which handle circuit synthesis and adjoint learning, next to the simulation engine.

**Why it matters.** Engines at this level expose a world object with typed operations, such as Sandspiel's `Universe` with tick and paint ([Making Sandspiel](https://maxbittker.com/making-sandspiel)). A mixed crate makes the engine's surface hard to find and hard to version.

**Recommendation.** Wrap `Resident` in `World::new(host, rule, grid)`, with `step(n)`, `paint(..)`, `read_region(..)` and `snapshot()`, and keep the binding layout private. Move the synthesis and learning modules into their own crate in the same workspace. *Effect:* a demo shrinks to roughly 30 lines, and the engine API can be versioned. *Cost:* about 3-4 days (estimate), best done together with finding 1.

## 9. Strength to protect: bit-exact determinism

**What holds.** Three properties make the result independent of list order, vendor and subgroup width:

- The 2x2 Margolus partition means no two invocations write the same cell.
- The tally uses integer atomics, which commute.
- The sand path uses no floats.

**Evidence.**

```
$env:GRIT_DEVICE="NVIDIA"; target\release\examples\audit_gpu.exe 2000
device NVIDIA GeForce RTX 4080 subgroup 32
mean_listed 6547 of 65536
gpu_hash d5326afd461cd507
cpu_hash d5326afd461cd507
gpu_vs_cpu_reference_differing_cells 0

$env:GRIT_DEVICE="AMD"; target\release\examples\audit_gpu.exe 2000
device AMD Radeon(TM) Graphics subgroup 64
mean_listed 6547 of 65536
gpu_hash d5326afd461cd507
cpu_hash d5326afd461cd507
gpu_vs_cpu_reference_differing_cells 0
```

`audit_cpu` shows the rule has no handedness. It prints `pour at x=128 mirrored vs pour at x=127 differing cells 0` for both sand and water, and `random blob vs its mirror image after 2000 frames: 0 cells break mirror symmetry`. The suite already proves that the sparse list equals a dense run (`tests/live.rs::the_list_is_shorter_and_never_different`). The suite does not cover identity across vendors or against the CPU reference.

**Why it matters.** Many GPU stacks give up reproducibility. PyTorch states that "Completely reproducible results are not guaranteed across PyTorch releases, individual commits, or different platforms" ([PyTorch reproducibility notes](https://docs.pytorch.org/docs/2.14/notes/randomness.html)). Noita avoids conflicts in its threaded update with a checker pattern of four passes ([80.lv](https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation/)). Grit gets the same freedom from conflicts with two alternating Margolus phases, one dispatch a frame.

**Recommendation.** Make determinism a contract:

- Add `audit_gpu`'s comparison of the GPU against `settle_reference` as a test, so CI runs it on lavapipe at widths 4, 8 and 16.
- Require every new rule (finding 1) to ship its reference.
- Require any future float field to use fixed point.

*Effect:* determinism becomes a tested guarantee rather than a property of the current rule. *Cost:* about half a day (estimate).

---

## Facts not verified

- Noita details come from the 80.lv article, not from the GDC talk itself ("Exploring the Tech and Design of Noita", 2019, listed on the [Noita wiki](https://noita.wiki.gg/wiki/Official_Interviews_and_Videos)). GDC Vault was in maintenance when I tried it.
- Whether naga or SPIRV-Cross can translate simdr's subgroup SPIR-V for Metal or WebGPU was not tested.
- The subgroup-64 4 GiB binding case comes from `Plan::sizes` and `vulkaninfo`. It was not run.
- Whether decoupled look-back hangs on either GPU here was not tested.
- Every engineer-day cost in this report is an estimate. No command produced it.

## Sources

- The Powder Toy water element: https://github.com/The-Powder-Toy/The-Powder-Toy/blob/master/src/simulation/elements/WATR.cpp ; feature overview in a fork's README: https://github.com/SpikeViper/The-Powder-Toy
- Making Sandspiel: https://maxbittker.com/making-sandspiel
- sands-of-rust (GPU block CA): https://github.com/wg-romank/sands-of-rust
- Noita on 80.lv: https://80.lv/articles/noita-a-game-based-on-falling-sand-simulation/ ; talk listing: https://noita.wiki.gg/wiki/Official_Interviews_and_Videos
- Golly file formats and RuleLoader: https://golly.sourceforge.io/Help/formats.html , https://golly.sourceforge.io/Help/Algorithms/RuleLoader.html
- CAX: https://arxiv.org/abs/2410.02651
- Taichi: https://yuanming.taichi.graphics/publication/2019-taichi/ , https://docs.taichi-lang.org/docs/sparse
- NanoVDB: https://research.nvidia.com/labs/prl/publication/nanovdb ; dynamic grids: https://research.nvidia.com/labs/prl/publication/sifakis2026dynamic/
- Merrill and Garland, decoupled look-back: https://research.nvidia.com/publication/2016-03_single-pass-parallel-prefix-scan-decoupled-look-back
- Random123 and the SC11 paper: https://www.thesalmons.org/john/random123 , https://users.cs.utah.edu/~hari/teaching/bigdata/random123sc11.pdf
- Vulkan subgroup size control: https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_subgroup_size_control.html ; pipelines chapter: https://docs.vulkan.org/spec/latest/chapters/pipelines.html
- WebGPU subgroups (Chrome 134): https://developer.chrome.com/blog/new-in-webgpu-134
- PyTorch reproducibility: https://docs.pytorch.org/docs/2.14/notes/randomness.html

## Evidence index

| File | Produced by (in `grit/`, after the setup above) |
|---|---|
| `evidence/audit_gpu.rs`, `audit_gpu.nvidia.txt`, `audit_gpu.amd.txt` | `$env:GRIT_DEVICE="NVIDIA"` (or `"AMD"`); `target\release\examples\audit_gpu.exe 2000` |
| `evidence/audit_cpu.rs`, `audit_cpu.out.txt` | `target\release\examples\audit_cpu.exe` |
| `evidence/audit_reach.rs`, `audit_reach.out.txt` | `target\release\examples\audit_reach.exe` |
| `evidence/suite-nvidia.txt`, `suite-amd.txt` | `$env:GRIT_DEVICE="NVIDIA"` (or `"AMD"`); `cargo test --release --no-fail-fast` |
| `evidence/vulkaninfo-summary.txt`, `vulkaninfo-full.txt` | `vulkaninfo --summary`, `vulkaninfo` |
