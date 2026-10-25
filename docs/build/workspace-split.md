# Job: Split Grit into a core, a demos and an MCP crate

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit b3238f4) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit 9dec3b7). Today one package, `grit`, holds the engine (`src/host.rs`, `resident.rs`, `layout.rs`, `compact.rs`, `sweep.rs`, `live.rs`, `sand.rs`, `gate.rs`, `rule.rs`, `board.rs`, `netlist.rs`, `marking.rs`, `machine.rs`), the MCP server (`src/serve.rs`, `src/wire.rs`, `src/bin/grit-mcp.rs`) and the demo and timing binaries (`src/bin/pour.rs`, `spike.rs`, `back.rs`, `climb.rs`, `loop_cost.rs`, `scale.rs`, `stages.rs`), with `minifb` behind the `window` feature. This job answers finding 8 of `docs/audit/reports/architecture.md`: the crate also holds unrelated subsystems. A running `grit-mcp.exe` also blocks every release build of the package, because cargo relinks it.

## Goal

The repository becomes a Cargo workspace of three crates: `grit-core` (the engine, no window and no MCP code), `grit-demos` (the binaries that show or time the engine) and `grit-mcp` (the MCP server). The engine builds and tests on its own, and building or testing `grit-core` or `grit-demos` never touches `grit-mcp.exe`.

## Requirements

- **Workspace.** The root `Cargo.toml` is a virtual manifest with exactly the members `crates/grit-core`, `crates/grit-demos` and `crates/grit-mcp`. Shared settings live once at the root: `[workspace.package]` (edition 2024, rust-version, license), `[workspace.lints]` with today's lints (`unsafe_op_in_unsafe_fn`, `unwrap_used`, `panic`, `indexing_slicing`), which every member uses with `lints.workspace = true`, and today's `[profile.release]`. One `Cargo.lock` at the root.
- **grit-core** (`crates/grit-core`, library name `grit_core`): the thirteen engine modules above and today's `pub use` of `Error`, `Host`, `Groups`, `Resident`, `Run` and `Ticket`. Its dependencies are simdr (path to F:\simdr's clone next to the Grit clone) and `ash`, nothing else; no `minifb`, no `serve` or `wire`, no dependency on the other two crates. It owns the `deep` feature and every test in `tests/` except `serve.rs` and `heldout.rs`, with the `lessons/` and `lifting/` folders.
- **grit-demos** (`crates/grit-demos`): the binaries `pour`, `spike`, `back`, `climb`, `loop_cost`, `scale` and `stages`, under the same names, depending on `grit-core` by path. It owns the `window` feature (default on, `minifb` as today, including the unix `x11` feature); `pour`, `spike`, `back` and `climb` require it as today.
- **grit-mcp** (`crates/grit-mcp`): `serve.rs`, `wire.rs` (with the JSON parsing and the tools they hold) and the `grit-mcp` binary, depending on `grit-core` by path. It owns `tests/serve.rs` and `tests/heldout.rs`.
- **History.** Files are moved with `git mv` in a commit of their own, so `git log --follow` reaches each file's history and patches against b3238f4 still apply with `git am -3`.
- **Same tests.** `cargo test --workspace --release -- --list` lists the same test names as `cargo test --release -- --list` at b3238f4 (the binary prefix may change), and the default and `--features deep` runs pass as before. No test is deleted, ignored or weakened; a changed test only renames `grit::` to `grit_core::` or adjusts a path.
- **Guard test.** A test in `crates/grit-core/tests/` reads `crates/grit-core/Cargo.toml` and `src/lib.rs` through `CARGO_MANIFEST_DIR` and fails if grit-core depends on anything but simdr and `ash`, names `minifb`, `grit-demos` or `grit-mcp`, or declares a `serve` or `wire` module. It runs without a device, so it never prints `SKIPPED`.
- **Independence, shown.** With `crates/grit-mcp` removed from a scratch copy of the workspace members, `cargo build --release -p grit-core` and `cargo test --release -p grit-core --no-run` succeed. With a built `target\release\grit-mcp.exe` running, `cargo build --release -p grit-core -p grit-demos` and `cargo test --release -p grit-core` succeed and leave that file's modification time unchanged. `cargo tree -p grit-core -e normal` lists no `minifb`. `notes.md` gives each command and its output.
- **Paths outside the crates.** `.github/workflows/ci.yml` runs the same checks against the workspace (the simdr checkout must still resolve from each member), and `README.md` and `docs/build/template.md` name the new paths and commands. The audit reports in `docs/audit/` stay as they are.
- Code follows the repository: English throughout, no new dependency, no behaviour change in any module.
- Every number written into a comment, README or commit message is produced by a command named in `notes.md`.
- Scope: the manifests, the moves, the `grit::` to `grit_core::` renames, `src/lib.rs` split into the three crates, the guard test, `ci.yml`, `README.md` and `docs/build/template.md`. Nothing in simdr, `front/` or `docs/audit/`.

