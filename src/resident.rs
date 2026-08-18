//! Buffers that outlive the frame, and every pass that runs over them.
//!
//! One descriptor set, shared by every pipeline: each pass sees every buffer, at the binding index
//! the `simdr` kernel was built against. That is the simplification the whole thing rests on — a
//! set per pass would be more general and would mean deciding, per pass, which buffers it may see,
//! and a simulation's answer to that is always *all of them*.
//!
//! # One submission a frame
//!
//! Every pass is recorded into one command buffer with a memory barrier between, and the whole
//! thing is submitted once and waited on once. Five kernels a frame is one fence wait, not five —
//! and a fence wait is where the round trip's hundred microseconds live.

use std::time::{Duration, Instant};

use ash::vk;

use crate::host::{Error, Host};

/// How many workgroups a pass runs, and who decides.
///
/// **The whole point of the second one.** A compaction's output is a list whose length the *device*
/// worked out, and dispatching over it means the workgroup count has to come from a buffer rather
/// than from a caller who would have to read it back first. Reading it back is a submit and a fence
/// — about a hundred microseconds by `decisions/DR-0008`'s measurement — to learn a number the
/// device already had.
#[derive(Debug, Clone, Copy)]
pub enum Groups {
    /// A count the host knows.
    ///
    /// **Nought means nought.** The pass is recorded and dispatched over no workgroups, which is a
    /// legal call that does nothing — and is how a caller holding several passes runs one and skips
    /// the others.
    Fixed(u32),
    /// Three words at `binding`, from `at`: the x, y and z counts, as Vulkan wants them.
    From {
        /// Which buffer holds them.
        binding: usize,
        /// Which word of it the three begin at.
        at: usize,
    },
}

/// A buffer and the memory under it.
struct Held {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    bytes: u64,
}

/// One run of words going somewhere, for [`Resident::write_many`].
#[derive(Debug, Clone, Copy)]
pub struct Run<'words> {
    /// Which binding.
    pub index: usize,
    /// How many words into it.
    pub at: usize,
    /// What to put there.
    pub words: &'words [u32],
}

/// A submission on the device, and what landing it owes the caller.
#[derive(Debug, Clone, Copy)]
struct Flight {
    /// Which ticket this is.
    ticket: u64,
    /// How many words to hand back, where the caller asked for some.
    out: Option<usize>,
    /// When it was submitted, so that a wait can say what the whole round trip cost.
    began: Instant,
}

/// A submission that has been started and not yet waited on.
///
/// Held by the caller and handed back to [`Resident::wait`]. It carries a number rather than a
/// borrow so that a caller may do anything at all in between — including writing into a buffer this
/// device is not reading — which is the whole reason the two halves are separate.
///
/// `#[must_use]` because dropping one does not cancel the submission. It leaves the device working
/// and the next [`Resident::submit`] refusing.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

impl Ticket {
    /// The number this ticket is, for a caller keeping its own record.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Everything a frame needs, allocated once.
pub struct Resident<'host> {
    host: &'host Host,
    /// Whether a write can go straight into a buffer, with no staging and no submission.
    direct: bool,
    buffers: Vec<Held>,
    staging: Held,
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    shaders: Vec<vk::ShaderModule>,
    pipelines: Vec<vk::Pipeline>,
    commands: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    /// The submission on the device, if there is one.
    ///
    /// `None` when nothing is in flight. A second submission while this is `Some` is refused rather
    /// than queued: there is one command buffer, and resetting it under a running submission is
    /// undefined behaviour rather than a slow path.
    flight: Option<Flight>,
    /// How many submissions this has made, which is where the next ticket's number comes from.
    tickets: u64,
}

