# Grit audit: correctness and GPU synchronization

Grit at `8419acf`, simdr at `9dec3b7`. Hardware: NVIDIA GeForce RTX 4080 (driver 610.88, subgroup 32) and AMD Radeon(TM) Graphics iGPU (driver 2.0.353, subgroup 64), Windows 11, Vulkan 1.4.

## Verdict

Grit's GPU plumbing holds up well. Both full suites pass on both cards: 327 passed and 2 ignored on each, with my probes left out. The world matches bit for bit across the two devices and their two subgroup widths. The Khronos validation layer, with synchronization validation on, reports no errors and no hazards across 15 test binaries. The frame-ordering fixes in the recent history hold. The engine's central claim does not hold yet. That claim is that stepping only the listed quads gives the same world as stepping every quad, and two bugs in the rule pipeline break it.

1. A quad whose only change is the water "held" bit wakes nobody. Water beside air then freezes for good.
2. The brush wakes the quads it paints one frame after it paints them. Every fresh paint stroke therefore diverges from the whole-grid result, and that is the normal case in `pour`.

The suite cannot see either bug. Its only listed-against-whole check (`tests/live.rs`) pours from frame 0, when everything is still awake, and never produces a change to the held bit alone. A third, API-level hazard also exists: indirect dispatch arguments written by a `Groups::Fixed` pass are read stale on both cards, and the validation layer does not catch it. Each of the two engine bugs has a fix of a few lines, and I verified the fixes on both cards. With them, the listed world matches the whole grid in all 18 configurations I ran and is bit-identical across NVIDIA (subgroup 32) and AMD (subgroup 64).

## Findings

| Rank | Title | Severity | One-line evidence |
|---|---|---|---|
| 1 | A change to the held bit alone is not counted, so neighbours are never woken and water freezes | High: wrong world | `audit_corner`: DIVERGED at frame 160, cell (2044,1015), listed 0x11 vs whole 0x0, on both cards. With the patch: AGREED |
| 2 | The brush wakes what it paints one frame late, so the listed world diverges whenever painting starts after frame 1 or the brush moves | High: wrong world | `audit_brush`: as shipped, DIVERGED at frame 10 (late start) and frame 140 (moving brush), fine and coarse tiers, both cards. With the brush moved after settle: AGREED |
| 3 | The barrier before an indirect read depends on the caller tagging the writer `Counting`; a `Fixed` writer gives stale arguments with no error | Medium: silent wrong result through the public API | `audit_validation`: `writer=Fixed(1) mark=[0]` against `writer=Counting(1) mark=[7]` on both cards; sync validation reports nothing |
| 4 | The suite does not reach the cases that break: one brush schedule from frame 0, no moving brush, a frames-in-flight path never compared with the whole grid, 19 tests with no assertion | High gain: would have caught findings 1 and 2 | `tests/live.rs:48-55,114`; assertion scan below; findings 1 and 2 pass the shipped suite on both cards |
| 5 | Host writes to near or direct buffers are accepted while a frame is in flight, giving a non-deterministic frame | Medium | `audit_host_write_lands_inside_a_frame_in_flight`: 8 runs, 8 different sums between the old-only and new-only values, every write accepted |
| 6 | No validation layer is installed. With the layer added: 0 errors, 0 sync hazards, 354 BestPractices warnings, and it misses finding 3 | Low: tooling | `vulkaninfo --summary` lists no `VK_LAYER_KHRONOS_validation`; `evidence/logs/validation-nvidia.txt` |
| 7 | `Resident::read` maps staging memory after a fence with no transfer-to-host barrier | Low: spec conformance, not observed | `src/resident.rs:925-950`, against the readback path at `:1482-1493` |

Checked with no defect found, with the evidence for each:

