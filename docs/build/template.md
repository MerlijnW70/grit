# Job: <title>

<!--
Template for a build job. Copy to docs/build/<name>.md, replace every <...>,
delete this comment, and commit the spec before its escrow is posted: the
escrow pins the spec's hash, so the spec cannot change afterwards.
-->

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit <grit commit>) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit <simdr commit>). <One or two sentences on the part of Grit this job touches, with the files that hold it.> <The audit finding this job answers: docs/audit/reports/<lens>.md, finding <n>.>

## Goal

<One paragraph: the change, and the observable result it has once it is in. Name the measurement or the behaviour that moves, not the implementation.>

## Requirements

- <Each requirement is a check the judge can run or read, one per line.>
- Public behaviour outside the goal is unchanged: every test that passes at the pinned commit still passes, and no test is deleted, ignored or weakened. A test that has to change says why in its commit message.
- New behaviour comes with a test in tests/ that fails at the pinned commit and passes with the patch.
- Code follows the repository: English throughout, the lints in Cargo.toml (`unwrap_used` and `panic` are denied), no new dependency unless this spec allows it.
- Every number written into a comment, README or commit message is produced by a command named in the delivery notes.
- <Scope limit: the files or modules the patch may touch.>

## How to work

- Clone both repositories next to each other into your job directory, so Grit's `simdr = { path = "../simdr" }` resolves: `git clone F:\grit grit` and `git clone F:\simdr simdr`, then check out the commits above. Never change F:\grit or F:\simdr themselves.
- Build and test on Windows with the machine's GPUs (an NVIDIA RTX 4080 and an AMD Radeon iGPU, both with Vulkan 1.4). `GRIT_DEVICE=<substring>` picks a device by name, case-insensitive. A test that finds no device prints `SKIPPED` and passes, so a green run with `SKIPPED` lines has not tested anything on that device.
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
```

- `gate-4080.log` contains no `SKIPPED` line and no failed test. `--nocapture` is required: `SKIPPED` goes to stderr, which libtest swallows for a passing test. Count with `Select-String -CaseSensitive -Pattern 'SKIPPED' gate-4080.log`; without `-CaseSensitive` it also matches test names that contain "skipped".

Measured at 86f6dc5 on a fresh clone on this machine (rustc 1.98.1): cold `cargo build --release` 77.8 s; `cargo fmt --all --check` 0.9 s; `cargo clippy --release --all-targets` with `-D warnings` 18 s; `cargo test --release --no-run` 474 s; 329 tests listed by default, 346 with `--features deep`. At c945b06, `cargo test --release --no-fail-fast -- --nocapture` with `GRIT_DEVICE=4080` took 378.3 s wall with the test binaries already built: 327 passed, 0 failed, 2 ignored across 46 binaries, no `SKIPPED` line. The AMD iGPU has not been timed here.
- <If the job touches device code: the same test command with `GRIT_DEVICE = "Radeon"` into `gate-radeon.log`, for the tests: <list>.>
- <If the job touches the descent research: `cargo test --release --features deep --test descending`.>

## Deliberate break

A green suite proves nothing unless it can turn red. The delivery includes `break.patch`: a small change on top of your patches that undoes the essential part of the work (a removed barrier, a reverted bound, an off-by-one put back) and makes at least one named test fail. Name the test and paste its failure.

The judge also writes a break of their own choosing against the delivered code. If no test fails under the judge's break, the job is refused.

## Deliver

In your job directory:

- `patches/`: the work as `git format-patch <grit commit>..HEAD -o patches`, one commit per logical step, each with an English message that says what changed and why. They must apply with `git am` on a fresh clone at the pinned commit.
- `break.patch`: as above, made with `git diff` on top of the applied patches.
- `notes.md`: what was done and what was not; the gate's commands with their exit codes and wall time; the failing test under `break.patch` with its output; every number the patches state and the command that produced it.
- `gate-4080.log` (and any other gate log).

Announce the sha256 of a tar of these files to the judge.

## Acceptance

The judge, on a fresh clone at the pinned commit:

1. `git am patches/*.patch` applies cleanly.
2. The gate passes, with no `SKIPPED` on the RTX 4080.
3. `break.patch` applied on top makes the named test fail, and the judge's own break makes a test fail.
4. Every requirement above is met, and every number in the patches reproduces from the command `notes.md` gives for it.
5. Nothing in F:\grit or F:\simdr changed.

The job is refused if any step fails. The judge records why.