impl Host {
    /// Allocate `sizes` buffers in words, and build one pipeline per module over them.
    ///
    /// Everything expensive happens here and none of it again: the allocations, the descriptor set,
    /// and a pipeline per pass. A frame after this is a recording and a submit.
    ///
    /// # Errors
    ///
    /// [`Error::NoMemory`] if nothing suits, otherwise whatever Vulkan said.
    pub fn resident<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, false)
    }

    /// The same, with buffers the host can write into **without a submission**.
    ///
    /// Asks for memory that is both device-local and host-visible — resizable BAR, where the
    /// platform has it. A write is then one copy straight into the buffer, with no staging buffer in
    /// system memory, no command buffer, no queue submission and no fence.
    ///
    /// Whether that is *faster* is a different question and belongs to the caller's measurements:
    /// a copy into device memory across the bus is not the same speed as a copy into system memory,
    /// and which wins depends on the platform. `hyperia`'s card tier measures both.
    ///
    /// # Errors
    ///
    /// [`Error::NoMemory`] where the device offers no such memory, which is the answer rather than a
    /// quiet fall back to staging — a caller asking for this is asking because it measured.
    pub fn resident_direct<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, true)
    }

    /// Allocate `sizes` buffers and build one pipeline per module over them.
    fn buffers_in<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        direct: bool,
    ) -> Result<Resident<'host>, Error> {
        let words = |count: usize| (count.max(1) * size_of::<u32>()) as u64;
        let largest = sizes.iter().copied().map(words).max().unwrap_or(4);

        // SAFETY: every handle created below is owned by the `Resident` and destroyed in its
        // `Drop`, which cannot run while a frame is in flight because every frame waits on its
        // fence before returning.
        unsafe {
            let living = if direct {
                vk::MemoryPropertyFlags::DEVICE_LOCAL
                    | vk::MemoryPropertyFlags::HOST_VISIBLE
                    | vk::MemoryPropertyFlags::HOST_COHERENT
            } else {
                vk::MemoryPropertyFlags::DEVICE_LOCAL
            };
            let mut buffers = Vec::with_capacity(sizes.len());
            for &size in sizes {
                buffers.push(self.allocate(
                    words(size),
                    vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::TRANSFER_SRC
                        | vk::BufferUsageFlags::TRANSFER_DST
                        // Any binding may end up holding a workgroup count. Declaring it on all of
                        // them costs nothing and saves the caller telling the allocator which.
                        | vk::BufferUsageFlags::INDIRECT_BUFFER,
                    living,
                )?);
            }

            let staging = self.allocate(
                largest,
                vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;

            let (layout, pool, set) = self.describe(&buffers)?;
            let (pipeline_layout, shaders, pipelines) = self.compile(layout, modules)?;
            let (commands, command) = self.record_into()?;
            let fence = self
                .device()
                .create_fence(&vk::FenceCreateInfo::default(), None)?;

            Ok(Resident {
                host: self,
                direct,
                buffers,
                staging,
                layout,
                pool,
                set,
                pipeline_layout,
                shaders,
                pipelines,
                commands,
                command,
                fence,
                flight: None,
                tickets: 0,
            })
        }
    }

    /// One buffer and its memory.
    ///
    /// # Safety
    ///
    /// The caller owns what comes back and must destroy it.
    unsafe fn allocate(
        &self,
        bytes: u64,
        usage: vk::BufferUsageFlags,
        wanted: vk::MemoryPropertyFlags,
    ) -> Result<Held, Error> {
        let create = vk::BufferCreateInfo::default()
            .size(bytes.max(4))
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: the caller's contract.
        let handle = unsafe { self.device().create_buffer(&create, None) }?;

        // SAFETY: `handle` was just created by this device.
        let requirements = unsafe { self.device().get_buffer_memory_requirements(handle) };
        let Some(kind) = self.suits(requirements, wanted) else {
            // SAFETY: nothing is bound to it.
            unsafe { self.device().destroy_buffer(handle, None) };
            return Err(Error::NoMemory { bytes });
        };

        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(kind);
        // SAFETY: the size and type come from the requirements just queried.
        let memory = unsafe { self.device().allocate_memory(&allocate, None) }?;
        // SAFETY: freshly allocated memory of a suitable type, nothing bound yet.
        unsafe { self.device().bind_buffer_memory(handle, memory, 0) }?;

        Ok(Held {
            handle,
            memory,
            bytes: bytes.max(4),
        })
    }

    /// The descriptor set every pipeline shares.
    ///
    /// # Safety
    ///
    /// As [`Host::allocate`].
    unsafe fn describe(
        &self,
        buffers: &[Held],
    ) -> Result<
        (
            vk::DescriptorSetLayout,
            vk::DescriptorPool,
            vk::DescriptorSet,
        ),
        Error,
    > {
        let count = u32::try_from(buffers.len()).unwrap_or(u32::MAX).max(1);

        let bindings: Vec<vk::DescriptorSetLayoutBinding<'_>> = (0..count)
            .map(|index| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(index)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE)
            })
            .collect();
        let create = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        // SAFETY: `create` borrows `bindings`, which outlives the call.
        let layout = unsafe { self.device().create_descriptor_set_layout(&create, None) }?;

        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(count)];
        let create = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes);
        // SAFETY: as above.
        let pool = unsafe { self.device().create_descriptor_pool(&create, None) }?;

        let layouts = [layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: the pool was sized for exactly this set.
        let sets = unsafe { self.device().allocate_descriptor_sets(&allocate) }?;
        let set = sets.first().copied().unwrap_or_default();

        // Point every binding at its buffer, once. Nothing rebinds after this.
        let infos: Vec<[vk::DescriptorBufferInfo; 1]> = buffers
            .iter()
            .map(|held| {
                [vk::DescriptorBufferInfo::default()
                    .buffer(held.handle)
                    .offset(0)
                    .range(held.bytes)]
            })
            .collect();
        let writes: Vec<vk::WriteDescriptorSet<'_>> = infos
            .iter()
            .enumerate()
            .map(|(index, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(u32::try_from(index).unwrap_or(0))
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(info)
            })
            .collect();
        // SAFETY: every write names this set and a buffer that outlives the call.
        unsafe { self.device().update_descriptor_sets(&writes, &[]) };

        Ok((layout, pool, set))
    }

    /// One shader module and one pipeline per pass.
    ///
    /// # Safety
    ///
    /// As [`Host::allocate`].
    unsafe fn compile(
        &self,
        layout: vk::DescriptorSetLayout,
        modules: &[&[u32]],
    ) -> Result<(vk::PipelineLayout, Vec<vk::ShaderModule>, Vec<vk::Pipeline>), Error> {
        let layouts = [layout];
        let create = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
        // SAFETY: `create` borrows `layouts`, which outlives the call.
        let pipeline_layout = unsafe { self.device().create_pipeline_layout(&create, None) }?;

        let mut shaders = Vec::with_capacity(modules.len());
        let mut pipelines = Vec::with_capacity(modules.len());
        let entry = c"main";

        for words in modules {
            let create = vk::ShaderModuleCreateInfo::default().code(words);
            // SAFETY: `words` is SPIR-V this crate emitted and outlives the call.
            let shader = unsafe { self.device().create_shader_module(&create, None) }?;
            shaders.push(shader);

            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(entry);
            let create = [vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(pipeline_layout)];
            // SAFETY: the stage names a module just created and a layout that outlives the call.
            let built = unsafe {
                self.device()
                    .create_compute_pipelines(vk::PipelineCache::null(), &create, None)
            };
            match built {
                Ok(made) => pipelines.extend(made),
                Err((made, code)) => {
                    pipelines.extend(made);
                    return Err(code.into());
                }
            }
        }

        Ok((pipeline_layout, shaders, pipelines))
    }

    /// The command pool and the one buffer every frame is recorded into.
    ///
    /// # Safety
    ///
    /// As [`Host::allocate`].
    unsafe fn record_into(&self) -> Result<(vk::CommandPool, vk::CommandBuffer), Error> {
        let create = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.family())
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: the family is the one the device was created with.
        let pool = unsafe { self.device().create_command_pool(&create, None) }?;

        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: the pool was just created by this device.
        let buffers = unsafe { self.device().allocate_command_buffers(&allocate) }?;
        let command = buffers.first().copied().unwrap_or_default();

        Ok((pool, command))
    }
}