- Full suite: NVIDIA 327 passed, 2 ignored (`evidence/logs/suite-nvidia.txt`; its one failure is my own first `audit_world` probe, and its 328 passes include my `audit_timestamps`). AMD 327 passed, 2 ignored, in 18m46s (`evidence/logs/suite-amd.txt`).
- Determinism across devices and subgroup widths: every state hash in `audit_brush` and the patched `audit_world` is identical on NVIDIA (subgroup 32) and AMD (subgroup 64). For example, 2048x1024 with walls gives `1af85f3df154accb` on both (`fixed-nvidia.txt`, `fixed-amd.txt`).
- Fine against coarse tier: identical hashes in every configuration.
- Grid seams: the world wraps as a torus with no walls, across 64x64, 512x256 and 2048x1024 (one and several tiles). It agrees once findings 1 and 2 are patched.
- Stage timestamps: `audit_timestamps` puts 0.8% (NVIDIA, `audit-nvidia.txt`) and 0.0% (AMD, `timestamps-amd.txt`) of a heavy stage's time on the light stage after it, so `stage_times` attributes work correctly on both cards.
- Leftover data in the unwritten `HOT_COUNTS` tail: no divergence in 200 frames on either card (`audit_seed_depends_on_buffers_the_caller_zeroed`).
- The frame-start barrier `after_earlier` (`resident.rs:1569-1592`) covers compute, transfer and indirect access across frames in flight. The sync layer agrees.

---

## 1. A change to the held bit alone is not counted, so neighbours are never woken

**What is wrong.** In `live::listed` (the rule pass the engine uses), the inputs `a` and `b` are replaced by their `hold` versions before the change count is taken:

```
src/live.rs:536  let a = sand::hold::<LANES>(&mut lanes, a, c)?;
src/live.rs:537  let b = sand::hold::<LANES>(&mut lanes, b, d)?;
src/live.rs:548  for (before, after) in [(a, na), (b, nb), (c, nc), (d, nd)] {
```

The store at `:584-594` writes the held word, so the cell does change. But `count`, which `wake` uses as `holding` (`:602-606`), compares the held word with itself. A step that only sets `HELD` on water standing on water therefore wakes no neighbour. On the other phase, the overlapping quad sees held, idle water next to air. The rule (`sand::start`) would now start that water running, but the quad is asleep, so the water never moves. The whole-grid reference does move it.

**Evidence.** The probe is `evidence/probes/audit_corner.rs`, run as `tests/audit_corner.rs`. It uses a 2048x1024 room with walls and the brush schedule from `audit_world`. The brush already runs after settle there, which isolates this bug from finding 2. It stops at the first differing cell and dumps the surroundings. Output on the RTX 4080 (`evidence/logs/corner-nvidia.txt`; the AMD run is in `evidence/logs/audit-amd.txt`):

```
NVIDIA GeForce RTX 4080 sg=32 grid=2048x1024 blocks=2048 tiles=8 brush_last=true
DIVERGED frame=160 phase=0 cell=(2044,1015) listed=0x11 whole=0x0
quad 520190 (1022,507) flag before frame=0 on list used this frame=false list length=10008
```

The dumped state shows held water 0x11 at (2044,1015) with air at (2045,1015). The whole grid turns this into running water 0x39 one cell to the right. The listed engine leaves it in place because quad (1022,507) has flag 0. AMD (subgroup 64, 1024 blocks, 2 tiles) stops at the same frame and cell (`audit-amd.txt`: `DIVERGED frame=160 phase=0 cell=(2044,1015)`). With `evidence/fix-hold-wake.patch` applied in a second clone (`grit-fix`), the same probe prints `AGREED` on both cards (`fixed-nvidia.txt`, `fixed-amd.txt`). `audit_world` against the as-shipped engine (`evidence/logs/world-nvidia.txt`) fails the same way: 2048x1024 with walls, fine and coarse, both stop at frame 160 at (2044,1015). With the patch, all 12 configurations agree on both cards.

**Why it matters.** "The bill follows activity and not extent" is only acceptable if the answer is the answer of the rule. Here the sparse engine computes a different world, with water stuck as visible artefacts, whenever a hold-only change happens next to a sleeping quad. That happens by chance, so a short test will often miss it.

**Recommended change.** Wake on "loaded word differs from stored word" rather than "post-`hold` word differs". The patch keeps the loaded `[a, b, c, d]` and passes the new `touched` count to `wake`. The tally (`moved`) is left as it was. It is a 12-line change in one function, with no new pass and no new buffer. **Expected effect:** the listed world equals the whole grid in every configuration tested (18 runs on each card). **Cost:** slightly more quads listed, because hold-only changes now wake neighbours. Measured mean listed quads: 6933 → 6960 (+0.4%) at 512x256 with walls (`world-nvidia.txt` against `fixed-nvidia.txt`), and 596 → 604 in the `audit_brush` FromFrameZero run with the brush after settle (`audit-nvidia.txt` against `fixed-nvidia.txt`). Add `audit_corner` and `audit_world` to the suite as the regression test, at about 40 s on the 4080 and 60 s on the iGPU.

