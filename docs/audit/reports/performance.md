# Grit audit: performance

This audit measures Grit 8419acf on simdr 9dec3b7 on this machine's two cards: an NVIDIA GeForce RTX 4080 (driver 610.88, subgroup 32) and an AMD Radeon(TM) Graphics iGPU (driver 2.0.353, subgroup 64), under Windows 11 and Vulkan 1.4. Other GPU jobs ran at the same time; at one point `nvidia-smi` showed 97 % load from other processes. So every timing comes from many single-frame submissions, reported as min / median / max. Every number here was produced by a command in this report. Raw outputs are in `evidence/runs/` and probe sources in `evidence/probes/`.

## Verdict

Grit's frame is already cheap in the cases it was tuned for:

- An idle 67 M-cell grid costs about 15 µs on the 4080.
- A busy scene's cost grows linearly with active quads: 0.15 to 0.19 ns per active quad from 2048² to 8192².
- The emitted SPIR-V uses few registers and never spills.

It is not yet top tier. Five problems account for most of the gap, and each one is measured:

1. **"The bill follows activity, not extent" holds only for activity along rows.** A block is a horizontal strip of 256 quads (512 on AMD). So 4 102 active quads cost 1.7× (4080) to 29× (iGPU) more in a column than in a row, and the gap grows with the grid.
2. **The six `TALLY` atomics in `settle` serialise on six addresses.** Spreading them over more addresses cuts a busy 4096² frame on the 4080 by 38–42 %.
3. **`decay` stores each value before loading the next.** The same work with the loads issued first is 2.0× (4080) to 5.2× (iGPU) faster. That change alone cuts the iGPU column frame from 1.2 ms to 0.41 ms, and the cell state comes out identical.
4. **The interactive `pour` loop makes three blocking submissions a frame.** Its wall time is 4.5× its GPU time.
5. **The switch between one and two tiers is a constant tuned on the 4080.** On the iGPU it costs 12× at 1024².

Memory runs at 12.27 bytes of device memory per 4-byte cell, plus three host copies of the largest buffer. The largest grid on the 4080 is 8192². The two-level scan sets that limit, not memory.

## Findings

| Rank | Title | Severity / gain | One-line evidence |
|---:|---|---|---|
| 1 | Blocks are one-quad-tall row strips, so cost follows the rows touched, not activity | High: 1.7× (4080) to 55× (iGPU) on vertical activity | `census 4096`, 4102 listed quads: row 18.8–18.9 µs vs column 31.4–31.5 µs (4080); 34.9–40.2 µs vs 1.16–1.19 ms (iGPU); 128× / 256× quads processed per listed quad |
| 2 | Every workgroup's six `TALLY` atomics hit the same six words | High on NVIDIA: busy frame −38 to −42 % | `fixes 4096` 4080 busy: 316.6 → 194.8 µs and 412.5 → 238.6 µs (spread); identical state and tally |
| 3 | `decay` stores each value before the next load, so its eight loads run one at a time | High on the iGPU: column frame 2.8–2.9× faster; busy −9 to −11 % on both cards | `fixes 4096` iGPU column 1.155–1.198 ms → 0.410 ms; 4080 busy 316.6 → 283.1 µs; identical state hash |
| 4 | Three blocking submissions per frame in the interactive loop | Medium: 1.66× frame rate (4080), 1.7× (iGPU) | `frameloop` 4080: wall 528 µs vs GPU 117 µs; two frames in flight: 319 µs |
| 5 | The tier switch is a constant tuned for NVIDIA | Medium: 12× on the iGPU at 1024² | `CENSUS_COARSE=0/1 census 1024`, iGPU idle: 154.7 µs (one tier) vs 12.7 µs (two tiers) |
| 6 | 12.27 B/cell on the device, plus three copies of the largest buffer in host memory | Medium: −40 % device memory, host staging from hundreds of MiB to under 10 MiB | `budget`: 4096² = 196 MiB device + 384 MiB staging; `STATE` is twice the grid |
| 7 | The chain of dependent dispatches sets a ~15 µs floor per frame | Low to medium: about 1.3 µs per stage removed | `floor`: 1.1–1.4 µs per dependent pass (4080), 0.8–2.0 µs (iGPU) |
| 8 | The two-level scan, not memory, caps the largest grid | Medium for "largest grid" | `budget`: 8192² uses 785 MiB of 16 GiB on the 4080, and `Plan::over` refuses 16384²; the iGPU allocates 16384² |
| 9 | Grit's timing tools hide or misattribute cost | Low (tooling) | TOP_OF_PIPE stage stamps; batch averages absorb other processes' GPU time; the `pour` title leaves out two uploads; no validation layer; no pipeline cache |
| 10 | Checked and found fine: registers, occupancy, the strided lane layout | None (non-finding) | `regs`: ≤37 registers (4080), ≤40 VGPRs (iGPU), no scratch; `pattern`: strided vs coalesced within 5 % (651 vs 640 GB/s on the 4080) |

