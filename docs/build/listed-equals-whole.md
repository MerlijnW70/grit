# Job: The listed world equals the whole grid

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit bbc3dfb) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit 9dec3b7). Each frame steps only the quads on the work list. The list is rebuilt on the device from the quads the rule wakes. The listed step is `live::settle` (`src/live.rs`), and the frame schedule is `live::arranged` and `live::keeping`. Grit's central claim is that stepping only the listed quads gives the same world as stepping every quad. This job answers findings 1, 2 and 4 of `docs/audit/reports/correctness.md`, which break that claim; `docs/audit/reports/review.md` reproduced 1 and 2 on both cards.

## Goal

For every brush schedule and grid tested, the listed engine produces bit for bit the same world as the whole-grid step, on the RTX 4080 and on the AMD iGPU. The suite contains tests that fail at bbc3dfb and would have caught both bugs.

## Requirements

- **Held bit.** A step that changes a cell only by `sand::hold` wakes the neighbours. The change count that `wake` receives compares the word loaded from `STATE` with the word stored, not the post-`hold` word with the result (finding 1). The tally of moved, wet and dry cells keeps its present meaning.
- **Brush order.** A quad the brush paints is settled in the frame it is painted, or the brush runs after `settle` and before `mark` so that the painted quads are on the next list (finding 2). Pick one, apply it to both `arranged` and `keeping`, and give the whole-grid reference the same order. Say in a commit message which you picked and what "a frame" now means for the brush.
- **Tests.** New tests in `tests/` compare the listed world with the whole grid after every frame and fail at the first differing cell, naming the frame, the cell and both values. Together they cover:
  - a brush that starts after frame 1, and a brush that moves every frame;
  - a hold-only change next to a sleeping quad (the report's 2048x1024 room with walls diverges at frame 160 at bbc3dfb);
  - the fine and the coarse tier;
  - a run driven through two frames in flight (`Resident::submit` twice before reading), compared with the whole grid.
- **Each fix has its own failing test.** With only the held-bit fix applied, at least one new test fails. With only the brush fix applied, at least one new test fails. Name them.
- The new tests take no more than 90 s together on the RTX 4080, measured with `Measure-Command` and stated in `notes.md` for both cards.
- The cost is measured: the mean listed quads per frame before and after the patch, in one named configuration, with the command.
- Public behaviour outside the goal is unchanged: every test that passes at bbc3dfb still passes, and no test is deleted, ignored or weakened. `tests/live.rs` may change only for the brush order, and the commit says why.
- Code follows the repository: English throughout, the lints in Cargo.toml (`unwrap_used` and `panic` are denied in `src`), no new dependency.
- Every number written into a comment, README or commit message is produced by a command named in `notes.md`.
- Scope: `src/live.rs`, `src/sand.rs` where the brush needs it, and `tests/`. Nothing else.

## How to work

- Clone both repositories next to each other into your job directory, so Grit's `simdr = { path = "../simdr" }` resolves: `git clone F:\grit grit` and `git clone F:\simdr simdr`, then check out the commits above. Never change F:\grit or F:\simdr themselves.
- Build and test on Windows with the machine's GPUs (an NVIDIA RTX 4080 and an AMD Radeon iGPU, both with Vulkan 1.4). `GRIT_DEVICE=<substring>` picks a device by name, case-insensitive. A test that finds no device prints `SKIPPED` and passes, so a green run with `SKIPPED` lines has not tested anything on that device.
- The audit's probes are not available to you; the report describes them well enough to write your own. `tests/live.rs` holds the existing comparison (`against_the_whole_grid`) to start from.
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
$env:GRIT_DEVICE = "Radeon"; cmd /c "cargo test --release --no-fail-fast --test live --test <each new test file> -- --nocapture > gate-radeon.log 2>&1"
```

- Neither log contains a `SKIPPED` line or a failed test. `--nocapture` is required: `SKIPPED` goes to stderr, which libtest swallows for a passing test. Count with `Select-String -CaseSensitive -Pattern 'SKIPPED'`; without `-CaseSensitive` it also matches test names that contain "skipped".

## Deliberate break

The delivery includes `break.patch`: on top of your patches, it puts back the post-`hold` comparison in the change count and leaves the brush fix in place. Name the test that fails under it and paste the failure.

The judge also writes a break of their own choosing against the delivered code. If no test fails under the judge's break, the job is refused.

## Deliver

In your job directory:

- `patches/`: the work as `git format-patch bbc3dfb..HEAD -o patches`, one commit per logical step (held bit, brush order, tests), each with an English message that says what changed and why. They must apply with `git am` on a fresh clone at bbc3dfb.
- `break.patch`: as above, made with `git diff` on top of the applied patches.
- `notes.md`: what was done and what was not; the gate's commands with their exit codes and wall time; which new test fails with only one fix, for each fix; the failing test under `break.patch` with its output; the new tests' wall time on both cards; the listed-quads cost; every number the patches state and the command that produced it.
- `gate-4080.log` and `gate-radeon.log`.

Announce the sha256 of a tar of these files to the judge.

## Acceptance

The judge, on a fresh clone at bbc3dfb:

1. `git am patches/*.patch` applies cleanly.
2. The gate passes, with no `SKIPPED` on either card.
3. At bbc3dfb with only the new tests added, the new tests fail; with each fix alone, the test `notes.md` names fails.
4. `break.patch` applied on top makes the named test fail, and the judge's own break makes a test fail.
5. Every requirement above is met, and every number in the patches reproduces from the command `notes.md` gives for it.
6. Nothing in F:\grit or F:\simdr changed.

The job is refused if any step fails. The judge records why.