## How to work

- Clone both repositories next to each other into your job directory, so Grit's simdr path resolves: `git clone F:\grit grit` and `git clone F:\simdr simdr`, then check out the commits above. Never change F:\grit or F:\simdr themselves.
- Build and test on Windows with the machine's GPUs (an NVIDIA RTX 4080 and an AMD Radeon iGPU, both with Vulkan 1.4). `GRIT_DEVICE=<substring>` picks a device by name, case-insensitive. A test that finds no device prints `SKIPPED` and passes, so a green run with `SKIPPED` lines has not tested anything on that device.
- Other GPU work can run on this machine at the same time; this job measures no timings, so that only slows the suite.
- Run the gate below before delivering, and keep its output.

## Gate

Run in PowerShell from the Grit clone, in this order. Each step must exit 0.

```
cargo fmt --all --check
$env:RUSTFLAGS = "-D warnings"
cargo clippy --workspace --all-targets
cargo clippy --workspace --no-default-features --all-targets
Remove-Item Env:RUSTFLAGS
$env:RUSTDOCFLAGS = "-D warnings"; cargo doc --workspace --no-deps; Remove-Item Env:RUSTDOCFLAGS
$env:GRIT_DEVICE = "4080"; cmd /c "cargo test --workspace --release --no-fail-fast -- --nocapture > gate-4080.log 2>&1"
cmd /c "cargo test --release -p grit-core --features deep --test descending -- --nocapture > gate-deep.log 2>&1"
```

- `gate-4080.log` contains no `SKIPPED` line and no failed test. Count with `Select-String -CaseSensitive -Pattern 'SKIPPED' gate-4080.log`; without `-CaseSensitive` it also matches test names that contain "skipped".
- The passed and ignored totals in `gate-4080.log` equal those of the same command at b3238f4 (`cargo test --release --no-fail-fast -- --nocapture`), run on the same machine and given in `notes.md`.

## Deliberate break

A green suite proves nothing unless it can turn red. The delivery includes `break.patch`: a small change on top of your patches that adds `minifb` to grit-core's dependencies and makes the guard test fail. Name the test and paste its failure.

The judge also writes a break of their own choosing against the delivered code, for example a `pub mod serve;` put back into grit-core or a test dropped from a crate's list. If no test or listed check catches the judge's break, the job is refused.

## Deliver

In your job directory:

- `patches/`: the work as `git format-patch b3238f4..HEAD -o patches`, with the `git mv` commit separate from the edits, each commit with an English message that says what changed and why. They must apply with `git am` on a fresh clone at b3238f4.
- `break.patch`: as above, made with `git diff` on top of the applied patches.
- `notes.md`: what was done and what was not; the gate's commands with their exit codes and wall time; the test lists before and after with the command that compares them; the independence commands with their output; the failing test under `break.patch` with its output; every number the patches state and the command that produced it.
- `gate-4080.log` and `gate-deep.log`.

Announce the sha256 of a tar of these files to the judge.

## Acceptance

The judge, on a fresh clone at b3238f4:

1. `git am patches/*.patch` applies cleanly, and `git log --follow` on a moved file reaches a commit before b3238f4.
2. The gate passes, with no `SKIPPED` on the RTX 4080 and the same totals as b3238f4.
3. `break.patch` applied on top makes the guard test fail, and the judge's own break is caught.
4. The independence checks reproduce, and every requirement above is met.
5. Nothing in F:\grit or F:\simdr changed.

The job is refused if any step fails. The judge records why.
