# grit

`grit` is a sparse-grid cellular simulation on Vulkan compute, built on the `simdr` SPIR-V emitter. Each frame steps only the quads its work list names, and the list is rebuilt on the device by a two-level scan and gather. The crate is a library with four binaries: `scale` and `loop_cost` measure it, `pour` and `spike` open a window on it.
