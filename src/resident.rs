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
//! and a fence wait is where the cost is.
//!
//! # What that costs, measured rather than inherited
//!
//! Every timing in this crate is a host clock around a submit and a fence, divided by however many
//! frames were in the submission. That is what a caller waits, and it is a *sum* — of what a
//! submission costs once, and of what one more frame inside it costs. `tests/floor.rs` takes the two
//! apart by varying the only thing that separates them, and reads them off the line:
//!
//! ```text
//!   a submission, holding nothing          35-42 us
//!   one more frame in it, holding nothing        0
//!   a dispatch                            210-245 ns
//!   a barrier                            996-1009 ns
//!   asking the device for one word         41-42 us
//! ```
//!
//! Over four runs, and the spreads say something themselves: a barrier comes back the same number
//! every time and a submission does not, which is what a pipeline flush looks like next to a queue
//! and a fence.
//!
//! **Thirty-seven, not the hundred this crate said everywhere.** That hundred came from `simdr`'s
//! `decisions/DR-0008`, which priced a round trip that *moves data* — `run_grid` uploads, dispatches
//! and downloads — and it is still about right for one: `src/bin/loop_cost.rs` measures 95 µs for a
//! frame that copies a megabyte back and 50 µs for the same frame that copies nothing. A bare submit
//! and fence is thirty-seven, and this file had been quoting the other number for its own.
//!
//! Nothing in the design changes for it. `crate::compact::prepare` exists so that a workgroup count
//! never comes back to the host, and forty-one microseconds to learn one number is still more than
//! the entire frame that produced it.

use std::time::{Duration, Instant};

use ash::vk;

use crate::host::{Error, Host};

/// How many workgroups a pass runs, and who decides.
///
/// **The whole point of the second one.** A compaction's output is a list whose length the *device*
/// worked out, and dispatching over it means the workgroup count has to come from a buffer rather
/// than from a caller who would have to read it back first. Reading it back is a submit and a fence
/// — 41 µs by `tests/floor.rs`'s measurement, which is more than the frame that produced it — to
/// learn a number the device already had.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Groups {
    /// A count the host knows.
    ///
    /// **Nought means nought, and a pass told nought is not recorded at all.** It is how a caller
    /// holding several passes runs one and skips the others, and a skipped pass costs no dispatch
    /// and takes no barrier with it — `tests/dispatch.rs` holds both halves of that.
    Fixed(u32),
    /// A count the host knows, from a pass that **writes** a workgroup count for a later one.
    ///
    /// # Why this is a variant and not a flag somewhere else
    ///
    /// Because a count the device wrote has to be made visible to the command that reads it, and
    /// that is a barrier naming `DRAW_INDIRECT`. `tests/floor.rs` prices a stage that wrote one at
    /// 2373–2401 ns against 1097–1098 ns for a stage that did not, so naming it where it is not
    /// needed is most of a barrier thrown away.
    ///
    /// `Resident` cannot work out where it is needed: it sees which binding a `From` reads and has
    /// no idea which binding a kernel writes. So the caller says, the way it already says which
    /// passes may share a stage — `decisions/DR-0005`. This marks the *writer*; every later stage
    /// reads what it wrote through the one barrier that follows it, because Vulkan makes a write
    /// visible to every subsequent indirect read rather than to the next one only.
    ///
    /// **Nought is not recorded either**, and a pass that did not run wrote no count, so it takes no
    /// barrier with it.
    Counting(u32),
    /// Three words at `binding`, from `at`: the x, y and z counts, as Vulkan wants them.
    From {
        /// Which buffer holds them.
        binding: usize,
        /// Which word of it the three begin at.
        at: usize,
    },
}

/// What the command buffer already holds, so that a frame the same shape as the last is not
/// recorded again.
///
/// # Why a frame can be recorded once and submitted many times
///
/// Because **none of what changes between frames is in the recording.** A dispatch names a pipeline
/// and a workgroup count; a descriptor set names buffers. What a frame actually differs by is what
/// those buffers *hold*, and a command buffer does not carry that. `Groups::From` is the sharpest
/// case: the count is read out of a buffer at the `DRAW_INDIRECT` stage, so a recording that says
/// "dispatch from binding eight" is correct on every frame however the count changes.
///
/// So the recording is a function of exactly the three things below, and a caller that hands over
/// the same three wants the same commands. `grit`'s loop hands over the same three every frame; a
/// caller that varies a workgroup count between calls — `hyperia`'s card tier does, per token —
/// gets a miss and pays what it used to.
///
/// # What it is worth
///
/// About 7 µs a frame, measured with [`Resident::recording`] in `src/bin/scale.rs` when that file
/// re-recorded every frame and ran sixteen passes at every size, against a frame of
/// 35 µs once the round trip is spread over twenty of them. It is **host** time, which is what makes
/// it worth more than the number suggests: it is spent on the thread that ought to be waiting for
/// something else.
///
/// # And what invalidates it
///
/// [`Resident::once`] records into the same command buffer, so every staged write and every read
/// throws this away. That is the whole of the invalidation, and it is why this is set to `None`
/// there rather than reasoned about here.
#[derive(Debug, PartialEq, Eq)]
struct Recorded {
    /// The passes, grouped as the caller grouped them.
    stages: Vec<Vec<Groups>>,
    /// Which buffers are copied out at the end, and how much of each.
    out: Vec<(vk::Buffer, u64)>,
    /// How many frames were recorded back to back.
    times: u32,
}

