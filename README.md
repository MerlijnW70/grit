# grit

`grit` is a sparse-grid cellular simulation on Vulkan compute, built on the `simdr` SPIR-V emitter. Each frame steps only the quads its work list names, and the list is rebuilt on the device by a two-level scan and gather. The crate is a library with five binaries: `scale`, `loop_cost` and `stages` measure it, `pour` and `spike` open a window on it.

## What a frame costs

The premise is that the bill follows activity, not extent. `scale` pours sand into worlds of growing size while the awake set stays the same size, and reports the time the card spent:

| cells | card time |
|---|---|
| 262144 | 11.513µs |
| 1048576 | 17.262µs |
| 4194304 | 18.833µs |
| 16777216 | 19.041µs |

The world grows by 64 times and the frame does not. What is left over is the work list: a larger world is scanned by more stages, and a stage costs a barrier whether or not there is anything in it.

## What a frame costs to look at

`loop_cost` steps a dense world and reads it back. Reading a window of the world costs 93.006µs at 262144 cells and 525.656µs at 16777216. Reading all of it costs 3.048ms at 16777216 cells, which is the bus rather than the card: the same copy on the same link does not go faster whatever the kernel does.

So a viewer should read a window, which `pour` does, and a measurement should read nothing, which `scale` does.
