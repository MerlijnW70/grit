# Job: Grit audit: architecture against the state of the art

Grit (https://github.com/MerlijnW70/grit, local F:\grit at commit 8419acf) is a sparse-grid cellular simulation on Vulkan compute, built on the simdr SPIR-V emitter (local F:\simdr at commit 9dec3b7). A cell is one 32-bit word whose meaning belongs to the rule; each frame steps only the quads its work list names, and the list is rebuilt on the device by a two-level scan and gather. Its README states the measured numbers and how to run the demos (`cargo run --release --bin pour`) and the table tools (`scale`, `stages`, `loop_cost`); its tests live in tests/. The owner wants Grit to be a technically top-tier engine. This job is one lens of a three-lens audit; the other two lenses run in parallel.

## Lens

Compare Grit with the best GPU cellular and falling-sand simulations and sparse-grid engines you can find online (published papers, open-source engines, well-documented commercial ones), and judge its architecture: how general the rule model is (materials, interactions, multi-cell and multi-step rules, temperature or pressure fields), the public API and how a user writes a rule, determinism and replay, saving and loading worlds, very large or streamed worlds, portability across vendors and subgroup sizes, how the simdr emitter limits or enables it, and what a user of a top-tier engine would expect that Grit lacks. Cite every external source with its link.

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