/// A buffer and the memory under it.
struct Held {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    bytes: u64,
    /// Whether the host can write into it without a submission.
    ///
    /// Decided per buffer rather than per `Resident`: a window of device-local host-visible memory
    /// is a few hundred megabytes where a model's weights are gigabytes, and asking for all of them
    /// at once turns a token of 8 KiB into a staged copy because an expert slot would not fit.
    visible: bool,
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
#[derive(Debug, Clone)]
struct Flight {
    /// Which ticket this is.
    ticket: u64,
    /// How many words to hand back from the staging buffer, in the order they were copied into it.
    ///
    /// Two, where a caller asked a frame for two buffers — see [`Resident::submit_reading_both`].
    /// Empty where it asked for none.
    out: Vec<usize>,
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
    /// The second command buffer, for one-off copies. See `Recorded` for why it is not the first.
    transfer: vk::CommandBuffer,
    fence: vk::Fence,
    /// The submission on the device, if there is one.
    ///
    /// `None` when nothing is in flight. A second submission while this is `Some` is refused rather
    /// than queued: there is one command buffer, and resetting it under a running submission is
    /// undefined behaviour rather than a slow path.
    flight: Option<Flight>,
    /// How many submissions this has made, which is where the next ticket's number comes from.
    tickets: u64,
    /// Whether the staging buffer is in cached host memory.
    cached: bool,
    /// How long has gone into recording command buffers, and over how many frames.
    ///
    /// A frame's passes are recorded afresh on every call — reset, begin, bind, and a dispatch and
    /// a barrier apiece. The structure does not change between calls; only what the parameters
    /// buffer holds does. So this is the part of a frame's cost that is repetition, measured rather
    /// than reasoned about, because a caller's own clock cannot separate it from the submit.
    recording: Duration,
    /// How many frames that was over.
    recorded: u64,
    /// How long has gone into one-off copies on and off the card, and how many there were.
    ///
    /// A `write` that cannot go straight into device memory, and **every** `read`, records a copy,
    /// submits it and waits on its fence. That is a whole round trip for a transfer, and it does
    /// not appear in the frame timings a caller sees because it is not a frame.
    copying: Duration,
    /// How many of those there were.
    copies: u64,
    /// How long has gone into writing buffers, and how many writes that was.
    ///
    /// A write is either a map, a copy and an unmap — which is `direct` — or a staged copy with a
    /// submit and a fence behind it, which is not. Neither is a frame, so neither shows up in a
    /// caller's frame timings, and the two differ by about fifty microseconds a call.
    pushing: Duration,
    /// How many writes that was.
    pushes: u64,
    /// How many of them went straight into device memory rather than through a submission.
    poked: u64,
    /// How long a fence is waited on before the device is presumed lost.
    patience: Duration,
    /// Two timestamps a submission, where the queue counts them at all.
    ///
    /// **The only number in this crate that comes from the device's own clock.** Everything else is
    /// a host clock around a submit and a fence, which `tests/floor.rs` shows is 35-42 us before any
    /// work happens -- so what a frame spends *on the card* was a subtraction until this.
    ///
    /// `None` where the queue family reports no valid timestamp bits.
    queries: Option<vk::QueryPool>,
    /// What the last submission measured on the device, per frame.
    device: Option<Duration>,
    /// What the command buffer holds, if it holds a frame. See [`Recorded`].
    held: Option<Recorded>,
    /// How many submissions went out over a recording that was already there.
    reused: u64,
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
        self.buffers_in(sizes, modules, false, 0)
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
        self.buffers_in(sizes, modules, true, 0)
    }