## 2. The brush wakes what it paints one frame late

**What is wrong.** `live::arranged` (`src/live.rs:225-253`) orders a frame as: `[tick, decay, wipe]`, then `[brush]`, then `[settle over the list]`, then `[mark … gather]`. `settle` consumes the list built at the end of the previous frame. The brush's `wake` (`sand.rs:682-712`) marks the painted quads, but they are only listed for the next frame. A freshly painted cell in a quad that was asleep is therefore not settled in the frame it was painted. The whole grid settles it immediately. `tests/live.rs` cannot see this because `seed` marks every quad awake for frames 0 and 1, and its brush pours from frame 0 at one fixed spot (`tests/live.rs:48-55`).

**Evidence.** The probe is `evidence/probes/audit_brush.rs`, on a 512x256 room (the `tests/live.rs` room), for 1200 frames. It runs three schedules: the original one from frame 0; the same pour starting at frame 10; and a brush moving 3 cells a frame. Each runs on the fine and coarse tiers, first as shipped and then with the brush stage moved after settle. RTX 4080 output (`evidence/logs/audit-nvidia.txt`):

```
as shipped: coarse=false FromFrameZero -> None
as shipped: coarse=false FromFrameTen -> Some(10)
as shipped: coarse=false Moving -> Some(140)
as shipped: coarse=true FromFrameZero -> None
as shipped: coarse=true FromFrameTen -> Some(10)
as shipped: coarse=true Moving -> Some(140)
... coarse=false FromFrameTen brush_last=true: AGREED for 1200 frames, mean listed 580 of 32768
... coarse=false Moving brush_last=true: AGREED for 1200 frames, mean listed 3162 of 32768
```

AMD gives the same frames, cells and hashes (`audit-amd.txt`). For example, `FromFrameTen … DIVERGED at frame 10, cell (185, 28), listed 0x2 whole 0x0, 10 cells differ`.

**Why it matters.** `pour` paints wherever the mouse is, every frame. Every stroke into a calm region gives a world the rule did not produce. The divergence then spreads through the timing of everything downstream (57 cells differ by frame 140 in the moving case).

**Recommended change.** Move the brush stage after `settle` and before `mark`. The swap is stages 1 and 2 in `arranged` and `keeping`, which is passes 4 and 5 in the flat list. The probe does exactly this (`wired.swap(4, 5); stages.swap(1, 2)`). The painted quads are then woken before the list for the next settle is built. The rule's semantics become "paint at the end of the frame", and the whole-grid reference in the tests must use the same order (`tick, settle, brush`). **Expected effect:** listed equals whole for every schedule. Measured: AGREED for all 6 combinations on both cards, and for all 12 `audit_world` configurations once combined with finding 1. **Cost:** no extra pass, barrier or buffer. Mean listed quads 594 → 596 in FromFrameZero (`audit-nvidia.txt`). I did not measure frame time.

## 3. Indirect arguments written by a `Fixed` pass are read stale

**What is wrong.** In `run_times` (`src/resident.rs:1420-1446`), the barrier between stages adds `DRAW_INDIRECT` / `INDIRECT_COMMAND_READ` only when the stage just finished contains a `Groups::Counting` pass. A stage of `Groups::Fixed` passes that writes a buffer the next stage reads through `Groups::From` therefore gets a compute-to-compute barrier only. The indirect read is not ordered after the write. The engine's own loop tags its `prepare` passes `Counting` (`live.rs:83,110,188,250`), so it is not affected. Nothing in the type or at runtime stops a caller from getting this wrong.

**Evidence.** The probe is `evidence/probes/audit_validation.rs`. Pass 1 writes `[1,1,1]` into binding 0. Pass 2 is dispatched `From { binding: 0 }` and writes 7 to binding 1. The two tests differ only in the writer's tag:

```
NVIDIA GeForce RTX 4080 writer=Counting(1) mark=[7]
NVIDIA GeForce RTX 4080 writer=Fixed(1) mark=[0]
AMD Radeon(TM) Graphics writer=Counting(1) mark=[7]
AMD Radeon(TM) Graphics writer=Fixed(1) mark=[0]
```

