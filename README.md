# grit

`grit` is a sparse-grid cellular simulation on Vulkan compute, built on the `simdr` SPIR-V emitter.

A cell is one 32-bit word, and what those bits mean belongs to the rule rather than to the engine. Each frame steps only the quads its work list names, and that list is rebuilt on the device by a two-level scan and gather, so the bill follows activity and not extent.