    /// Buffers the host can write into without a submission **where they fit**, and device-local
    /// memory for the rest.
    ///
    /// # Why this rather than [`Host::resident_direct`]
    ///
    /// Because that asks for every buffer at once and fails if any of them will not fit. Memory
    /// that is both device-local and host-visible is a window of a few hundred megabytes on most
    /// cards; a model's weights are gigabytes. So a caller holding one large buffer and a handful
    /// of small ones is refused, falls back, and pays a submission and a fence for **every** write
    /// — including the small ones written on every frame, which would have fitted the window a
    /// thousand times over.
    ///
    /// `window` is the largest buffer worth asking for. Anything above it goes device-local without
    /// asking; anything at or below it is asked for and falls back quietly where the window is
    /// full. Nought asks for nothing, which is [`Host::resident`].
    ///
    /// # Errors
    ///
    /// As [`Host::resident`]. A buffer that will not fit the window is not an error — it is the
    /// ordinary case, and it is what the fallback is for.
    pub fn resident_near<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        window: u64,
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, false, window)
    }

    /// Allocate `sizes` buffers and build one pipeline per module over them.
    fn buffers_in<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        direct: bool,
        window: u64,
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
            let usage_of = vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST
                // Any binding may end up holding a workgroup count, and declaring it here saves the
                // caller telling the allocator which.
                //
                // **This used to say it costs nothing, and nothing has measured that.** It is a
                // creation-time flag, so it cannot reach a recorded dispatch — what it could reach
                // is `vkGetBufferMemoryRequirements`, in size, alignment or which memory types
                // suit. That has not been looked at. The reason for saying so rather than assuming
                // is thirty lines below: the same file said naming `DRAW_INDIRECT` in a barrier
                // "costs nothing", and it cost a fifth of the frame.
                | vk::BufferUsageFlags::INDIRECT_BUFFER;
            let near = vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT;

            let mut buffers = Vec::with_capacity(sizes.len());
            for &size in sizes {
                let bytes = words(size);

                // **Small enough to be worth the window, and asked for one at a time.** A buffer
                // written every frame wants memory the host can poke; a buffer holding an expert
                // does not, and there is not enough of the first kind for both. Where `window` is
                // nought this asks nothing and every buffer is `living`, which is what `resident`
                // and `resident_direct` want.
                let held = if bytes <= window {
                    match self.allocate(bytes, usage_of, near) {
                        Ok(held) => Some(Held { visible: true, ..held }),
                        Err(_) => None,
                    }
                } else {
                    None
                };
                buffers.push(match held {
                    Some(held) => held,
                    None => Held {
                        visible: direct,
                        ..self.allocate(bytes, usage_of, living)?
                    },
                });
            }

            // **Cached where the device has it.** Staging is written by the host on the way in
            // and *read* by the host on the way out, and uncached write-combined memory — which is
            // what `HOST_VISIBLE | HOST_COHERENT` alone usually gets — is fast to write and about
            // 0.3 GB/s to read. Asked for first, and fallen back from rather than failed on,
            // because a device with no cached host-visible type is a device this still works on.
            let usage = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST;
            let plain = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let (staging, cached) = match self.allocate(
                largest,
                usage,
                plain | vk::MemoryPropertyFlags::HOST_CACHED,
            ) {
                Ok(held) => (held, true),
                Err(_) => (self.allocate(largest, usage, plain)?, false),
            };

            let (layout, pool, set) = self.describe(&buffers)?;
            let (pipeline_layout, shaders, pipelines) = self.compile(layout, modules)?;
            let (commands, command, transfer) = self.record_into()?;
            let fence = self
                .device()
                .create_fence(&vk::FenceCreateInfo::default(), None)?;

            // Two timestamps: one before the first pass and one after the last. Asked for only
            // where the queue counts them, so a device without them costs nothing here and answers
            // `None` rather than a wrong number.
            let queries = if self.ticking().1 {
                let create = vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::TIMESTAMP)
                    .query_count(2);
                self.device().create_query_pool(&create, None).ok()
            } else {
                None
            };

            Ok(Resident {
                recording: Duration::ZERO,
                recorded: 0,
                copying: Duration::ZERO,
                copies: 0,
                pushing: Duration::ZERO,
                pushes: 0,
                poked: 0,
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
                transfer,
                fence,
                flight: None,
                tickets: 0,
                cached,
                patience: Resident::PATIENCE,
                held: None,
                reused: 0,
                queries,
                device: None,
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
            // Whatever the caller asked for; `buffers_in` says which it settled on.
            visible: wanted.contains(vk::MemoryPropertyFlags::HOST_VISIBLE),
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
    unsafe fn record_into(
        &self,
    ) -> Result<(vk::CommandPool, vk::CommandBuffer, vk::CommandBuffer), Error> {
        let create = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.family())
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: the family is the one the device was created with.
        let pool = unsafe { self.device().create_command_pool(&create, None) }?;

        // **Two, and the second is what makes a kept recording worth keeping.** A frame is recorded
        // into the first and left there; a one-off copy — every staged write and every read — is
        // recorded into the second. They used to be one buffer, so a single parameter word written
        // between two frames threw the frame away and it was recorded again. Measured on `grit`'s
        // loop, which writes four brush words between batches: **recorded sixteen times, reused
        // nought.** See `Recorded`.
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(2);
        // SAFETY: the pool was just created by this device.
        let buffers = unsafe { self.device().allocate_command_buffers(&allocate) }?;
        let command = buffers.first().copied().unwrap_or_default();
        let transfer = buffers.get(1).copied().unwrap_or_default();

        Ok((pool, command, transfer))
    }
}

