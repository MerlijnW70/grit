# grit

`grit` is a sparse-grid cellular simulation on Vulkan compute, built on the `simdr` SPIR-V emitter.

A cell is one 32-bit word, and what those bits mean belongs to the rule rather than to the engine. Each frame steps only the quads its work list names, and that list is rebuilt on the device by a two-level scan and gather, so the bill follows activity and not extent.

## See it run

```
cargo run --release --bin pour
```

![sand and water poured onto shelves in a 4096 by 4096 grid](demos/pour.png)

Left mouse pours sand, right mouse pours water. Sand piles into cones and water
lies flat, which is the rule's doing and not the engine's. `W`, `A`, `S` and `D`
move the viewport, shift moves it faster, `F` follows whatever is moving most, and
escape quits. The window title carries the numbers as it runs: frames a second,
milliseconds a frame, quads on the work list, quads in the grid, cells that moved,
and how many of them are dry and wet.

Measured on an RTX 4080 with a subgroup of 32, while that picture was being taken:
**719 quads on the work list out of the grid's 4 194 304**, at 0.80 ms a frame. The
grid is 4096 by 4096 and the window shows 1024 by 512 of it. Laying the room out
took 24.2 ms, and the sixteen compiled passes are 23 675 words holding 198 MB on
the card.

`spike`, `back` and `climb` also open a window. `scale`, `stages` and `loop_cost`
print tables and need none. `grit-mcp` speaks MCP on stdin and stdout.
