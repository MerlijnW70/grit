# Job: `decay` issues its loads before its stores

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit 3207c1d) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit 9dec3b7). Every frame, `live::decay` (`src/live.rs`, built by `decayed`) lowers the awake flag of the quads each lane owns. It returns the pass as SPIR-V words. This job answers finding 3 of `docs/audit/reports/performance.md`: `decayed` runs `load AWAKE[q]; store AWAKE[q]` eight times in a row, so the eight loads do not overlap. `docs/audit/reports/review.md` reproduced the gain on the AMD iGPU (column frame 1.127 ms to 0.395 ms) and on the RTX 4080 (busy frame −9 to −12 % over three runs).

## Goal

`decay` issues all its `AWAKE` loads before any of its stores. The world it produces is bit for bit unchanged, and the frame time it costs drops on both cards, measured.

## Requirements

- **Order.** In the words `decay` emits, every load from `AWAKE` comes before the first store to `AWAKE`, at every subgroup width `decay` accepts (4, 8, 16, 32, 64).
- **Same world.** The state after a run is bit-identical with and without the patch. Show it with a state hash over a named scene and frame count, on both cards.
- **Order test.** A test in `tests/` reads the words `live::decay` returns and fails if a store to `AWAKE` comes before a load from it. It runs without a device, so it never prints `SKIPPED`.
- **Measured gain.** The wall or card time of one frame, before and after the patch, in at least two scenes (one with activity in a column, one busy), on the RTX 4080 and on the AMD iGPU. Report the minimum and the median of at least three runs each, with the command. Use Grit's own `stages` or `scale` binaries, or a probe you add under `examples/` and deliver.
- **Survey, not change.** List every other pass in `src/` that stores to a binding before a later load from the same binding in one invocation, with file and line. Do not change them in this job.
- Public behaviour outside the goal is unchanged: every test that passes at 3207c1d still passes, and no test is deleted, ignored or weakened.
- Code follows the repository: English throughout, the lints in Cargo.toml, no new dependency.
- Every number written into a comment, README or commit message is produced by a command named in `notes.md`.
- Scope: `decayed` in `src/live.rs`, one new test file in `tests/`, and probes under `examples/`. Nothing else, not simdr.

## How to work

- Clone both repositories next to each other into your job directory, so Grit's `simdr = { path = "../simdr" }` resolves: `git clone F:\grit grit` and `git clone F:\simdr simdr`, then check out the commits above. Never change F:\grit or F:\simdr themselves.
- Build and test on Windows with the machine's GPUs (an NVIDIA RTX 4080 and an AMD Radeon iGPU, both with Vulkan 1.4). `GRIT_DEVICE=<substring>` picks a device by name, case-insensitive. A test that finds no device prints `SKIPPED` and passes, so a green run with `SKIPPED` lines has not tested anything on that device.
- Other GPU work can run on this machine at the same time. Time single-frame submissions, report minimum and median, and say what `nvidia-smi` showed for other processes during the timing.
- Run the gate below before delivering, and keep its output.

## Gate

Run in PowerShell from the Grit clone, in this order. Each step must exit 0.

```
cargo fmt --all --check
$env:RUSTFLAGS = "-D warnings"
cargo clippy --all-targets
cargo clippy --no-default-features --all-targets
Remove-Item Env:RUSTFLAGS
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --no-deps; Remove-Item Env:RUSTDOCFLAGS
$env:GRIT_DEVICE = "4080"; cmd /c "cargo test --release --no-fail-fast -- --nocapture > gate-4080.log 2>&1"
$env:GRIT_DEVICE = "Radeon"; cmd /c "cargo test --release --no-fail-fast --test live --test <your order test> -- --nocapture > gate-radeon.log 2>&1"
```

- Neither log contains a `SKIPPED` line or a failed test. `--nocapture` is required: `SKIPPED` goes to stderr, which libtest swallows for a passing test. Count with `Select-String -CaseSensitive -Pattern 'SKIPPED'`; without `-CaseSensitive` it also matches test names that contain "skipped".

## Deliberate break

The delivery includes `break.patch`: on top of your patches, it moves one store back ahead of the next load in `decayed`. Name the test that fails under it and paste the failure.

The judge also writes a break of their own choosing against the delivered code. If no test fails under the judge's break, the job is refused.

## Deliver

In your job directory:

- `patches/`: the work as `git format-patch 3207c1d..HEAD -o patches`, one commit per logical step, each with an English message that says what changed and why. They must apply with `git am` on a fresh clone at 3207c1d.
- `break.patch`: as above, made with `git diff` on top of the applied patches.
- `notes.md`: what was done and what was not; the gate's commands with their exit codes and wall time; the state hashes before and after; the timing table with its commands; the survey of other passes; the failing test under `break.patch` with its output; every number the patches state and the command that produced it.
- `gate-4080.log`, `gate-radeon.log`, and every probe under `examples/` that `notes.md` cites.

Announce the sha256 of a tar of these files to the judge.

## Acceptance

The judge, on a fresh clone at 3207c1d:

1. `git am patches/*.patch` applies cleanly.
2. The gate passes, with no `SKIPPED` on either card.
3. The state hashes before and after the patch match on both cards.
4. The judge reruns the timing for one scene on each card; the direction of the change holds and the after-time is within the range `notes.md` reports.
5. `break.patch` applied on top makes the named test fail, and the judge's own break makes a test fail.
6. Every requirement above is met, and every number in the patches reproduces from the command `notes.md` gives for it.
7. Nothing in F:\grit or F:\simdr changed.

The job is refused if any step fails. The judge records why.