impl Resident<'_> {
    /// How long a fence is waited on, by default, before the device is presumed lost.
    ///
    /// This is not an estimate of how long work takes. `simdr`'s `decisions/DR-0008` prices a round
    /// trip at 37 µs — `tests/floor.rs` — so ten seconds is five orders of magnitude of
    /// headroom: it is the point past which work is not coming back, not the point past which it is
    /// slow. Windows resets a *graphics* device that misses its watchdog in about two seconds; a
    /// compute queue has no such watchdog, which is the whole reason this number has to exist.
    pub const PATIENCE: Duration = Duration::from_secs(10);

    /// How long this one waits before giving up on a submission.
    pub const fn patience(&self) -> Duration {
        self.patience
    }

    /// Wait a different length of time before giving up.
    ///
    /// For a caller whose submissions are genuinely longer than [`Resident::PATIENCE`] — a batch
    /// sized so that one frame is seconds of arithmetic rather than milliseconds.
    pub fn set_patience(&mut self, patience: Duration) {
        self.patience = patience;
    }

    /// Refuse to touch the device once a submission has been given up on.
    ///
    /// Every entry point that reaches the queue or maps memory asks this first. A wedged device
    /// still owns the command buffer, the fence and every buffer bound to them, so recording into
    /// them again is undefined behaviour, and mapping them reads whatever the dispatch that never
    /// finished is still doing.
    fn live(&self) -> Result<(), Error> {
        if self.host.wedged() {
            return Err(Error::Wedged {
                after: self.patience,
            });
        }
        Ok(())
    }

    /// Wait on the fence for at most [`Resident::patience`], and poison the host if it does not
    /// come back.
    ///
    /// # Why this is bounded when it used to wait forever
    ///
    /// A dispatch that never signals its fence parks the calling thread inside the driver, and a
    /// thread parked there has no way out: the wait cannot be cancelled and the submission cannot
    /// be taken back. Bounding it turns a thread with no remaining move into an error a caller can
    /// act on, which is worth having whether or not anything has hit it yet.
    ///
    /// # What this was written for, and did not fix
    ///
    /// `hyperia`'s mutation gate lost twenty-eight sandboxes in one run over `src/gemv.rs` to test
    /// processes that outlived their kill, and an unbounded wait here was the obvious suspect. It
    /// was the wrong one. With this bound merged the very next run quarantined a sandbox anyway,
    /// and the mutant caught in the act was `strips = wide / subgroup` turned into `wide *
    /// subgroup` — a kernel about a thousand times larger, which is slow rather than stuck. Those
    /// processes escape `taskkill /T` because the `cargo` between them and the killer has already
    /// exited, which is a fact about process trees and nothing to do with fences.
    ///
    /// So: a real bug, fixed, and not the one it was reached for. Kept because an unbounded wait is
    /// indefensible on its own terms, not because it bought anything back.
    ///
    /// A timeout does not rescue the submission. Nothing can: it stays the device's. What it
    /// rescues is the *thread*, which can now return an error and let the process end.
    fn await_fence(&self) -> Result<(), Error> {
        let patience = u64::try_from(self.patience.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the fence belongs to this resident, and the caller has established that the
        // submission it belongs to is the one outstanding.
        let waited = unsafe {
            self.host
                .device()
                .wait_for_fences(&[self.fence], true, patience)
        };
        match waited {
            Ok(()) => Ok(()),
            Err(vk::Result::TIMEOUT) => {
                self.host.wedge();
                Err(Error::Wedged {
                    after: self.patience,
                })
            }
            Err(code) => Err(code.into()),
        }
    }

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

    /// How long has gone into writing buffers, how many writes, and whether they went straight in.
    ///
    /// The third is how many of them went straight into device memory. A direct write maps, copies
    /// and unmaps; a staged one records a copy, submits it and waits on a fence, which is a whole
    /// round trip for a transfer. Neither is a frame, so a caller timing its frames sees neither.
    #[must_use]
    pub const fn pushing(&self) -> (Duration, u64, u64) {
        (self.pushing, self.pushes, self.poked)
    }

    /// Put `words` into binding `index`, through the staging buffer.
    ///
    /// **Once, at the start.** Everything after that stays where it is.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchBinding`] or [`Error::Overrun`], otherwise whatever Vulkan said.
    pub fn write(&mut self, index: usize, words: &[u32]) -> Result<(), Error> {
        self.live()?;
        let began = std::time::Instant::now();
        let out = self.written(index, words);
        self.pushing += began.elapsed();
        self.pushes += 1;
        // Said by the write itself rather than looked up again: `written` already has the buffer in
        // hand, and asking `at` a second time is a second bounds check for a counter.
        self.poked += u64::from(*out.as_ref().unwrap_or(&false));
        out.map(|_| ())
    }

    /// The write itself, with the clock around it in [`Resident::write`].
    ///
    /// True where it went straight into device memory, false where it was staged.
    fn written(&mut self, index: usize, words: &[u32]) -> Result<bool, Error> {
        let held = self.at(index)?;
        let bytes = size_of_val(words) as u64;
        if bytes > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: words.len(),
                held: (held.bytes as usize) / size_of::<u32>(),
            });
        }
        if held.visible {
            // SAFETY: this buffer's memory is host-visible and coherent — it was allocated asking
            // for nothing less, which is what `visible` records — and nothing else maps it for the
            // length of this call.
            return unsafe { self.poke(held.memory, 0, words) }.map(|()| true);
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
        // Lifted out of the closure: `once` takes `&mut self` now, so that it can forget the frame
        // it is about to record over, and a closure reading `self` would be a second borrow.
        let from = self.staging.handle;
        // SAFETY: both buffers are live and the region is inside both.
        unsafe {
            self.once(|device, command| device.cmd_copy_buffer(command, from, handle, &region))
                .map(|()| false)
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

    /// Whether the staging buffer is in **cached** host memory.
    ///
    /// Everything the host reads back off this device comes through staging, and uncached
    /// write-combined memory reads at about 0.3 GB/s — which is the difference between a read-back
    /// being free and being the largest thing in a call. False on a device offering no cached
    /// host-visible memory type, and worth asking before believing a figure that includes one.
    #[must_use]
    pub const fn cached(&self) -> bool {
        self.cached
    }

    /// Whether **every** buffer of this `Resident` is one the host can write without a submission.
    ///
    /// True only for a [`Host::resident_direct`]. False for a [`Host::resident_near`] even where
    /// most of its buffers got that memory — the small ones written every frame usually do and the
    /// large ones cannot — so this is a question about the whole allocation and not about any one
    /// write. What a given write did is [`Resident::pushing`], which counts them.
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
        self.live()?;
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
        if held.visible {
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
        let from = self.staging.handle;
        // SAFETY: both buffers are live and the region is inside both.
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, from, handle, &region);
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
        self.live()?;
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

    /// How long has gone into one-off copies on and off the card, and how many there were.
    ///
    /// Every `read` is a recorded copy, a submit and a fence wait — a round trip for a transfer,
    /// spent outside any frame. A caller timing its frames does not see it.
    #[must_use]
    pub const fn copying(&self) -> (Duration, u64) {
        (self.copying, self.copies)
    }

    /// Read `count` words back from binding `index`.
    ///
    /// # Errors
    ///
    /// As [`Resident::write`].
    pub fn read(&mut self, index: usize, count: usize) -> Result<Vec<u32>, Error> {
        self.live()?;
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
        let began = std::time::Instant::now();
        let into = self.staging.handle;
        // SAFETY: as in `write`.
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, handle, into, &region);
            })
        }?;
        self.copying += began.elapsed();
        self.copies += 1;
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
        self.start(stages, &[], 1)
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
        self.start(stages, &[(index, count)], 1)
    }

    /// [`Resident::submit_reading`], and copy a **second** binding out in the same submission.
    ///
    /// # Why two and not a read afterwards
    ///
    /// Because a read afterwards is a second submit and a second fence, which this file already
    /// says is 37 µs for nothing — and a caller whose frame produces two answers
    /// was paying it on every frame. Both copies land in the staging buffer one after the other and
    /// come back together from [`Resident::wait_reading_both`].
    ///
    /// # Errors
    ///
    /// As [`Resident::submit_reading`], and [`Error::Overrun`] where the two together will not fit
    /// the staging buffer.
    pub fn submit_reading_both(
        &mut self,
        stages: &[&[Groups]],
        first: (usize, usize),
        second: (usize, usize),
    ) -> Result<Ticket, Error> {
        self.bounded(first.0, first.1)?;
        self.bounded(second.0, second.1)?;
        self.start(stages, &[first, second], 1)
    }

    /// Record every pass, submit, and copy two bindings out when the work lands.
    ///
    /// # Errors
    ///
    /// As [`Resident::submit_reading_both`].
    pub fn staged_reading_both(
        &mut self,
        stages: &[&[Groups]],
        first: (usize, usize),
        second: (usize, usize),
    ) -> Result<(Duration, Vec<u32>, Vec<u32>), Error> {
        let ticket = self.submit_reading_both(stages, first, second)?;
        self.wait_reading_both(ticket)
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
        let (spent, mut every) = self.landed(ticket)?;
        Ok((spent, if every.is_empty() { Vec::new() } else { every.swap_remove(0) }))
    }

    /// The same, for a frame that asked for two buffers — see [`Resident::submit_reading_both`].
    ///
    /// **The ticket is spent either way.** The fence is waited on and the flight cleared before the
    /// shape is checked, so a frame that copied one buffer has landed by the time this refuses — and
    /// the one answer it did copy is gone with it. That is the cost of the refusal being a caller's
    /// mistake rather than a device's, and it is why the alternative is worse: an empty second
    /// vector is a wrong answer that reads like a right one.
    ///
    /// # Errors
    ///
    /// As [`Resident::wait_reading`], and [`Error::WrongOutputs`] where the frame copied fewer than
    /// two.
    pub fn wait_reading_both(
        &mut self,
        ticket: Ticket,
    ) -> Result<(Duration, Vec<u32>, Vec<u32>), Error> {
        let (spent, mut every) = self.landed(ticket)?;
        if every.len() < 2 {
            return Err(Error::WrongOutputs {
                asked: every.len(),
                wanted: 2,
            });
        }
        let second = every.swap_remove(1);
        Ok((spent, every.swap_remove(0), second))
    }

    /// Wait, and take every buffer the frame asked for out of the staging buffer in order.
    fn landed(&mut self, ticket: Ticket) -> Result<(Duration, Vec<Vec<u32>>), Error> {
        let (spent, counts) = self.land(ticket)?;
        let mut every = Vec::with_capacity(counts.len());
        let mut at = 0_usize;
        for count in counts {
            // SAFETY: the submission has completed — `land` waited on its fence — and the copies
            // were recorded into the staging buffer at these offsets in this order.
            every.push(unsafe { self.mapped_at(at, count) }?);
            at += count.max(1);
        }
        Ok((spent, every))
    }

    /// The same, and copy `count` words of binding `index` back inside the same submission.
    ///
    /// **One fence for the whole frame.** Reading afterwards with [`Resident::read`] would be a
    /// second submit and a second wait, which on this machine is another 37 µs for
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
        out: &[(usize, usize)],
        times: u32,
    ) -> Result<Ticket, Error> {
        // **Ahead of the in-flight check, not behind it.** A wedged resident has a submission that
        // never landed, so `flight` is still `Some` and this would otherwise answer `StillInFlight`
        // — true in its way, and the wrong thing to tell a caller whose device is gone. A poisoned
        // device says so before it says anything else.
        self.live()?;
        if self.flight.is_some() {
            return Err(Error::StillInFlight);
        }
        let mut copy = Vec::with_capacity(out.len());
        let mut wanted = 0_u64;
        for &(index, count) in out {
            let held = self.at(index)?;
            let bytes = (count * size_of::<u32>()) as u64;
            copy.push((held.handle, bytes));
            wanted += bytes.max(4);
        }
        // Both copies land in the one staging buffer, one after the other, so together they have to
        // fit it. Refused here rather than overrunning it, which would be the second copy quietly
        // landing on top of the first.
        if wanted > self.staging.bytes {
            return Err(Error::Overrun {
                index: out.len(),
                needed: (wanted as usize) / size_of::<u32>(),
                held: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }

        let began = Instant::now();
        self.run_times(stages, &copy, times)?;
        self.tickets = self.tickets.wrapping_add(1);
        self.flight = Some(Flight {
            ticket: self.tickets,
            out: out.iter().map(|&(_, count)| count).collect(),
            began,
        });
        Ok(Ticket(self.tickets))
    }

    /// Wait for the outstanding submission, and say how long it took and what it owes.
    ///
    /// The duration is from the submit and not from here, so it is the round trip whether the
    /// caller waited immediately or went away and did something first.
    fn land(&mut self, ticket: Ticket) -> Result<(Duration, Vec<usize>), Error> {
        self.live()?;
        // Read out of the flight rather than moved out of it: a mismatched ticket leaves the real
        // submission outstanding, and taking it here would forget a frame that is still running.
        let (began, out) = {
            let Some(held) = self.flight.as_ref() else {
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
            (held.began, held.out.clone())
        };

        // **`flight` is cleared only on the way out.** Where the fence does not come back the
        // submission really is still outstanding, so leaving it recorded is the true statement.
        self.await_fence()?;
        self.flight = None;
        self.device = self.timed();
        Ok((began.elapsed(), out))
    }

    /// Record `times` frames back to back into one command buffer, submit once, and wait once.
    ///
    /// # What this is for, and what a single frame cannot tell you
    ///
    /// [`Resident::frame`] times a submit and a fence around the work, and on this machine that
    /// round trip costs 37 µs whatever is inside it — `tests/floor.rs` prices it, and `decisions/DR-0008` in
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
        self.staged_frames(&stages, times)
    }

    /// The same, over passes a caller has said are not all ordered against each other.
    ///
    /// **The measurement that made this worth having.** [`Resident::frames`] puts every pass in a
    /// stage of its own, which is a barrier between each of them, and `tests/floor.rs` prices a
    /// barrier at 981–1013 ns against 224–232 ns for a dispatch. Ten of them are most of a frame at
    /// a size where no pass has any work to do.
    ///
    /// It was 3.8 µs a barrier when this went in, measured by taking five passes out of `grit`'s
    /// seventeen — 60.2 µs to 41.2 — and that number was a barrier naming `DRAW_INDIRECT`, which all
    /// ten did then. `decisions/DR-0002` is what that turned out to be worth.
    ///
    /// So the passes that do not read what their neighbours write want saying so, and this is the
    /// batched timing for them. `crate::live::wired` is where that grouping is decided.
    ///
    /// # Errors
    ///
    /// As [`Resident::frames`].
    pub fn staged_frames(&mut self, stages: &[&[Groups]], times: u32) -> Result<Duration, Error> {
        let ticket = self.start(stages, &[], times.max(1))?;
        self.wait(ticket).map(|spent| spent / times.max(1))
    }

    /// How long has gone into recording command buffers, and over how many frames.
    ///
    /// The passes of a frame are recorded afresh on every call. Their structure does not change
    /// between calls — only the parameters buffer does — so this is the part of a frame's cost that
    /// is repetition, and a caller's own clock cannot tell it from the submit.
    #[must_use]
    pub const fn recording(&self) -> (Duration, u64) {
        (self.recording, self.recorded)
    }

    /// How many submissions went out over commands that were already there.
    ///
    /// A frame is recorded once and submitted again while the passes, their grouping and the frame
    /// count match the last call. What decides that is `Recorded` in `src/resident.rs`, which is
    /// private — a caller cannot reach it, and does not need to.
    /// Against [`Resident::recording`]'s second number, this is the hit rate: a caller that hands
    /// over the same passes and the same grouping every frame records once and reuses for ever,
    /// and one that varies a workgroup count between calls records every time.
    ///
    /// **A caller that expects the first and measures the second has a bug rather than a slow
    /// frame** — something in its stage list is changing that it did not mean to change, and this
    /// is the only place that would say so.
    #[must_use]
    pub const fn reused(&self) -> u64 {
        self.reused
    }

    /// What the last submission spent on the device, per frame, from the device's own clock.
    ///
    /// `None` where the queue family counts no timestamps, or where the query has not landed.
    ///
    /// **This is the only figure in this crate that is not a host clock.** Everything else brackets
    /// a submit and a fence, and `tests/floor.rs` shows that is 41.1–43.9 µs before any work happens —
    /// so what a frame spends on the card was a subtraction until this existed.
    #[must_use]
    pub const fn on_device(&self) -> Option<Duration> {
        self.device
    }

    /// Read the two stamps and turn the difference into a duration a frame at a time.
    fn timed(&self) -> Option<Duration> {
        let pool = self.queries?;
        let times = self.held.as_ref().map_or(1, |held| held.times).max(1);
        let mut stamps = [0_u64; 2];
        // SAFETY: the fence has been waited on, so both writes have landed.
        unsafe {
            self.host
                .device()
                .get_query_pool_results(pool, 0, &mut stamps, vk::QueryResultFlags::TYPE_64)
                .ok()?;
        }
        let [first, last] = stamps;
        let ticks = last.checked_sub(first)?;
        let (tick, _) = self.host.ticking();
        let nanos = ticks as f64 * f64::from(tick) / f64::from(times);
        Some(Duration::from_nanos(nanos as u64))
    }

    /// The recording, `times` frames over.
    fn run_times(
        &mut self,
        stages: &[&[Groups]],
        out: &[(vk::Buffer, u64)],
        times: u32,
    ) -> Result<(), Error> {
        self.live()?;
        let given: usize = stages.iter().map(|one| one.len()).sum();
        if given != self.pipelines.len() {
            return Err(Error::WrongPasses {
                given,
                held: self.pipelines.len(),
            });
        }

        let device = self.host.device();
        let began = std::time::Instant::now();
        let took;

        let commands = [self.command];
        let submit = [vk::SubmitInfo::default().command_buffers(&commands)];

        // What was asked for, in the form the command buffer can be asked whether it already holds.
        let wanted = Recorded {
            stages: stages.iter().map(|one| one.to_vec()).collect(),
            out: out.to_vec(),
            times,
        };
        if self.held.as_ref() == Some(&wanted) {
            // SAFETY: nothing is in flight — `start` refuses while something is — and the recording
            // below is made *without* `ONE_TIME_SUBMIT`, so the command buffer is still in the
            // executable state and may be submitted again as it stands.
            unsafe {
                device.reset_fences(&[self.fence])?;
                device.queue_submit(self.host.queue(), &submit, self.fence)?;
            }
            self.reused += 1;
            return Ok(());
        }

        // **Forgotten before it is rebuilt, not after.** A recording that fails part way through
        // leaves the command buffer in a state nobody can describe, and every `?` below is a place
        // that can happen. What is remembered has to stop being true at the moment it does.
        self.held = None;

        // SAFETY: nothing is in flight — `start` is the only caller and it refuses while something
        // is — so resetting the command buffer here cannot touch a running submission.
        unsafe {
            device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            // **No `ONE_TIME_SUBMIT`, which is what makes the reuse above legal.** That flag tells
            // the implementation the recording will be submitted once and is free to be invalid
            // afterwards; without it the buffer returns to the executable state and may go again.
            // Nothing here needs `SIMULTANEOUS_USE`: two submissions never overlap, because there is
            // one fence and `start` refuses a second while the first is outstanding.
            let begin = vk::CommandBufferBeginInfo::default();
            device.begin_command_buffer(self.command, &begin)?;

            // **Reset and first stamp before anything is bound.** The reset has to be inside the
            // command buffer rather than on the host, because the recording is kept and replayed:
            // a pool reset done once at record time would leave every later submission writing into
            // slots that already hold an answer.
            if let Some(pool) = self.queries {
                device.cmd_reset_query_pool(self.command, pool, 0, 2);
                device.cmd_write_timestamp(
                    self.command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    pool,
                    0,
                );
            }

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
                    if !matches!(count, Groups::Fixed(0) | Groups::Counting(0)) {
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
                            Groups::Fixed(count) | Groups::Counting(count) => {
                                device.cmd_dispatch(self.command, count, 1, 1);
                            }
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
                    // **`DRAW_INDIRECT` after the stage that wrote a count, not before every stage
                    // that reads one.** A workgroup count is read at that pipeline stage rather than
                    // in the shader, so a barrier naming only `COMPUTE_SHADER` would order the
                    // buffer's *contents* and not the command that reads them. This once named it
                    // everywhere, on the grounds that doing so "costs nothing and cannot be
                    // forgotten"; that was worth a fifth of the frame, and it then named it in front
                    // of every reader, which is worth two barriers more.
                    //
                    // Vulkan makes a write visible to every *later* indirect read, not to the next
                    // one only. So one barrier behind the writer covers every reader after it, and
                    // asking again in front of each of them buys nothing —
                    // `tests/floor.rs` prices the difference at about 1300 ns a stage.
                    //
                    // Which pass wrote a count is the one thing `Resident` cannot see: it knows
                    // which binding a `Groups::From` reads and nothing about what a kernel writes.
                    // `Groups::Counting` is the caller saying so, as `decisions/DR-0005` has it.
                    let next = if nth + 1 < running.len() {
                        running.get(nth + 1)
                    } else if repeat + 1 < times {
                        running.first()
                    } else {
                        None
                    };
                    if next.is_some() {
                        let counted = stage
                            .iter()
                            .any(|&(_, count)| matches!(count, Groups::Counting(_)));
                        let (reach, stage) = if counted {
                            (
                                vk::AccessFlags::INDIRECT_COMMAND_READ,
                                vk::PipelineStageFlags::DRAW_INDIRECT,
                            )
                        } else {
                            (
                                vk::AccessFlags::empty(),
                                vk::PipelineStageFlags::empty(),
                            )
                        };
                        let barrier = [vk::MemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ
                                    | vk::AccessFlags::SHADER_WRITE
                                    | reach,
                            )];
                        device.cmd_pipeline_barrier(
                            self.command,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::PipelineStageFlags::COMPUTE_SHADER | stage,
                            vk::DependencyFlags::empty(),
                            &barrier,
                            &[],
                            &[],
                        );
                    }
                }
            }

            let mut into = 0_u64;
            for &(handle, bytes) in out {
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

                let region = [vk::BufferCopy::default()
                    .dst_offset(into)
                    .size(bytes.max(4))];
                device.cmd_copy_buffer(self.command, handle, self.staging.handle, &region);
                into += bytes.max(4);

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

            if let Some(pool) = self.queries {
                device.cmd_write_timestamp(
                    self.command,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    pool,
                    1,
                );
            }

            device.end_command_buffer(self.command)?;
            took = began.elapsed();

            device.reset_fences(&[self.fence])?;
            device.queue_submit(self.host.queue(), &submit, self.fence)?;
        }

        // Only now, with the whole recording behind it and nothing left that can fail.
        self.held = Some(wanted);
        self.recording += took;
        self.recorded += 1;

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
        // **The one fence.** A staged write records and submits, so it cannot happen while a frame
        // is in flight. A *direct* write does not come through here at all — it is a copy into
        // mapped device memory with nothing submitted — which is why a caller with resizable BAR
        // may upload during a frame and a caller without it may not.
        //
        // The command *buffer* is a second one, and that is not a tidiness. Recording here into the
        // frame's buffer threw away whatever it held, and the frame was then recorded afresh — which
        // is what a caller writing one parameter word between two frames does. See [`Recorded`].
        self.live()?;
        if self.flight.is_some() {
            return Err(Error::StillInFlight);
        }
        let device = self.host.device();
        // SAFETY: the fence is waited on before returning, so nothing is in flight.
        unsafe {
            device.reset_command_buffer(self.transfer, vk::CommandBufferResetFlags::empty())?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(self.transfer, &begin)?;
            record(device, self.transfer);
            device.end_command_buffer(self.transfer)?;

            let commands = [self.transfer];
            let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
            device.reset_fences(&[self.fence])?;
            device.queue_submit(self.host.queue(), &submit, self.fence)?;
        }
        self.await_fence()
    }

    /// The staging buffer's first `count` words.
    ///
    /// # Safety
    ///
    /// Whatever wrote them must have completed.
    unsafe fn mapped(&self, count: usize) -> Result<Vec<u32>, Error> {
        // SAFETY: the caller's contract, with nothing before it in the staging buffer.
        unsafe { self.mapped_at(0, count) }
    }

    /// The same, `at` words into the staging buffer.
    ///
    /// # Safety
    ///
    /// As [`Resident::mapped`], and the words at that offset must be the ones the caller means —
    /// which for a frame is the order its copies were recorded in.
    unsafe fn mapped_at(&self, at: usize, count: usize) -> Result<Vec<u32>, Error> {
        let from = (at * size_of::<u32>()) as u64;
        let bytes = ((count * size_of::<u32>()) as u64).max(4);
        // SAFETY: host-visible coherent memory, nothing else maps it, and the caller's contract
        // says the copy into it has finished.
        unsafe {
            let mapped = self.host.device().map_memory(
                self.staging.memory,
                from,
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
        // **A wedged device is leaked on purpose.** Every handle below is still the device's: a
        // submission that never landed still owns its command buffer, its fence and every buffer
        // bound to them, and destroying those is undefined behaviour rather than a tidy-up.
        // `device_wait_idle` would not get past it either — it waits for a queue that never
        // drains. So this hands the memory to the operating system and returns. `Error::Wedged`.
        if self.host.wedged() {
            return;
        }
        let device = self.host.device();
        // SAFETY: every frame waits on its fence before returning, so nothing is in flight.
        unsafe {
            let _ = device.device_wait_idle();
            device.destroy_fence(self.fence, None);
            device.destroy_command_pool(self.commands, None);
            if let Some(pool) = self.queries {
                device.destroy_query_pool(pool, None);
            }
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