The `Fixed` case dispatched zero workgroups, from the stale `[0,0,0]`, on both cards. Under the Khronos layer with `validate_sync = true`, this run produced no error or hazard (`evidence/logs/validation-nvidia.txt` contains both `writer=` lines and no `SYNC-HAZARD`).

**Why it matters.** A top-tier engine does not let a tag that only concerns barriers decide whether a dispatch runs. Here the failure is silent, the same on both vendors, and invisible to the validation layer.

**Recommended change.** Always include `DRAW_INDIRECT` in the destination stages and `INDIRECT_COMMAND_READ` in the destination access of the stage barrier when any later stage of the frame uses `Groups::From`. This is decided when the frame is recorded. Alternatively, drop the `Counting` variant and always include them. **Expected effect:** `audit_indirect_written_by_a_fixed_pass` returns `[7]`. **Cost:** a wider barrier on stages that do not write arguments. `tests/floor.rs` prices barriers on this repo but is `#[ignore]`d, and I did not measure the delta. Treat the cost as unverified and measure it with `floor.rs` before choosing between the two variants.

## 4. The suite does not reach the cases that break

**What is missing.**

- The only listed-against-whole-grid check is `tests/live.rs`. It uses one brush schedule that starts at frame 0 at one spot, while `seed` keeps everything awake (`tests/live.rs:48-55`). It covers one room and three sizes, and only 400 frames at the size with several tiles. Findings 1 and 2 both pass it on both cards.
- No test compares the frames-in-flight path (`submit` × 2) or batched frames (`staged_frames`) with the whole grid. `tests/overlap.rs` checks tickets and readbacks, not the world.
- A scan of every `#[test]` body in `tests/` for `assert`, `panic!` or `is_err` finds 19 tests with none. Three of them are false positives (`live.rs`, whose helper asserts). That leaves 16 tests that cannot fail on a wrong answer, including `hunt.rs::whether_a_barrier_is_what_is_missing`, seven in `wander.rs` (for example `a_block_named_twice_in_one_list`), three in `taught.rs` and two in `descending.rs`.
- `descending.rs` needs `--features deep`, and `floor.rs` and one `wander.rs` test are `#[ignore]`. These are the 2 ignored tests on each card, so `cargo test --release` never runs them.

**Evidence.** The suite logs in `evidence/logs/` (327 passed on both cards with findings 1 and 2 present). For the scan, I ran a Python pass over `tests/*.rs` and `tests/*/mod.rs` that matched `#[test] fn` bodies with no `assert|panic!|unwrap_err|expect_err|.is_err()`. It prints 19, and the script is under "Commands", item 6.

**Recommended change.** Move `audit_world.rs`, `audit_brush.rs` and `audit_corner.rs` into the suite. They cover moving and late brushes, seams, three sizes, both tiers and walled and open rooms. Add one run of `audit_world`'s comparison driven through `staged_frames(…, 20)` and through two tickets in flight. Turn each of the 16 assertion-free tests into a claim or move it to a bench target. **Expected effect:** findings 1 and 2 would have failed CI. **Cost:** about 40 s on the 4080 and 60 s on the iGPU for the three files (`audit_world` took 38.4 s and 60.0 s; `audit_brush` 8.7 s and 33.6 s).

## 5. Host writes during a frame in flight make that frame non-deterministic

**What is wrong.** `Resident::write` and `write_at` check `live()`, not `idle()`, when the binding is host-visible (`src/resident.rs:576-577, 647-648`). They then memcpy straight into memory that a submitted frame may be reading. `tests/dispatch.rs::a_write_into_a_near_buffer_does_not_wait_for_the_frame_in_flight` pins this as intended, using a binding that the kernel does not touch. Nothing refuses the write when the in-flight frame does use that binding.

**Evidence.** The probe is `audit_host_write_lands_inside_a_frame_in_flight` (`evidence/probes/audit_contract.rs`). It runs 64 passes that sum binding 0 (1 Mi words of 1) into binding 1. The host writes 2s into binding 0 right after `submit`. RTX 4080 (`evidence/logs/contract-nvidia.txt`):

```
expected if the frame saw only the old data: 67108864, only the new: 134217728
(write accepted, sum) per run: [(true, 69392592), (true, 73832432), (true, 71526384), (true, 68022704), (true, 70025296), (true, 72086592), (true, 68982416), (true, 73581616)]
```