impl Resident<'_> {
    /// How many buffers this holds.
    #[must_use]
    pub fn bindings(&self) -> usize {
        self.buffers.len()
    }

    /// How many passes a frame runs.
    #[must_use]
    pub fn passes(&self) -> usize {
        self.pipelines.len()
    }

    /// Put `words` into binding `index`, through the staging buffer.
    ///
    /// **Once, at the start.** Everything after that stays where it is.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchBinding`] or [`Error::Overrun`], otherwise whatever Vulkan said.
    pub fn write(&mut self, index: usize, words: &[u32]) -> Result<(), Error> {
        let held = self.at(index)?;
        let bytes = size_of_val(words) as u64;
        if bytes > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: words.len(),
                held: (held.bytes as usize) / size_of::<u32>(),
            });
        }
        if self.direct {
            // SAFETY: the buffer's memory is host-visible and coherent — `resident_direct` asked
            // for nothing less — and nothing else maps it for the length of this call.
            return unsafe { self.poke(held.memory, 0, words) };
        }
        let (handle, staging) = (held.handle, self.staging.memory);

        // SAFETY: the staging memory is host-visible and coherent, and nothing else maps it.
        unsafe {
            let mapped = self.host.device().map_memory(
                staging,
                0,
                bytes.max(4),
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast::<u32>(), words.len());
            self.host.device().unmap_memory(staging);
        }

        let region = [vk::BufferCopy::default().size(bytes.max(4))];
        // SAFETY: both buffers are live and the region is inside both.
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, self.staging.handle, handle, &region)
            })
        }
    }

    /// Copy `words` into `memory` at `offset` bytes, with no submission at all.
    ///
    /// # Safety
    ///
    /// `memory` must be host-visible and coherent, must hold at least `offset + words`, and must not
    /// be mapped by anything else for the length of the call.
    unsafe fn poke(
        &self,
        memory: vk::DeviceMemory,
        offset: u64,
        words: &[u32],
    ) -> Result<(), Error> {
        let bytes = (size_of_val(words) as u64).max(4);
        // SAFETY: the caller's contract.
        unsafe {
            let mapped =
                self.host
                    .device()
                    .map_memory(memory, offset, bytes, vk::MemoryMapFlags::empty())?;
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast::<u32>(), words.len());
            self.host.device().unmap_memory(memory);
        }
        Ok(())
    }

    /// Whether a write goes straight into a buffer, with no staging and no submission.
    ///
    /// True only for a [`Host::resident_direct`], and worth asking because it is the difference
    /// between a write costing one copy and a write costing a copy, a submission and a fence.
    #[must_use]
    pub const fn direct(&self) -> bool {
        self.direct
    }

    /// Put `words` into binding `index`, starting `at` words in.
    ///
    /// **What an interactive frame needs.** The device owns some of a parameter buffer — a frame
    /// counter it increments itself — and the host owns the rest, which is where a mouse goes.
    /// Writing from nought would put the host's idea of the frame number back on top of the
    /// device's every time.
    ///
    /// # Errors
    ///
    /// As [`Resident::write`].
    pub fn write_at(&mut self, index: usize, at: usize, words: &[u32]) -> Result<(), Error> {
        let held = self.at(index)?;
        let bytes = size_of_val(words) as u64;
        let offset = (at * size_of::<u32>()) as u64;
        if offset + bytes > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: at + words.len(),
                held: (held.bytes as usize) / size_of::<u32>(),
            });
        }
        if self.direct {
            // SAFETY: as in `write` — host-visible, coherent, and unmapped elsewhere. The offset is
            // inside the buffer, checked immediately above.
            return unsafe { self.poke(held.memory, offset, words) };
        }
        let (handle, staging) = (held.handle, self.staging.memory);

        // SAFETY: host-visible coherent memory, and nothing else maps it.
        unsafe {
            let mapped = self.host.device().map_memory(
                staging,
                0,
                bytes.max(4),
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast::<u32>(), words.len());
            self.host.device().unmap_memory(staging);
        }

        let region = [vk::BufferCopy::default()
            .dst_offset(offset)
            .size(bytes.max(4))];
        // SAFETY: both buffers are live and the region is inside both.
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, self.staging.handle, handle, &region);
            })
        }
    }

    /// Put several scattered runs into bindings, in **one submission**.
    ///
    /// # What it is worth, and where it is worth nothing
    ///
    /// [`Resident::write_at`] maps the staging buffer, copies, unmaps, submits a command buffer and
    /// **waits on its fence** — a full host-to-device round trip per call. This maps once, copies
    /// every run into staging back to back, records one region per run, and waits once.
    ///
    /// That saves one fixed cost per run, and measurement on an RTX 4080 puts it at about fifty
    /// microseconds. Which makes the answer entirely a matter of how large a run is:
    ///
    /// ```text
    ///   run size   one at a time      batched   gain
    ///     1 MiB       7.38 GiB/s   11.90 GiB/s   1.6x
    ///     4 MiB      10.31 GiB/s   12.13 GiB/s   1.2x
    ///    32 MiB      10.76 GiB/s   11.01 GiB/s   1.0x
    /// ```
    ///
    /// So this is for **many small runs** and there is no reason to reach for it above a few
    /// mebibytes, where the copy itself dominates and `write_at` in a loop is already at the bulk
    /// rate.
    ///
    /// It was added believing the gain was two and a half times at 32 MiB, from a twenty-four-sample
    /// run whose first write faulted in four gibibytes of staging and dominated the median. It did
    /// not reproduce. The table above is what ninety-six samples say, three times over.
    ///
    /// The destinations may be scattered across bindings and offsets; what is not scattered is the
    /// submission.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchBinding`] or [`Error::Overrun`] for any run, and [`Error::Overrun`] against
    /// binding nought where the runs together exceed the staging buffer — which is as large as the
    /// largest binding, so the ceiling is one binding's worth a call.
    pub fn write_many(&mut self, runs: &[Run<'_>]) -> Result<(), Error> {
        if runs.is_empty() {
            return Ok(());
        }

        // Every run checked before anything is copied, so a bad one cannot leave the staging buffer
        // half filled and the caller unsure how much of it landed.
        let mut total = 0_u64;
        for run in runs {
            let held = self.at(run.index)?;
            let bytes = size_of_val(run.words) as u64;
            let offset = (run.at * size_of::<u32>()) as u64;
            if offset + bytes > held.bytes {
                return Err(Error::Overrun {
                    index: run.index,
                    needed: run.at + run.words.len(),
                    held: (held.bytes as usize) / size_of::<u32>(),
                });
            }
            total += bytes;
        }
        if total > self.staging.bytes {
            return Err(Error::Overrun {
                index: 0,
                needed: (total as usize) / size_of::<u32>(),
                held: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }

        let staging = self.staging.memory;
        // SAFETY: host-visible coherent memory, nothing else maps it, and `total` is inside it by
        // the check above. Each run is copied to its own stretch, so the writes do not overlap.
        unsafe {
            let mapped =
                self.host
                    .device()
                    .map_memory(staging, 0, total.max(4), vk::MemoryMapFlags::empty())?;
            let mut along = 0_usize;
            for run in runs {
                let into = mapped.cast::<u32>().add(along);
                std::ptr::copy_nonoverlapping(run.words.as_ptr(), into, run.words.len());
                along += run.words.len();
            }
            self.host.device().unmap_memory(staging);
        }

        // `cmd_copy_buffer` takes one source and one destination, so regions are grouped by
        // destination. They all go into the same command buffer, which is the point of the exercise.
        let source = self.staging.handle;
        let mut grouped: Vec<(vk::Buffer, Vec<vk::BufferCopy>)> = Vec::new();
        let mut along = 0_u64;
        for run in runs {
            let handle = self.at(run.index)?.handle;
            let bytes = size_of_val(run.words) as u64;
            let region = vk::BufferCopy::default()
                .src_offset(along)
                .dst_offset((run.at * size_of::<u32>()) as u64)
                .size(bytes.max(4));
            along += bytes;
            match grouped.iter_mut().find(|held| held.0 == handle) {
                Some(held) => held.1.push(region),
                None => grouped.push((handle, vec![region])),
            }
        }

        // SAFETY: every buffer is live and every region was bounds-checked above.
        unsafe {
            self.once(|device, command| {
                for (handle, regions) in &grouped {
                    device.cmd_copy_buffer(command, source, *handle, regions);
                }
            })
        }
    }

    /// Read `count` words back from binding `index`.
    ///
    /// # Errors
    ///
    /// As [`Resident::write`].
    pub fn read(&mut self, index: usize, count: usize) -> Result<Vec<u32>, Error> {
        let held = self.at(index)?;
        let bytes = (count * size_of::<u32>()) as u64;
        if bytes > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: count,
                held: (held.bytes as usize) / size_of::<u32>(),
            });
        }
        let handle = held.handle;

        let region = [vk::BufferCopy::default().size(bytes.max(4))];
        // SAFETY: as in `write`.
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, handle, self.staging.handle, &region);
            })
        }?;
        // SAFETY: the copy above has completed — `once` waits on its fence.
        unsafe { self.mapped(count) }
    }

    /// Run every pass once, in order, with a barrier between.
    ///
    /// `groups` is one workgroup count per pass. What comes back is how long the submission took
    /// on the wall clock, fence wait included — which is what a frame actually costs.
    ///
    /// **Every pass is ordered against every other**, which is the safe reading of a flat list and
    /// is what a chain of passes wants. A caller whose passes are not all a chain says so with
    /// [`Resident::staged`] and pays for fewer barriers.
    ///
    /// # Errors
    ///
    /// [`Error::WrongPasses`] if the counts and the pipelines disagree, otherwise Vulkan's.
    pub fn frame(&mut self, groups: &[Groups]) -> Result<Duration, Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        self.staged(&stages)
    }

    /// Run every pass once, with a barrier only **between** stages and none inside one.
    ///
    /// The stages laid end to end are the passes, in order. Passes in one stage may run at the same
    /// time as each other and are ordered against every pass in every later stage.
    ///
    /// # What this is for
    ///
    /// A caller holding one pass per *thing* rather than one pass per *step*. `hyperia`'s card runs
    /// four steps over each of several experts, and the experts have nothing to say to one another:
    /// as a flat list that is one barrier between every pair, and as four stages it is three
    /// barriers however many experts there are.
    ///
    /// A barrier here is a full memory barrier — the same one [`Resident::frame`] emits — so
    /// dropping one is not a weaker guarantee within a stage. It is the statement that there was
    /// nothing to order.
    ///
    /// **A stage whose passes do depend on each other is a wrong answer, not a slow one**, and
    /// nothing here can check it. What is in a stage is the caller's claim.
    ///
    /// # Errors
    ///
    /// As [`Resident::frame`].
    pub fn staged(&mut self, stages: &[&[Groups]]) -> Result<Duration, Error> {
        let ticket = self.submit(stages)?;
        self.wait(ticket)
    }

    /// Start [`Resident::staged`] and **do not wait for it**.
    ///
    /// # What this is for
    ///
    /// A submission and its fence cost about eighty microseconds on this machine whatever is inside
    /// them, and [`Resident::frame`] spends that with the host doing nothing. This hands the ticket
    /// back instead, so the host can spend it: reading the next layer off a disk, writing an expert
    /// into device memory that this frame is not reading, or anything else that does not touch what
    /// is in flight.
    ///
    /// The device is not idle either way — this does not make the frame faster. It makes the wait
    /// available.
    ///
    /// # What must not happen in between
    ///
    /// **Nothing may write a buffer this submission reads, or read one it writes.** Nothing here can
    /// check that; the ticket is a promise the caller makes. A write to a buffer the frame is not
    /// touching is fine and is the point — and it becomes visible to the *next* submission, because
    /// `vkQueueSubmit` carries an implicit host-write barrier.
    ///
    /// # Errors
    ///
    /// [`Error::StillInFlight`] where a submission has not been waited on, otherwise as
    /// [`Resident::frame`].
    pub fn submit(&mut self, stages: &[&[Groups]]) -> Result<Ticket, Error> {
        self.start(stages, None, 1)
    }

    /// [`Resident::submit`], and copy `count` words of binding `index` out when the work lands.
    ///
    /// The words come back from [`Resident::wait_reading`], which is the only thing that may read
    /// them: before the fence they are whatever the last frame left.
    ///
    /// # Errors
    ///
    /// As [`Resident::submit`] and [`Resident::frame_reading`].
    pub fn submit_reading(
        &mut self,
        stages: &[&[Groups]],
        index: usize,
        count: usize,
    ) -> Result<Ticket, Error> {
        self.bounded(index, count)?;
        self.start(stages, Some((index, count)), 1)
    }

    /// Wait for a submission to land, and say how long it took from the submit.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchTicket`] for a ticket that is not the one outstanding, otherwise Vulkan's.
    pub fn wait(&mut self, ticket: Ticket) -> Result<Duration, Error> {
        self.land(ticket).map(|(spent, _)| spent)
    }

    /// The same, and hand back the words [`Resident::submit_reading`] asked for.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchTicket`] as [`Resident::wait`], and where the ticket was made by
    /// [`Resident::submit`] rather than [`Resident::submit_reading`] there is nothing to read and
    /// the words come back empty.
    pub fn wait_reading(&mut self, ticket: Ticket) -> Result<(Duration, Vec<u32>), Error> {
        let (spent, count) = self.land(ticket)?;
        let Some(count) = count else {
            return Ok((spent, Vec::new()));
        };
        // SAFETY: the submission has completed — `land` waited on its fence.
        let words = unsafe { self.mapped(count) }?;
        Ok((spent, words))
    }

    /// The same, and copy `count` words of binding `index` back inside the same submission.
    ///
    /// **One fence for the whole frame.** Reading afterwards with [`Resident::read`] would be a
    /// second submit and a second wait, which on this machine is another hundred microseconds for
    /// nothing.
    ///
    /// # Errors
    ///
    /// As [`Resident::frame`].
    pub fn frame_reading(
        &mut self,
        groups: &[Groups],
        index: usize,
        count: usize,
    ) -> Result<(Duration, Vec<u32>), Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        self.staged_reading(&stages, index, count)
    }

    /// [`Resident::staged`], and copy `count` words of binding `index` back inside the same
    /// submission.
    ///
    /// # Errors
    ///
    /// As [`Resident::frame_reading`].
    pub fn staged_reading(
        &mut self,
        stages: &[&[Groups]],
        index: usize,
        count: usize,
    ) -> Result<(Duration, Vec<u32>), Error> {
        let ticket = self.submit_reading(stages, index, count)?;
        self.wait_reading(ticket)
    }

    /// Refuse a read of more words than the binding holds, before anything is recorded.
    fn bounded(&self, index: usize, count: usize) -> Result<(), Error> {
        let held = self.at(index)?;
        if (count * size_of::<u32>()) as u64 > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: count,
                held: (held.bytes as usize) / size_of::<u32>(),
            });
        }
        Ok(())
    }

    /// The buffer at `index`, or the refusal naming both numbers.
    fn at(&self, index: usize) -> Result<&Held, Error> {
        self.buffers.get(index).ok_or(Error::NoSuchBinding {
            index,
            held: self.buffers.len(),
        })
    }

    /// Record every pass and submit, optionally copying a buffer out at the end.
    /// Record and submit, and hand back the ticket.
    ///
    /// **The one place a command buffer is recorded.** Everything that runs a frame comes through
    /// here, so the refusal below is the whole of what stops one submission resetting another's
    /// command buffer.
    fn start(
        &mut self,
        stages: &[&[Groups]],
        out: Option<(usize, usize)>,
        times: u32,
    ) -> Result<Ticket, Error> {
        if self.flight.is_some() {
            return Err(Error::StillInFlight);
        }
        let copy = match out {
            Some((index, count)) => {
                let held = self.at(index)?;
                Some((held.handle, (count * size_of::<u32>()) as u64))
            }
            None => None,
        };

        let began = Instant::now();
        self.run_times(stages, copy, times)?;
        self.tickets = self.tickets.wrapping_add(1);
        self.flight = Some(Flight {
            ticket: self.tickets,
            out: out.map(|(_, count)| count),
            began,
        });
        Ok(Ticket(self.tickets))
    }

    /// Wait for the outstanding submission, and say how long it took and what it owes.
    ///
    /// The duration is from the submit and not from here, so it is the round trip whether the
    /// caller waited immediately or went away and did something first.
    fn land(&mut self, ticket: Ticket) -> Result<(Duration, Option<usize>), Error> {
        let Some(held) = self.flight else {
            return Err(Error::NoSuchTicket {
                ticket: ticket.get(),
                held: None,
            });
        };
        if held.ticket != ticket.get() {
            return Err(Error::NoSuchTicket {
                ticket: ticket.get(),
                held: Some(held.ticket),
            });
        }

        // SAFETY: the fence belongs to the submission this is waiting for, and nothing resets the
        // command buffer until `flight` is cleared below.
        unsafe {
            self.host
                .device()
                .wait_for_fences(&[self.fence], true, u64::MAX)?;
        }
        self.flight = None;
        Ok((held.began.elapsed(), held.out))
    }

    /// Record `times` frames back to back into one command buffer, submit once, and wait once.
    ///
    /// # What this is for, and what a single frame cannot tell you
    ///
    /// [`Resident::frame`] times a submit and a fence around the work, and on this machine that
    /// round trip is about a hundred microseconds whatever is inside it — `decisions/DR-0008` in
    /// `simdr` names that as the unit of cost and it is not an estimate. A frame that does twenty
    /// microseconds of work and a frame that does eighty both come back at "about a hundred and
    /// something", and a change that halves the work is invisible.
    ///
    /// That is not a flaw in the loop — a viewer submits once a frame and pays it once a frame — but
    /// it is useless for asking whether a pass got cheaper. Twenty frames in one submission pay the
    /// round trip once and the work twenty times, so the difference between them **is** the work.
    ///
    /// The barrier between the last pass of one frame and the first of the next is the same one that
    /// sits between passes, for the same reason.
    ///
    /// # Errors
    ///
    /// As [`Resident::frame`].
    pub fn frames(&mut self, groups: &[Groups], times: u32) -> Result<Duration, Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        let ticket = self.start(&stages, None, times.max(1))?;
        self.wait(ticket).map(|spent| spent / times.max(1))
    }

    /// The recording, `times` frames over.
    fn run_times(
        &mut self,
        stages: &[&[Groups]],
        out: Option<(vk::Buffer, u64)>,
        times: u32,
    ) -> Result<(), Error> {
        let given: usize = stages.iter().map(|one| one.len()).sum();
        if given != self.pipelines.len() {
            return Err(Error::WrongPasses {
                given,
                held: self.pipelines.len(),
            });
        }

        let device = self.host.device();

        // SAFETY: nothing is in flight — `start` is the only caller and it refuses while something
        // is — so resetting the command buffer here cannot touch a running submission.
        unsafe {
            device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(self.command, &begin)?;

            device.cmd_bind_descriptor_sets(
                self.command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.set],
                &[],
            );

            // A pass told to run no workgroups does nothing and writes nothing, so recording it —
            // a pipeline bind and the barrier that would follow it — buys an ordering guarantee
            // over an empty set. Skipping it is the same frame with less in the command buffer, and
            // on this machine a card holding sixty-four passes to run two of them spent about
            // 2.3 µs a pass on exactly that. `Groups::From` is never skipped: its count is read on
            // the device and is not knowable here.
            //
            // The stages that have nothing running are dropped whole, so the barriers go between
            // the stages that do — a stage that is entirely skipped orders nothing either side of
            // it.
            let mut pass = 0;
            let mut running: Vec<Vec<(usize, Groups)>> = Vec::with_capacity(stages.len());
            for stage in stages {
                let mut held = Vec::new();
                for &count in *stage {
                    if !matches!(count, Groups::Fixed(0)) {
                        held.push((pass, count));
                    }
                    pass += 1;
                }
                if !held.is_empty() {
                    running.push(held);
                }
            }

            for repeat in 0..times {
                for (nth, stage) in running.iter().enumerate() {
                    for &(pass, count) in stage {
                        let &pipeline = self.pipelines.get(pass).ok_or(Error::WrongPasses {
                            given: pass,
                            held: self.pipelines.len(),
                        })?;
                        device.cmd_bind_pipeline(
                            self.command,
                            vk::PipelineBindPoint::COMPUTE,
                            pipeline,
                        );
                        match count {
                            // Not clamped to one. A caller asking for no workgroups means none, and
                            // rounding it up would run a pass that was told not to run, over buffers
                            // it may have had no reason to size for it. Such a pass never reaches
                            // here — it is filtered out above — and this arm stays honest about what
                            // a count of nothing means rather than relying on that filter.
                            Groups::Fixed(count) => device.cmd_dispatch(self.command, count, 1, 1),
                            Groups::From { binding, at } => {
                                let held = self.buffers.get(binding).ok_or(Error::NoSuchBinding {
                                    index: binding,
                                    held: self.buffers.len(),
                                })?;
                                let offset = (at * size_of::<u32>()) as u64;
                                device.cmd_dispatch_indirect(self.command, held.handle, offset);
                            }
                        }
                    }

                    // Between stages only: what one wrote, the next reads. A barrier after the last
                    // one would order nothing, and the copy below brings its own.
                    //
                    // **`DRAW_INDIRECT` is in the destination whether the next pass is indirect or
                    // not.** A workgroup count is read at that stage rather than in the shader, so a
                    // barrier that only mentioned `COMPUTE_SHADER` would order the buffer's
                    // *contents* and not the command that reads them. Naming it always costs nothing
                    // and cannot be forgotten for the one pass that needs it.
                    if nth + 1 < running.len() || repeat + 1 < times {
                        let barrier = [vk::MemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ
                                    | vk::AccessFlags::SHADER_WRITE
                                    | vk::AccessFlags::INDIRECT_COMMAND_READ,
                            )];
                        device.cmd_pipeline_barrier(
                            self.command,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::PipelineStageFlags::COMPUTE_SHADER
                                | vk::PipelineStageFlags::DRAW_INDIRECT,
                            vk::DependencyFlags::empty(),
                            &barrier,
                            &[],
                            &[],
                        );
                    }
                }
            }

            if let Some((handle, bytes)) = out {
                let barrier = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)];
                device.cmd_pipeline_barrier(
                    self.command,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &barrier,
                    &[],
                    &[],
                );

                let region = [vk::BufferCopy::default().size(bytes.max(4))];
                device.cmd_copy_buffer(self.command, handle, self.staging.handle, &region);

                let barrier = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)];
                device.cmd_pipeline_barrier(
                    self.command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::HOST,
                    vk::DependencyFlags::empty(),
                    &barrier,
                    &[],
                    &[],
                );
            }

            device.end_command_buffer(self.command)?;

            let commands = [self.command];
            let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
            device.reset_fences(&[self.fence])?;
            device.queue_submit(self.host.queue(), &submit, self.fence)?;
        }

        // **Submitted, not landed.** Whoever called this waits on the fence — `run` immediately,
        // `start` whenever its caller asks. Waiting here is what made a frame cost its round trip
        // with the host doing nothing.
        Ok(())
    }

    /// Record one command, submit it, and wait.
    ///
    /// # Safety
    ///
    /// `record` may only record into the buffer it is handed.
    unsafe fn once<F>(&self, record: F) -> Result<(), Error>
    where
        F: FnOnce(&ash::Device, vk::CommandBuffer),
    {
        // **The one command buffer again.** A staged write records and submits, so it cannot happen
        // while a frame is in flight. A *direct* write does not come through here at all — it is a
        // copy into mapped device memory with nothing submitted — which is why a caller with
        // resizable BAR may upload during a frame and a caller without it may not.
        if self.flight.is_some() {
            return Err(Error::StillInFlight);
        }
        let device = self.host.device();
        // SAFETY: the fence is waited on before returning, so nothing is in flight.
        unsafe {
            device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(self.command, &begin)?;
            record(device, self.command);
            device.end_command_buffer(self.command)?;

            let commands = [self.command];
            let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
            device.reset_fences(&[self.fence])?;
            device.queue_submit(self.host.queue(), &submit, self.fence)?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
        }
        Ok(())
    }

    /// The staging buffer's first `count` words.
    ///
    /// # Safety
    ///
    /// Whatever wrote them must have completed.
    unsafe fn mapped(&self, count: usize) -> Result<Vec<u32>, Error> {
        let bytes = ((count * size_of::<u32>()) as u64).max(4);
        // SAFETY: host-visible coherent memory, nothing else maps it, and the caller's contract
        // says the copy into it has finished.
        unsafe {
            let mapped = self.host.device().map_memory(
                self.staging.memory,
                0,
                bytes,
                vk::MemoryMapFlags::empty(),
            )?;
            let mut out = vec![0_u32; count];
            std::ptr::copy_nonoverlapping(mapped.cast::<u32>(), out.as_mut_ptr(), count);
            self.host.device().unmap_memory(self.staging.memory);
            Ok(out)
        }
    }
}

impl Drop for Resident<'_> {
    fn drop(&mut self) {
        let device = self.host.device();
        // SAFETY: every frame waits on its fence before returning, so nothing is in flight.
        unsafe {
            let _ = device.device_wait_idle();
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.commands, None);
            for &pipeline in &self.pipelines {
                device.destroy_pipeline(pipeline, None);
            }
            for &shader in &self.shaders {
                device.destroy_shader_module(shader, None);
            }
            device.destroy_pipeline_layout(self.pipeline_layout, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_descriptor_set_layout(self.layout, None);
            for held in self.buffers.iter().chain(std::iter::once(&self.staging)) {
                device.destroy_buffer(held.handle, None);
                device.free_memory(held.memory, None);
            }
        }
    }
}
