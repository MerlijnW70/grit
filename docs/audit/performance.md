# Job: Grit audit: performance

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit 8419acf) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit 9dec3b7). A cell is one 32-bit word whose meaning belongs to the rule; each frame steps only the quads its work list names, and the list is rebuilt on the device by a two-level scan and gather. Its README states the measured numbers and how to run the demos (`cargo run --release --bin pour`) and the table tools (`scale`, `stages`, `loop_cost`); its tests live in tests/. The owner wants Grit to be a technically top-tier engine. This job is one lens of a three-lens audit; the other two lenses run in parallel.

## Lens

Measure where Grit's time and memory go and how it scales: per-pass GPU time with timestamp queries, the cost of the scan, gather, rule step and readbacks, dispatch and submission overhead, memory traffic against the cards' bandwidth, occupancy and register pressure of the emitted SPIR-V, how frame time scales with active quads versus grid extent (the README claims the bill follows activity), the cost on the AMD iGPU next to the RTX 4080, and what limits the largest grid and the busiest scene. Compare the numbers with what the hardware could do, and name the changes that would buy the most time or memory.

## How to work

- Clone both repositories next to each other into your job directory, so Grit's `simdr = { path = "../simdr" }` resolves: `git clone F:\grit grit` and `git clone F:\simdr simdr`, then check out the commits above. Never change F:\grit or F:\simdr themselves.
- Run on Windows with the machine's GPUs (an NVIDIA RTX 4080 and an AMD Radeon iGPU, both with Vulkan 1.4). `cargo test --release` in your Grit clone runs the suite; the Vulkan validation layers may be enabled through the usual environment variables if the SDK is installed, and their absence is itself worth reporting.
- You may write probe code, benchmarks and tests in your clone to prove a point. Keep each one, with the command that runs it, so the judge can reproduce it.

## Deliver

One Markdown report, `report.md`, in your job directory, and a zip-free folder `evidence/` next to it with every probe, benchmark and test you cite. Announce the report's sha256 to the judge.

The report has, in order: a one-paragraph verdict on where Grit stands for this lens; a ranked table of findings (rank, title, severity or gain, one-line evidence); then one section per finding with what is wrong or missing, the exact evidence (command and output, file and line, measurement with the hardware), why it matters for a top-tier engine, and the concrete change you recommend with its expected effect and cost. Findings are ranked by how much they move Grit toward top tier. Facts you could not verify are marked as such.

## Acceptance

1. Every finding cites evidence the judge can reproduce from the report alone on this machine; the judge reruns the top three and any two others, and the report is refused if a cited result does not reproduce.
2. Nothing in F:\grit or F:\simdr changed.
3. The report contains no recommendation without a cost and an expected effect, and no claim of a number that no command in the report produced.
