---
id: DR-0017
title: An address range is not an allocation
status: prose-only
---

## The Measurement

Both devices in this machine report `sparseBinding`, `sparseResidencyBuffer` and
`sparseResidencyAliased`, and on both the queue family this crate picks — family 0 on each — carries
`SPARSE_BINDING`. The address range they will hold is not the memory they have:

```text
                          sparse address space   memory   one descriptor reaches
  RTX 4080                    1 048 576 MiB      16 048 MiB      4 095 MiB
  Radeon(TM) Graphics       134 205 440 MiB      32 384 MiB      4 095 MiB
```

`Host::resident_vast` makes one binding out of an address range with nothing behind it, and
`Resident::bind` puts memory behind a window of it. Measured on both parts: a buffer of 4092 MiB of
range, one page of 64 KiB bound two gigabytes in, and a kernel that stores a word there and reads it
back gets what it wrote — 32 lanes on the one, 64 on the other. The page size is
`vkGetBufferMemoryRequirements`'s alignment and is 64 KiB on both, which is a measurement rather than
a number to assume.

A load from a page nothing is bound to answers **nought**, on both.

## The Decision

`Host::resident_vast(sizes, modules, vast)` builds every binding as `Host::resident` would except the
one named, which is created `SPARSE_BINDING | SPARSE_RESIDENCY` and starts empty.
`Resident::bind`, `unbind`, `page` and `bound` are the rest of it, and the device is now created with
the two features asked for — it asked for none before, and a device created without them refuses the
buffer rather than the flag.

**A chunk is the caller's to choose.** One allocation and one `vkQueueBindSparse`, waited on, per
call to `bind`: a device holds a few thousand allocations and a page is 64 KiB, so binding a page at a
time would run out of allocations at a few hundred megabytes. A voxel world binds a megabyte because
that is what its chunks are; a model binds an expert.

The staging buffer is sized by the largest *ordinary* binding. It was sized by the largest of all of
them, and an address range of four gigabytes would have asked for four gigabytes of staging.

## The Rejected Route

A terabyte through one descriptor is rejected because it is not available: `maxStorageBufferRange` is
4095 MiB on both parts, a shader sees a binding through a descriptor, and sparse binding does not lift
that. It lifts what has to be *behind* a range and not how much of one a shader can address. Reaching
further would want `VK_KHR_buffer_device_address`, which is a different extension and is not here.

Binding a page at a time, so that a caller need not choose a chunk, is rejected on the allocation
count above.

## The Limit

**The device does not fetch what a shader touches.** There is no page fault here that reaches the
host: a read of an unbound range answers with nought and a write to one is discarded, and nothing
anywhere is told it happened. What this replaces is the bookkeeping — a table of slots, a compacted
arena, an index turned into a slot on every access — and not the loop that decides what to fetch. That
loop is still the caller's, and a shader that writes down what it could not read is still how it is
driven.

What a bind costs is NOT MEASURED. It is an allocation, a queue submission and a fence, and the fence
alone is 41.1–43.9 µs by `tests/floor.rs`; whether the rest is that again or ten times it has not been
timed, and a streaming loop wants to know.

Whether a range bound while a frame is in flight is safe is not answered here and is not allowed:
`bind` takes `&mut self` and every frame waits on its fence before returning, so the question does not
arise in this crate and would in one that overlapped them.

The unbound read answering nought is what both these parts do. `residencyNonResidentStrict` is what
promises it, and this does not read that property — so on a device that does not promise it, that
nought is undefined rather than nought.