AMD gives sums between 129017696 and 134217728 (`audit-amd.txt`). Every write was accepted, and no two NVIDIA runs agree. The exact sums change on every run; what reproduces is that they vary and fall between the two bounds.

**Why it matters.** The engine refuses staged writes during a flight (`StillInFlight`) but quietly races direct ones. The same call means different things depending on which memory heap the allocator picked (`resident_near` window).

**Recommended change.** Refuse a write to a host-visible binding while any flight is outstanding, as `once` already does. Alternatively, record per flight which bindings the frame reads (the `Recorded` stages know them) and refuse only those. **Expected effect:** the probe gets `StillInFlight`, and frames become deterministic by construction. **Cost:** the first option is about 3 lines and changes the `dispatch.rs` test that pins the current behaviour. The second is about 30 lines.

## 6. No validation layer installed; with one, 0 errors and a blind spot

**Evidence.** `vulkaninfo --summary` on this machine lists six instance layers (AMD switchable, EOS, NV optimus, NV present, two Steam layers) and no `VK_LAYER_KHRONOS_validation`. For this audit I used the Khronos release `nightly` asset `Validation_Layers_Windows_64.zip`. It is kept at `vvl\vvl.zip` in the job directory, sha256 `0693DCA1739EE70FBD8A1ACA1C59DBEF4EB2DE95785BC49DB4FCD062E50199D4`, and reports itself as layer 1.4.366. I enabled it through `VK_ADD_LAYER_PATH`, `VK_INSTANCE_LAYERS` and `evidence/vvl/vk_layer_settings.txt`, which turns on sync, best practices and logging to stdout. I ran 15 test binaries on the RTX 4080: live, overlap, dispatch, coarse, vast, direct, staging, sand, compact, hot, counting, sleeping, wedged, audit_brush and audit_validation. All passed, with 0 `Validation Error`, 0 sync hazards and 354 × `BestPractices-vkBindBufferMemory-small-dedicated-allocation` (`evidence/logs/validation-nvidia.txt`). Without the settings file, the layer loaded (I confirmed this with `VK_LOADER_DEBUG=layer` but did not keep that log) and printed nothing on Windows.

**Why it matters.** Validation passes cleanly, which is a good result, but nothing in Grit asks for it. The one real synchronization hazard I found (finding 3) is invisible to it. Clean validation is necessary but not sufficient.

**Recommended change.** Add a `GRIT_VALIDATE=1` path in `Host::matching`. It would enable the layer and a `VK_EXT_debug_utils` messenger that counts errors, and the test harness would assert the count is zero. **Expected effect:** regressions in API usage fail tests. **Cost:** about 40 lines in `host.rs`. The layer is not installed here, so CI would also need it.

## 7. `read()` maps staging after a fence with no transfer-to-host barrier

**What is wrong.** `Resident::read` (`src/resident.rs:925-950`) records only `cmd_copy_buffer` into staging inside `once`, waits on the fence, and maps. The frame readback path puts a `TRANSFER_WRITE → HOST_READ` barrier after its copy (`:1482-1493`). `read` has none. My reading of the Vulkan synchronization chapter is that a fence alone does not make device writes visible to the host; the spec requires a memory dependency whose destination is the host. **Not verified:** host-coherent memory on both cards makes this invisible, and the validation layer does not check host visibility. I have no run that shows wrong data.

**Recommended change.** Append the same `TRANSFER → HOST` barrier inside `read`'s `once` closure. **Expected effect:** conformance on drivers that need it. **Cost:** 10 lines and one barrier per `read`. I did not measure the time.

---

## Commands

All commands are PowerShell. My clones are in the job directory. Set:

```
$J = "<job directory>"
```