## How to reproduce

Run all commands in PowerShell from the job directory (the folder holding this report, `grit\` and `simdr\`). The clones are at grit 8419acf and simdr 9dec3b7, as the spec asks.

`evidence\run.ps1 <probe> <nvidia|amd> [-Rest args]` does four things:

1. It checks both commits.
2. It copies `evidence\probes\<probe>.rs` into `grit\examples\`.
3. It sets `GRIT_DEVICE`, Grit's own device selector.
4. It runs `cargo run --release --example <probe>`.

Example: `.\evidence\run.ps1 census nvidia -Rest 4096`. The first build of a probe takes a few minutes. The probes add files only under `grit\examples\`. Nothing in `src\` or `tests\` was changed, and `git -C grit status` shows only `?? examples/`.

Some run files were produced by earlier versions of the same probes, before they were changed to time single-frame submissions or gained variants:

- `census-*-a.txt` and `census-*-b.txt` (card columns batch-averaged)
- `fixes-*-a.txt` and `fixes-*-b.txt` (no spread variant)
- `pattern-*-a.txt` (batch-averaged)

Every number cited here comes from the current probes (`-c`, `-d`, `-final`, the per-size files, `census-forced-tier.txt`), unless the text says otherwise. `scale-*`, `stages-*` and `loop_cost-*` are three runs of Grit's own table tools (`grit	argetelease\<tool>.exe` with `GRIT_DEVICE` set).

---

## 1. Blocks are one-quad-tall row strips, so cost follows the rows touched, not activity

**What is wrong.** Quads are numbered row by row (`src/live.rs:702-704`: `down = quad >> log2(across)`, `along` = the low bits). A block is `sweep::cells(subgroup)` consecutive quad numbers (`src/compact.rs:103-119`). That is 256 quads on the 4080 and 512 on the iGPU: a strip one quad row tall and 512 or 1024 cells wide. `wake` marks whole blocks hot (`src/live.rs:788-802`: `HOT[slot >> log2(per_block)]`). Then `decay`, `mark fine` and `gather fine` process every quad of every hot block. So the first active quad in a new row costs a full block of 256 or 512 quads.

**Evidence.** `evidence/probes/census.rs` builds a walled 4096² room with 512 falling sand grains spaced 4 cells apart. In the `row` scene they lie in one row; in the `column` scene they stand in one column. Every grain has air under it, so all of them move every frame. The probe reads `GRAND` (quads listed) and `HOT_GRAND` (blocks listed). It times 63 single-frame submissions with timestamp queries.

```
.\evidence\run.ps1 census nvidia -Rest 4096
.\evidence\run.ps1 census amd -Rest 4096
```

Results from `evidence/runs/census-nvidia-c.txt` and `census-amd-c.txt`, with reruns in `census-*-final.txt`:

| Card | Scene | Listed quads | Hot blocks | Quads in hot blocks | Amplification | Card time min / med / max |
|---|---|---:|---:|---:|---:|---|
| RTX 4080 | row | 4102 | 20 | 5120 | 1.2 | 18.944 / 19.744 / 20.896 µs |
| RTX 4080 | column | 4102 | 2052 | 525312 | 128.1 | 31.456 / 32.192 / 33.024 µs |
| iGPU | row | 4102 | 12 | 6144 | 1.5 | 40.160 / 42.440 / 49.520 µs |
| iGPU | column | 4102 | 2052 | 1050624 | 256.1 | 1.156 / 1.202 / 1.348 ms |

The final rerun gives 4080 row 18.848 µs and column 31.392 µs, and iGPU row 34.880 µs and column 1.186 ms.

- **Same work listed, far more done.** Both scenes list the same 4102 quads. The column scene processes 128× (4080) or 256× (iGPU) as many.
- **`decay` takes most of it.** Its stage alone is 12.0 µs vs 2.5 µs on the 4080, and 897 µs vs 8 µs on the iGPU.
- **The gap grows with height.** At 8192² (`-Rest 8192`, files `census-nvidia-8192.txt` and `census-amd-8192.txt`) it is 19.5 µs vs 51.7 µs on the 4080 (2.65×) and 61.1 µs vs 3.351 ms on the iGPU (55×).

**Why it matters.** Most interesting scenes do not run along rows: a falling column, a waterfall, the slope of a tall pile, a vertical wire. The README's central claim ("the bill follows activity and not extent") is true only for horizontal activity. In a top-tier sparse engine, the cost of a small active set depends on its area, not on how many rows it spans.

**Recommendation.** Number quads in tiles, so that a block is a square of 16×16 quads on subgroup 32, or 32×16 on subgroup 64, instead of a 256×1 strip. Either a tiled order (block index from the high bits of x and y) or a Morton order inside the block works. The index math sits in a few places:

- `corners` (`src/live.rs:691-725`)
- the neighbour math in `wake` (`src/live.rs:750-800`)
- `brush` and `paint` in `src/sand.rs`
- the host code that writes the room and reads it back

*Expected effect (estimate, not measured).* A one-quad-wide column 2048 quad rows tall would touch about 2048 / 16 = 128 square blocks plus their neighbours, against the 2052 strip blocks measured. That is roughly 10× less decay, mark and gather work. It would bring the iGPU column frame from about 1.2 ms toward the tens of µs of the row frame. Row scenes keep their locality.

*Cost.* About 1–2 days of index rewrites, plus updating the `*_reference` functions that the tests check against, and rerunning `tests/`.

## 2. Every workgroup's six `TALLY` atomics hit the same six words

**What is wrong.** `tally` in `settle` (`src/live.rs:664-681`) issues six `atomic_add` instructions from every lane of every workgroup to the same six words, `TALLY[0..6]`. Lane 0 adds the subgroup's sum and every other lane adds 0. The hardware merges the lanes within one instruction, but every workgroup in the dispatch still queues on the same six addresses.

**Evidence: micro-probe.** `evidence/probes/pattern.rs` runs 131 072 one-subgroup workgroups, which is the dispatch for a fully awake 4096² grid at 32 quads per workgroup.

```
.\evidence\run.ps1 pattern nvidia
.\evidence\run.ps1 pattern amd
```

Min / median / max of 31 single submissions, from `evidence/runs/pattern-nvidia-c.txt` and `pattern-amd-c.txt`:

| Card | Variant | Min | Median | Max |
|---|---|---:|---:|---:|
| RTX 4080 | 6 atomics, every lane (as `settle` does) | 360.19 µs | 360.77 µs | 362.88 µs |
| RTX 4080 | 6 atomics, leader lane only (branch) | 324.83 µs | 325.60 µs | 327.26 µs |
| RTX 4080 | 6 atomics spread over 1024 slots each | 57.98 µs | 58.75 µs | 59.30 µs |
| RTX 4080 | no atomics, same dispatch | 57.28 µs | 57.73 µs | 58.40 µs |
| iGPU | 6 atomics, every lane (as `settle` does) | 723.72 µs | 725.88 µs | 726.28 µs |
| iGPU | 6 atomics spread over 1024 slots each | 65.27 ms | 67.19 ms | 76.78 ms |
| iGPU | spread, leader lane only (branch) | 704.08 µs | 706.76 µs | 713.88 µs |
| iGPU | no atomics, same dispatch | 236.12 µs | 236.24 µs | 237.08 µs |

**Evidence: real frame.** `evidence/probes/fixes.rs` replaces pass 5 of Grit's real 15-pass 4096² frame with a copy of `settle` whose only change is where the tally adds. It checks that the state hash and the summed tally equal those of unmodified Grit after the same frames.

```
.\evidence\run.ps1 fixes nvidia -Rest 4096
.\evidence\run.ps1 fixes amd -Rest 4096
```

On the 4080, busy scene (2.1 M listed quads), card time min / med / max:

| Run (file) | Grit 8419acf | Tally spread over 1024 slots | Gain at min |
|---|---|---|---:|
| quiet (`fixes-nvidia-c.txt`) | 316.576 / 330.656 / 357.632 µs | 194.816 / 204.768 / 235.744 µs | −38 % |
| contended (`fixes-nvidia-d.txt`) | 452.8 µs min | 299.6 µs min | −34 % |
| rerun (`fixes-nvidia-final.txt`) | 412.5 µs min | 238.6 µs min | −42 % |

Every variant ends with state hash `966db4dc8c7b37cd` and tally `[4184258, 0, 2092129]`, the same as Grit.

On the iGPU (`fixes-amd-c.txt`, `-d.txt`, `-final.txt`) the result is the opposite:

- Spreading is 3.7× *slower*: 7.40 ms → 27.1–27.2 ms.
- The leader-lane branch helps a little: 7.40 ms → 6.78–6.79 ms (−8 %).

**Why it matters.** In a busy frame on the 4080, `settle` takes 258 µs of 347 µs (`census-nvidia-c.txt`). Spreading the tally takes about 120–170 µs off the whole frame. Statistics are engine overhead and should cost close to nothing.

**Recommendation.**

- **On NVIDIA:** add into `TALLY[word*1024 + (group & 1023)]`. Fold the 6×1024 slots on the host when the tally is read; `pour` reads it twice a second. Or fold them in an existing single-workgroup pass.
- **On AMD:** keep the single address and branch so only lane 0 issues the atomic.
- **Choosing between them:** pick per device with Grit's existing clock-based chooser (`live::cheaper`).
- **Not measured:** an atomic-free version, in which the leader writes per-workgroup partial sums that the scan pass folds.

*Expected effect.* A busy 4096² frame runs 38–42 % faster on the 4080 and 8 % faster on the iGPU (both measured above).

*Cost.* About half a day. `TALLY` grows from 6 to 6 144 words, and its reader (`src/bin/pour.rs:132`) sums the slots.

## 3. `decay` stores each value before the next load, so its eight loads run one at a time

**What is wrong.** `decayed` (`src/live.rs:404-417`) runs `load AWAKE[q]; store AWAKE[q]` eight times in a row on the same buffer. Its eight loads do not overlap; the measurements below show they run one after another, and issuing them first fixes it. simdr declares every binding without `Restrict` or `NonWritable` (`simdr/src/kernel/binding.rs:57-69`), which may be why the compilers keep each store ahead of the next load (not verified). `sweep`, `mark` and `scan` already issue all eight loads before any store.

**Evidence: micro-probe.** Same buffer, same 8-words-per-lane layout, 16 M words (`pattern-*-c.txt`):

| Card | Variant | Min time | Bandwidth |
|---|---|---:|---:|
| RTX 4080 | store before next load (as `decay` does) | 315.49 µs | 425 GB/s |
| RTX 4080 | all loads first | 156.19 µs | 859 GB/s (the data fits in the 64 MB L2) |
| iGPU | store before next load (as `decay` does) | 10.77 ms | 12 GB/s |
| iGPU | all loads first | 2.14 ms | 63 GB/s |

**Evidence: real frame.** `fixes` with loads issued first in `decay` as the only change. The state hashes stay identical. Min card time in runs c / d / final:

| Card | Scene | Grit | Loads first | Gain |
|---|---|---|---|---|
| iGPU | column | 1.198 / 1.165 / 1.155 ms | 409.9 / 409.6 / 410.2 µs | 2.8–2.9× |
| iGPU | busy | 7.398 / 7.392 / 7.409 ms | 6.741 / 6.729 / 6.741 ms | −9 % |
| RTX 4080 | busy | 316.6 / 452.8 / 412.5 µs | 283.1 / 410.0 / 372.5 µs | −9 to −11 % |
| RTX 4080 | column | 37.7 / 37.5 / 41.3 µs | 28.7 / 28.0 / 33.3 µs | −19 to −25 % |

**Why it matters.** `decay` is the stage that pays for every block listed in finding 1, so this defect multiplies that one. On the iGPU it takes 897 µs of the 1.156 ms column frame, and 122 µs of the 155 µs idle frame at 1024² with one tier (`census-forced-tier.txt`).

**Recommendation.** Emit all eight `AWAKE` loads first, then the eight stores, as `fixes.rs::decay_batched` does. Optionally, have simdr emit `NonWritable` and `Restrict` on bindings a pass only reads.

*Expected effect.* As measured above: 2.8–2.9× on the iGPU column frame, −9 to −11 % on busy frames on both cards, with bit-identical results.

*Cost.* Under an hour for `decay`. About half a day to audit the other passes for the same order of loads and stores.

## 4. Three blocking submissions per frame in the interactive loop

**What is wrong.** Each `pour` frame makes three round trips to the GPU, one after another:

1. `write_at(LOOKING)` (`src/bin/pour.rs:99`)
2. `write_at(BRUSH)` (`src/bin/pour.rs:115`)
3. `staged_reading` (`src/bin/pour.rs:121`), which submits the frame and waits for it

The two writes each go through `Resident::once` (`src/resident.rs:1523-1546`), which records a command buffer, submits it and calls `wait_for_fences`. Nothing overlaps: the CPU waits on the GPU three times a frame, and the GPU sits idle while the CPU copies the 2 MiB picture. The "ms a frame" in the window title (`src/bin/pour.rs:144`) adds up only the third wait.

**Evidence.** `evidence/probes/frameloop.rs` runs `pour`'s exact loop without a window: the 4096² room with shelves, a 1024×512 view, and the brush pouring for half the frames. It also runs two variants:

- **Host-visible:** the buffers of 64 bytes or less (`PARAMS` and the like) go into device-local, host-visible memory, through Grit's existing `Host::resident_near`.
- **Two in flight:** host-visible as above, and two frames in flight, using Grit's own `submit_reading` and `wait_reading`.

```
.\evidence\run.ps1 frameloop nvidia
.\evidence\run.ps1 frameloop amd
```

On the 4080, with the GPU otherwise quiet (`frameloop-nvidia-b.txt`; 5 rounds of 300 frames, sorted):

| Loop | Wall time per frame (5 rounds) | `spent` | Card |
|---|---|---|---|
| serial (as `pour`) | 528.3, 535.7, 589.6, 619.1, 658.2 µs | ~163–193 µs | ~115–129 µs |
| host-visible | 446.9, 457.4, 515.1, 516.4, 556.2 µs | | |
| two in flight | 318.9, 336.8, 348.8, 366.0, 464.9 µs (3136 fps best) | | |

On the iGPU (`frameloop-amd-a.txt`), with GPU time about 450 µs:

- `pour` loop: 2.361–10.04 ms wall per frame
- host-visible: 1.286 ms
- two in flight: 0.835 ms

The second iGPU run (`frameloop-amd-b.txt`) was heavily contended, at 6–694 fps; it is kept only to show the spread.

`floor` measures a single submit-and-wait round trip at 44–55 µs on the quiet 4080 and 50–77 µs on the iGPU (`floor-nvidia-b.txt`, `floor-amd-a.txt`, `floor-amd-b.txt`).

**Why it matters.** At these frame costs the host-side protocol sets the frame rate, not the GPU. A top-tier interactive engine overlaps CPU and GPU work and never makes a round trip to upload 16 bytes of parameters.

**Recommendation.**

1. Build `pour` and the other windowed binaries with `host.resident_near(&sizes, &modules, 64)`, so `write_at(PARAMS…)` becomes a plain store into mapped memory.
2. Keep two frames in flight with `submit_reading` / `wait_reading`, as `src/bin/loop_cost.rs` already demonstrates.
3. Give each in-flight slot its own copy of the brush parameters, or pass them as push constants, so a frame in flight is never written under.

*Expected effect.* 1.66× frame rate on the 4080 (528 → 319 µs best case, measured) and 1.7× on the iGPU (1.44 → 0.83 ms, measured in run a).

*Cost.* About half a day per binary. Push constants need a simdr `PushConstant` block, about 1 day more.

## 5. The tier switch is a constant tuned for NVIDIA

**What is wrong.** `wired` picks one tier or two with `coarse_pays(blocks) = blocks >= 2048` (`src/live.rs:22-27`, `src/live.rs:35`). Grit already has a chooser that measures both (`live::cheaper`, `src/live.rs:316-345`), but the binaries do not use it. On subgroup 64 the same grid has half as many blocks, and the iGPU's one-tier `decay` is slow (finding 3). So small grids land in the wrong tier there.

**Evidence.** `census` with the tier forced through the `CENSUS_COARSE` environment variable:

```
$env:CENSUS_COARSE='0'; .\evidence\run.ps1 census amd -Rest 1024
$env:CENSUS_COARSE='1'; .\evidence\run.ps1 census amd -Rest 1024
Remove-Item Env:CENSUS_COARSE
```

From `evidence/runs/census-forced-tier.txt`:

| Card, 1024² | Tiers | Idle | Row |
|---|---|---:|---:|
| iGPU | one (what `wired` picks) | 154.68 µs | 160.92 µs |
| iGPU | two | 12.72 µs | 24.20 µs |
| RTX 4080 | one (what `wired` picks) | 16.70 µs | 18.24 µs |
| RTX 4080 | two | 15.20 µs | 18.75 µs |

On the iGPU, the 1024² grid idles at 155 µs while the 4× larger 2048² grid, which runs two tiers, idles at 15.9 µs (`census-amd-2048.txt`). The same command at 512² gave one tier 49.9 µs and two tiers 13.6 µs; that console run was not saved, so rerun it with `-Rest 512`.

**Why it matters.** The engine's cost model should hold on every card it runs on. Here a smaller grid runs 10× slower than a larger one.

**Recommendation.** Call `live::both` and `live::cheaper` once at start-up in `wired` or the binaries, and keep the cheaper arrangement.

*Expected effect.* Small-grid frames 6–12× faster on the iGPU (measured above), no change on the 4080.

*Cost.* A few hours. Building both arrangements adds pipeline creation, which `frameloop` measured at 6–128 ms per resident.

## 6. 12.27 B/cell on the device, plus three copies of the largest buffer in host memory

**What is wrong.** Three allocations are larger than they need to be:

- **`STATE` is twice the grid.** `Plan::sizes` (`src/compact.rs:127-160`) sizes it at `grid + quads*4` words. The second half is a dump target: lanes past the end of the list store their four corners there (`src/live.rs:574-582`, `spare = cells + slot*4 + corner`). The dump is indexed by the lane's absolute slot in the list, although only lanes of the last workgroup can be past the end.
- **`AWAKE` is doubled** for the same kind of per-slot dump (`quads + quads.max(SEATS)`, `src/compact.rs:148`).
- **Host staging holds three copies of the largest binding.** `buffers_in_with` allocates `SLOTS + 1 = 3` host-visible staging buffers, each as large as the largest binding (`src/resident.rs:221-235`). The largest binding is `STATE`.

**Evidence.**

```
.\evidence\run.ps1 budget nvidia -Rest --allocate
.\evidence\run.ps1 budget amd -Rest --allocate
```

From `evidence/runs/budget-nvidia.txt` and `budget-amd.txt`:

| Grid | Device memory | Host staging | Notes |
|---|---|---|---|
| 4096² | 196 MiB (12.27 B/cell, 3.07× the 4-byte cell) | 3 × 128 MiB = 384 MiB | `STATE` 128 MiB, `AWAKE` 32 MiB |
| 8192² | 785 MiB (12.27 B/cell) | 3 × 512 MiB = 1536 MiB | |
| 16384² (iGPU only) | 3138 MiB | 3 × 2048 MiB = 6144 MiB | allocated |

**Why it matters.** Memory decides the largest grid once finding 8 is fixed. On the iGPU, device memory and host memory are the same RAM, so 6 GiB of staging comes out of the grid's own pool.

**Recommendation.**

- **(a) Shrink the dump.** Index it by lane only (`cells + lane*4 + corner`, and `quads + lane` for `AWAKE`), so the spare region is `4*subgroup` words. Lanes that share a dump slot overwrite each other there, which is harmless because nothing reads the dump. Better still, guard those stores with a branch.
- **(b) Shrink staging.** Size the staging buffers to the largest per-frame transfer (the 2 MiB picture), and upload large buffers in chunks through one of them.

*Expected effect (computed from `Plan::sizes`, change not implemented).*

- At 4096²: `STATE` 128 → 64 MiB and `AWAKE` 32 → 16 MiB, so device memory falls from 196 to 116 MiB (12.27 → 7.3 B/cell, −40 %). Host staging falls from 384 MiB to under 10 MiB.
- At 8192²: device 785 → 465 MiB; host 1.5 GiB → under 10 MiB.

*Cost.* About half a day for (a), plus the reference functions. About 1 day for (b): today `write` refuses with `Error::Staged` when a buffer is larger than the staging buffer, so it needs chunking.

## 7. The chain of dependent dispatches sets a ~15 µs floor per frame

**What is wrong.** A two-tier frame is 10 stages that depend on each other, separated by full compute barriers (`src/resident.rs:1412-1447`). Several stages are a single workgroup: `tick`, `wipe`, `scan … totals` and `prepare`. With nothing active, the frame costs what its barriers cost.

**Evidence.** `evidence/probes/floor.rs` chains N one-workgroup passes, each behind a barrier, recorded the way Grit records them.

```
.\evidence\run.ps1 floor nvidia
.\evidence\run.ps1 floor amd
```

| Card | Run (file) | Card time per dependent pass |
|---|---|---|
| RTX 4080 | quiet (`floor-nvidia-b.txt`) | 1.29–1.44 µs (min) |
| RTX 4080 | contended (`floor-nvidia-a.txt`) | 1.13–1.20 µs |
| iGPU | `floor-amd-b.txt` | 1.01–1.96 µs |
| iGPU | `floor-amd-a.txt` | 0.84–1.03 µs |

`census` measures idle frames at 15.0–15.4 µs (4080) and 15.1–19.0 µs (iGPU) at 4096², and 15.7 µs (4080) at 8192². That is about 11 stages × 1.3 µs, and it stays flat as the grid grows: the floor is the dispatch chain, not the grid.

**Why it matters.** Between one thousand and one hundred thousand active quads, the work itself takes a few µs, so the number of stages dominates. The `census` row scene costs 18.9 µs for 4 102 quads, and 15.4 µs of that is the floor.

**Recommendation.** Fold the single-workgroup passes into their neighbours:

- Move the `tick` and `wipe` writes into the previous frame's `prepare`.
- Replace `scan fine` + `scan fine totals` with one single-pass scan that uses decoupled look-back.

*Expected effect (estimate from the measured ~1.3 µs per stage; merges not implemented).* Removing 4–5 stages takes 5–7 µs off a 15 µs idle frame, about 40 %.

*Cost.* About 2–3 days for the look-back scan, about 1 day for the plain pass fusions.

## 8. The two-level scan, not memory, caps the largest grid

**What is wrong.** `Plan::over` refuses grids with more than `sweep::cells(subgroup)³` quads (`src/compact.rs:91-108`).

- **Subgroup 32 (the 4080):** 16 777 216 quads, which is 8192² cells. That grid uses only 785 MiB of the card's 16 GiB heap (finding 6).
- **Subgroup 64 (the iGPU):** 512³ quads. That is why the iGPU can allocate 16384² (3.1 GiB of device memory) and the 4080 cannot.
- **Limits beyond the scan:** `maxMemoryAllocationSize` is 2 GiB on the iGPU (`budget-amd.txt`, "ceiling 2048 MiB"; `STATE` at 16384² is exactly 2 GiB), and `maxStorageBufferRange` is 4 GiB − 1 on both cards (`vulkaninfo`).

**Evidence.**

- `budget-nvidia.txt`: "16384x16384: Plan::over refuses (262144 65536): quads 67108864 > reach 16777216".
- `budget-amd.txt`: 16384² allocated, with its 15 pipelines built in 565 ms.
- Busiest scene measured: `census 8192`, busy, 8.4 M listed quads. It takes 1.285 ms on the 4080 (`census-nvidia-8192.txt`) and 29.5 ms on the iGPU (`census-amd-8192.txt`).

**Why it matters.** The stronger card has the smaller maximum grid. Its memory would allow 16384² even with today's doubled `STATE`.

**Recommendation.** Add a third scan level, or use the single-pass look-back scan from finding 7, which has no depth limit. Once `STATE` outgrows `maxStorageBufferRange`, split it across two bindings.

*Expected effect.* 4× the cells on the 4080 (16384²) with today's memory layout.

*Cost.* About 1 day for a third level (`Plan`, `arranged`, one more scan pass), about 2 days for split bindings.

## 9. Grit's timing tools hide or misattribute cost

**What is wrong, with evidence.**

- **Batch averages absorb other processes' GPU time.** `scale` and `loop_cost` divide the timestamp span of one 20-frame submission by 20. `census … batch` compares 1-frame and 20-frame submissions:
  - While other processes loaded the 4080 (`evidence/runs/census-batch-nvidia.txt`), the column scene measured 29.1–29.5 µs per frame from 1-frame submissions but 72.4–72.7 µs per frame from 20-frame submissions. The busy scene measured 322–328 µs against 1.030–1.034 ms.
  - Rerun with a quieter GPU (`census-batch-nvidia-final.txt`), the gap closes: column 31.6–32.6 µs (1 frame) vs 28.5–28.6 µs (20 frames). The iGPU, which was unloaded during the first run, showed no gap at all.
  - So the inflated 20-frame numbers are other contexts' GPU time counted inside Grit's timestamp span. Command: `.\evidence\run.ps1 census nvidia -Rest 4096,batch`.
- **Stage stamps are taken at TOP_OF_PIPE.** `run_times` writes each stage's start stamp with `TOP_OF_PIPE` (`src/resident.rs:1373-1385`), and the frame's first stamp also uses it (`src/resident.rs:1328-1337`). A TOP_OF_PIPE stamp does not wait for earlier work to finish. On the 4080, the `stages` table is mostly values on exact 1.024 µs, 448 ns and 2.048 µs steps (`evidence/runs/stages-nvidia-*.txt`), which is the per-dispatch floor of finding 7.
- **The README and `pour`'s "ms a frame" leave out two of the three submissions** (finding 4). In `frameloop-nvidia-b.txt`, `spent` is 163 µs while the wall time is 528 µs. The README's figure of 0.80 ms at 719 quads was measured with a window and was *not reproduced* here.
- **No validation layer is installed.** `vulkaninfo` lists six instance layers (AMD switchable graphics, EOS overlay, NV optimus, NV present, Steam fossilize, Steam overlay) and no `VK_LAYER_KHRONOS_validation`. So the test suite runs without validation, and barrier or synchronisation errors go unreported.
- **No pipeline cache.** Pipelines are created with `vk::PipelineCache::null()` (`src/resident.rs:447-450`). On the 4080, building a resident with 16 pipelines took 127.6 ms cold and 6.4–15 ms warm, when the driver's own cache hits (`frameloop-nvidia-a.txt`, `-b.txt`).

**Recommendation.**

| Change | Cost |
|---|---|
| Time from many 1-frame submissions and report the minimum, as the probes here do | an hour per tool |
| Write stage stamps at `COMPUTE_SHADER` or `BOTTOM_OF_PIPE`, after each stage's barrier | an hour |
| Show the wall time per frame in the `pour` title | minutes |
| Install the Vulkan SDK and run `tests/` once with `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` and synchronisation validation | an hour |
| Add a `VkPipelineCache` saved to disk | half a day; about −100 ms at cold start |

*Expected effect.* Per-stage numbers that hold up on a shared GPU. Frame time does not change.

## 10. Checked and found fine: registers, occupancy, the strided lane layout

**Evidence.** `evidence/probes/regs.rs` compiles the 16 `pour` passes and reads the compiler's statistics through `VK_KHR_pipeline_executable_properties`.

```
.\evidence\run.ps1 regs nvidia
.\evidence\run.ps1 regs amd
```

- **Registers.** `regs-nvidia.txt`: 16–37 registers per pass (settle 35, mark 37), with no stack and no shared memory. `regs-amd.txt`: 3–40 VGPRs (settle 40), 12–28 SGPRs, and `scratchMemUsageInBytes=0`. These counts do not limit occupancy on either card.
- **Workgroup size.** Each workgroup is one subgroup, so the hardware's limit on resident workgroups caps occupancy. But a streaming copy runs about as fast with 32 threads per workgroup as with 256: 640 vs 639 GB/s on the 4080 and 65 vs 61 GB/s on the iGPU (`pattern-*-c.txt`; `pattern-nvidia-b.txt` gives 695 vs 667 GB/s).
- **Lane layout.** Grit's layout, 8 consecutive words per lane, reaches 651 GB/s against 640 GB/s for a coalesced layout on the 4080 (664 vs 695 in `pattern-nvidia-b.txt`), and 66 vs 65 GB/s on the iGPU.
- **Bandwidth ceilings.** The 4080's measured copy rate of 639–695 GB/s is close to its 716.8 GB/s data-sheet peak; the data-sheet figure was not measured here. The iGPU's best measured rate is 61–66 GB/s.

**Recommendation.** Nothing to change for registers or layout; spend the effort on findings 1–3.

One related idea, *not verified*: the iGPU supports subgroup 32 (`minSubgroupSize = 32` in `vulkaninfo`, through `VK_EXT_subgroup_size_control`). Requesting it would halve the block size on that card and so halve finding 1's amplification there. Cost: about 1 day across host-side pipeline creation and `Plan`.

## Compared with the hardware

- **Busy frame against a bandwidth floor (4080).** A busy 4096² frame takes 347–349 µs for 2.1 M active quads (`census-nvidia-c.txt`, `-final.txt`), of which `settle` is 258 µs. A floor model of 36 bytes per active quad (16 B read and 16 B written of `STATE`, plus 4 B of `DENSE`), at the measured 640–695 GB/s, gives 109–118 µs. Grit is about 3× off that floor today and 1.7–2.4× off with findings 2 and 3 applied (180.1–262.9 µs in `fixes-nvidia-c.txt` and `-final.txt`). The floor model is an estimate.
- **iGPU against 4080.** The busy frame takes 7.40 ms on the iGPU, 21× the 4080's time, while measured bandwidth differs only about 10× (66 vs 640–695 GB/s). `settle` alone takes 5.35 ms there. Beyond finding 2, the cause was not isolated (*not verified*).
- **Scaling with grid size and activity.** Idle frames stay flat as the grid grows: 15–17 µs on the 4080 from 2048² to 8192², and 16–28 µs on the iGPU (`census-*-2048.txt`, `census-*-c.txt`, `census-*-8192.txt`). Busy frames grow linearly with active quads on the 4080:

| Grid | Active quads | Frame time |
|---|---:|---:|
| 2048² | 0.53 M | 101.7 µs |
| 4096² | 2.1 M | 347 µs |
| 8192² | 8.4 M | 1.285 ms |

## Test suite

`cargo test --release` in the clone (on the default device, the 4080, since `Host::open` prefers the discrete GPU), from `evidence/runs/cargo-test-release.txt`:

- 46 test binaries: 327 passed, 0 failed, 2 ignored.
- 376 s in the final run.
- No validation layer, because none is installed (finding 9).

An earlier run, stopped partway while three suites shared the 4080, is not counted.
