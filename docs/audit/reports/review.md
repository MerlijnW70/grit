# Review of the three audit reports

Each report's top findings were rerun from fresh clones (grit 8419acf, simdr 9dec3b7) with the auditors' probes, on the RTX 4080 (subgroup 32) and the AMD iGPU (subgroup 64). The correctness copy here differs from the reviewed one only in a job directory path, now written as `<job directory>`.

## Correctness

| Finding | Rerun |
|---|---|
| 1 Held bit not counted | Reproduced on both cards: DIVERGED at frame 160, cell (2044,1015), listed 0x11, whole 0x0. With `fix-hold-wake.patch`, `audit_corner` and `audit_world` agree. |
| 2 Brush wakes one frame late | Reproduced: diverges at frame 10 (late start) and frame 140 (moving brush), fine and coarse tiers; agrees with the brush after settle. |
| 3 Indirect dispatch after a `Fixed` writer | Reproduced: `Fixed(1)` gives `mark=[0]`, `Counting(1)` gives `mark=[7]`. |

## Performance

| Finding | Rerun |
|---|---|
| 1 Row strips | Reproduced at 4096²: row 18.7 µs against column 31.0 µs on the 4080; 34.3 µs against 1.13 ms on the iGPU. |
| 2 `TALLY` atomics | Direction holds with identical state hashes, but the gain is noisy: three busy-frame runs on the 4080 gave −16 %, −36 % and −30 %, against the report's −38 to −42 %. On the iGPU spreading is 3.6× slower; leader lane −8.5 %. |
| 3 `decay` load order | Reproduced: iGPU column 1.127 ms to 0.395 ms; 4080 busy −9 to −12 % over three runs. |

## Architecture

| Finding | Rerun |
|---|---|
| 1 Sparse pipeline tied to sand | Confirmed in the code: `src/live.rs` calls `sand::` directly and `src` declares no public trait. |
| 2 Narrow rule model | Reproduced: a single grain falls 253 rows in 254 frames. |
| 3 Save and resume | Reproduced on both cards: resume from state plus frame word differs in 0 cells, frame off by one in 17. |
| 4 Reach depends on subgroup width | Reproduced: 16384² refused at subgroup 32, 4096² refused at subgroup 16. |
| 7 No random primitive | Partly wrong: `sweep::step` in `src/sweep.rs` already hashes the frame into a cell value, so a frame hash exists outside `src/bin/spike.rs`. No rule-facing random operation exists, so the recommendation stands. |
| 9 Bit-exact determinism | Reproduced: hash `d5326afd461cd507` on both cards and the CPU reference. |