- `$J\grit` is Grit 8419acf with `evidence\probes\*.rs` copied into `tests\`.
- `$J\grit-fix` is the same with `evidence\fix-hold-wake.patch` applied.
- `$J\simdr` is simdr 9dec3b7.

To recreate them from scratch:

```
git clone F:\grit $J\grit; git -C $J\grit checkout 8419acf; git clone F:\simdr $J\simdr; git -C $J\simdr checkout 9dec3b7
Copy-Item $J\evidence\probes\*.rs $J\grit\tests\
git clone $J\grit $J\grit-fix; git -C $J\grit-fix checkout 8419acf; Copy-Item $J\evidence\probes\*.rs $J\grit-fix\tests\; git -C $J\grit-fix apply $J\evidence\fix-hold-wake.patch
```

`GRIT_DEVICE` picks the card. It persists in the PowerShell session, so set it on every run.

1. **Finding 1 (held bit).** Expect `DIVERGED frame=160 phase=0 cell=(2044,1015) listed=0x11 whole=0x0`; the test then panics on purpose. With the patch, expect `AGREED`:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --test audit_corner -- --nocapture
   cd $J\grit-fix; $env:CARGO_TARGET_DIR="$J\target-fix"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --test audit_corner --test audit_world -- --nocapture --test-threads=1
   ```
   Use `$env:GRIT_DEVICE="AMD"` for the iGPU, which gives the same frame and cell.
2. **Finding 2 (brush latency).** Expect `as shipped: … FromFrameTen -> Some(10)`, `Moving -> Some(140)`, and every `brush_last=true` line `AGREED`:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --test audit_brush -- --nocapture --test-threads=1
   ```
3. **Finding 3 (indirect after a `Fixed` writer).** Expect `writer=Fixed(1) mark=[0]` and `writer=Counting(1) mark=[7]`:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --test audit_validation -- --nocapture --test-threads=1
   ```
4. **Finding 5 (host write in flight).** Sums vary from run to run between 67108864 and 134217728:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --test audit_contract -- --nocapture --test-threads=1
   ```
5. **Finding 6 (validation run).** The second command counts the messages:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; $env:VK_ADD_LAYER_PATH="$J\vvl\x"; $env:VK_INSTANCE_LAYERS="VK_LAYER_KHRONOS_validation"; $env:VK_LAYER_SETTINGS_PATH="$J\evidence\vvl"; $env:AUDIT_FRAMES="200"; cargo test --release --test live --test overlap --test dispatch --test coarse --test vast --test direct --test staging --test sand --test compact --test hot --test counting --test sleeping --test wedged --test audit_brush --test audit_validation -- --nocapture --test-threads=1 2>&1 | % { "$_" } | Out-File -Width 500 $J\v.txt
   Select-String -Path $J\v.txt -Pattern "Validation (Error|Warning|Performance Warning)" | Group-Object { ($_.Line -split '\]')[0] } | Select Count,Name
   vulkaninfo --summary
   ```
   The layer is `$J\vvl\x`, extracted from `$J\vvl\vvl.zip`. Afterwards, remove `VK_INSTANCE_LAYERS` from the session.
6. **Finding 4 (assertion scan).** Run from `$J\grit\tests` in Git Bash; it prints 19 and the names:
   ```
   python -I -c "import re,glob;n=[];[n.append(f+'::'+m.group(1)) for f in glob.glob('*.rs')+glob.glob('*/mod.rs') if not f.startswith('audit_') for s in [open(f,encoding='utf8').read()] for m in re.finditer(r'#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn (\w+)\(\)\s*\{',s) for b in [s[m.end():s.find('\n}\n',m.end())]] if not re.search(r'assert|panic!|unwrap_err|expect_err|\.is_err\(\)',b)];print(len(n));print('\n'.join(n))"
   ```
7. **Full suites.** The NVIDIA run took 12m04s and the AMD run 18m46s:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="NVIDIA"; cargo test --release --no-fail-fast -- --skip audit_
   $env:GRIT_DEVICE="AMD"; cargo test --release --no-fail-fast -- --skip audit_
   ```
8. **Checked areas.** `audit_timestamps` (stage attribution) and the patched `audit_world` on both cards, for the cross-device hashes:
   ```
   cd $J\grit; $env:CARGO_TARGET_DIR="$J\target"; $env:GRIT_DEVICE="AMD"; cargo test --release --test audit_timestamps -- --nocapture
   cd $J\grit-fix; $env:CARGO_TARGET_DIR="$J\target-fix"; $env:GRIT_DEVICE="AMD"; cargo test --release --test audit_world -- --nocapture
   ```

F:\grit and F:\simdr were only cloned from, never written. At the end of the audit, F:\simdr's `git status` was clean. F:\grit was at 86f6dc5 with one untracked `docs/build/` directory that I did not create.
