use std::time::{Duration, Instant};

use ash::vk;

use crate::host::{Error, Host};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Groups {
    Fixed(u32),
    Counting(u32),
    From { binding: usize, at: usize },
}

#[derive(Debug, PartialEq, Eq)]
struct Recorded {
    stages: Vec<Vec<Groups>>,
    out: Vec<(vk::Buffer, u64)>,
    times: u32,
}

struct Held {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    bytes: u64,
    page: u64,
    pages: Vec<(u64, vk::DeviceMemory, u64)>,
    visible: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Run<'words> {
    pub index: usize,
    pub at: usize,
    pub words: &'words [u32],
}

#[derive(Debug, Clone)]
struct Flight {
    ticket: u64,
    out: Vec<usize>,
    began: Instant,
}

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

impl Ticket {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

pub const STAGES: usize = 64;

pub const SLOTS: usize = 2;

struct Slot {
    command: vk::CommandBuffer,
    fence: vk::Fence,
    staging: Held,
    queries: Option<vk::QueryPool>,
    held: Option<Recorded>,
    flight: Option<Flight>,
    stamped: usize,
}

#[derive(Clone, Copy)]
struct Facing {
    command: vk::CommandBuffer,
    fence: vk::Fence,
    queries: Option<vk::QueryPool>,
    staging: vk::Buffer,
}

pub struct Resident<'host> {
    host: &'host Host,
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
    transfer: vk::CommandBuffer,
    fence: vk::Fence,
    slots: Vec<Slot>,
    tickets: u64,
    cached: bool,
    recording: Duration,
    recorded: u64,
    copying: Duration,
    copies: u64,
    pushing: Duration,
    pushes: u64,
    poked: u64,
    patience: Duration,
    device: Option<Duration>,
    spans: Vec<Duration>,
    reused: u64,
}

impl Host {
    pub fn resident<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, false, 0)
    }

    pub fn resident_direct<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, true, 0)
    }

    pub fn resident_near<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        window: u64,
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in(sizes, modules, false, window)
    }

    pub fn resident_vast<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        vast: usize,
    ) -> Result<Resident<'host>, Error> {
        if !self.sparse() {
            return Err(Error::NoSparse);
        }
        self.buffers_in_with(sizes, modules, false, 0, Some(vast))
    }

    fn buffers_in<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        direct: bool,
        window: u64,
    ) -> Result<Resident<'host>, Error> {
        self.buffers_in_with(sizes, modules, direct, window, None)
    }

    fn buffers_in_with<'host>(
        &'host self,
        sizes: &[usize],
        modules: &[&[u32]],
        direct: bool,
        window: u64,
        vast: Option<usize>,
    ) -> Result<Resident<'host>, Error> {
        let words = |count: usize| (count.max(1) * size_of::<u32>()) as u64;
        let largest = sizes
            .iter()
            .enumerate()
            .filter(|&(at, _)| Some(at) != vast)
            .map(|(_, &count)| words(count))
            .max()
            .unwrap_or(4);

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
                | vk::BufferUsageFlags::INDIRECT_BUFFER;
            let near = vk::MemoryPropertyFlags::DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT;

            let mut buffers = Vec::with_capacity(sizes.len());
            for (at, &size) in sizes.iter().enumerate() {
                let bytes = words(size);
                if Some(at) == vast {
                    buffers.push(self.allocate_vast(bytes, usage_of)?);
                    continue;
                }

                let held = if bytes <= window {
                    match self.allocate(bytes, usage_of, near) {
                        Ok(held) => Some(Held {
                            visible: true,
                            ..held
                        }),
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

            let usage = vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST;
            let plain =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let mut staged = Vec::with_capacity(SLOTS + 1);
            let mut cached = true;
            for _ in 0..=SLOTS {
                let held = match self.allocate(
                    largest,
                    usage,
                    plain | vk::MemoryPropertyFlags::HOST_CACHED,
                ) {
                    Ok(held) => held,
                    Err(_) => {
                        cached = false;
                        self.allocate(largest, usage, plain)?
                    }
                };
                staged.push(held);
            }
            let staging = staged.pop().ok_or(Error::NoMemory { bytes: largest })?;

            let (layout, pool, set) = self.describe(&buffers)?;
            let (pipeline_layout, shaders, pipelines) = self.compile(layout, modules)?;
            let (commands, mut spare, transfer) = self.record_into()?;
            let fence = self
                .device()
                .create_fence(&vk::FenceCreateInfo::default(), None)?;

            let mut slots = Vec::with_capacity(SLOTS);
            for staging in staged {
                let command = spare.pop().unwrap_or_default();
                let fence = self
                    .device()
                    .create_fence(&vk::FenceCreateInfo::default(), None)?;
                let queries = if self.ticking().1 {
                    let create = vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(u32::try_from(STAGES + 3).unwrap_or(3));
                    self.device().create_query_pool(&create, None).ok()
                } else {
                    None
                };
                slots.push(Slot {
                    command,
                    fence,
                    staging,
                    queries,
                    held: None,
                    flight: None,
                    stamped: 0,
                });
            }

            Ok(Resident {
                spans: Vec::new(),
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
                transfer,
                fence,
                slots,
                tickets: 0,
                cached,
                patience: Resident::PATIENCE,
                reused: 0,
                device: None,
            })
        }
    }

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
        let handle = unsafe { self.device().create_buffer(&create, None) }?;

        let requirements = unsafe { self.device().get_buffer_memory_requirements(handle) };
        let Some(kind) = self.suits(requirements, wanted) else {
            unsafe { self.device().destroy_buffer(handle, None) };
            return Err(Error::NoMemory { bytes });
        };

        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(kind);
        let memory = unsafe { self.device().allocate_memory(&allocate, None) }?;
        unsafe { self.device().bind_buffer_memory(handle, memory, 0) }?;

        Ok(Held {
            handle,
            memory,
            bytes: bytes.max(4),
            page: 0,
            pages: Vec::new(),
            visible: wanted.contains(vk::MemoryPropertyFlags::HOST_VISIBLE),
        })
    }

    unsafe fn allocate_vast(&self, bytes: u64, usage: vk::BufferUsageFlags) -> Result<Held, Error> {
        let create = vk::BufferCreateInfo::default()
            .size(bytes.max(4))
            .usage(usage)
            .flags(vk::BufferCreateFlags::SPARSE_BINDING | vk::BufferCreateFlags::SPARSE_RESIDENCY)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let handle = unsafe { self.device().create_buffer(&create, None) }?;
        let requirements = unsafe { self.device().get_buffer_memory_requirements(handle) };
        Ok(Held {
            handle,
            memory: vk::DeviceMemory::null(),
            bytes: bytes.max(4),
            page: requirements.alignment.max(1),
            pages: Vec::new(),
            visible: false,
        })
    }

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
        let layout = unsafe { self.device().create_descriptor_set_layout(&create, None) }?;

        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(count)];
        let create = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes);
        let pool = unsafe { self.device().create_descriptor_pool(&create, None) }?;

        let layouts = [layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let sets = unsafe { self.device().allocate_descriptor_sets(&allocate) }?;
        let set = sets.first().copied().unwrap_or_default();

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
        unsafe { self.device().update_descriptor_sets(&writes, &[]) };

        Ok((layout, pool, set))
    }

    unsafe fn compile(
        &self,
        layout: vk::DescriptorSetLayout,
        modules: &[&[u32]],
    ) -> Result<(vk::PipelineLayout, Vec<vk::ShaderModule>, Vec<vk::Pipeline>), Error> {
        let layouts = [layout];
        let create = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
        let pipeline_layout = unsafe { self.device().create_pipeline_layout(&create, None) }?;

        let mut shaders = Vec::with_capacity(modules.len());
        let mut pipelines = Vec::with_capacity(modules.len());
        let entry = c"main";

        for words in modules {
            let create = vk::ShaderModuleCreateInfo::default().code(words);
            let shader = unsafe { self.device().create_shader_module(&create, None) }?;
            shaders.push(shader);

            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(entry);
            let create = [vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(pipeline_layout)];
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

    unsafe fn record_into(
        &self,
    ) -> Result<(vk::CommandPool, Vec<vk::CommandBuffer>, vk::CommandBuffer), Error> {
        let create = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.family())
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let pool = unsafe { self.device().create_command_pool(&create, None) }?;

        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(u32::try_from(SLOTS + 1).unwrap_or(2));
        let mut buffers = unsafe { self.device().allocate_command_buffers(&allocate) }?;
        let transfer = buffers.pop().unwrap_or_default();

        Ok((pool, buffers, transfer))
    }
}

impl Resident<'_> {
    pub const PATIENCE: Duration = Duration::from_secs(10);

    pub const fn patience(&self) -> Duration {
        self.patience
    }

    pub fn set_patience(&mut self, patience: Duration) {
        self.patience = patience;
    }

    fn live(&self) -> Result<(), Error> {
        if self.host.wedged() {
            return Err(Error::Wedged {
                after: self.patience,
            });
        }
        Ok(())
    }

    fn await_fence(&self, fence: vk::Fence) -> Result<(), Error> {
        let patience = u64::try_from(self.patience.as_nanos()).unwrap_or(u64::MAX);
        let waited = unsafe { self.host.device().wait_for_fences(&[fence], true, patience) };
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

    #[must_use]
    pub fn bindings(&self) -> usize {
        self.buffers.len()
    }

    #[must_use]
    pub fn passes(&self) -> usize {
        self.pipelines.len()
    }

    #[must_use]
    pub const fn pushing(&self) -> (Duration, u64, u64) {
        (self.pushing, self.pushes, self.poked)
    }

    pub fn write(&mut self, index: usize, words: &[u32]) -> Result<(), Error> {
        self.live()?;
        let began = std::time::Instant::now();
        let out = self.written(index, words);
        self.pushing += began.elapsed();
        self.pushes += 1;
        self.poked += u64::from(*out.as_ref().unwrap_or(&false));
        out.map(|_| ())
    }

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
            return unsafe { self.poke(held.memory, 0, words) }.map(|()| true);
        }
        if bytes > self.staging.bytes {
            return Err(Error::Staged {
                needed: words.len(),
                staging: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }
        let (handle, staging) = (held.handle, self.staging.memory);

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
        let from = self.staging.handle;
        unsafe {
            self.once(|device, command| device.cmd_copy_buffer(command, from, handle, &region))
                .map(|()| false)
        }
    }

    unsafe fn poke(
        &self,
        memory: vk::DeviceMemory,
        offset: u64,
        words: &[u32],
    ) -> Result<(), Error> {
        let bytes = (size_of_val(words) as u64).max(4);
        unsafe {
            let mapped = self.host.device().map_memory(
                memory,
                offset,
                bytes,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast::<u32>(), words.len());
            self.host.device().unmap_memory(memory);
        }
        Ok(())
    }

    #[must_use]
    pub const fn cached(&self) -> bool {
        self.cached
    }

    pub const fn direct(&self) -> bool {
        self.direct
    }

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
            return unsafe { self.poke(held.memory, offset, words) };
        }
        if bytes > self.staging.bytes {
            return Err(Error::Staged {
                needed: words.len(),
                staging: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }
        let (handle, staging) = (held.handle, self.staging.memory);

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
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, from, handle, &region);
            })
        }
    }

    pub fn write_many(&mut self, runs: &[Run<'_>]) -> Result<(), Error> {
        self.live()?;
        if runs.is_empty() {
            return Ok(());
        }

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
            return Err(Error::Staged {
                needed: (total as usize) / size_of::<u32>(),
                staging: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }

        let staging = self.staging.memory;
        unsafe {
            let mapped = self.host.device().map_memory(
                staging,
                0,
                total.max(4),
                vk::MemoryMapFlags::empty(),
            )?;
            let mut along = 0_usize;
            for run in runs {
                let into = mapped.cast::<u32>().add(along);
                std::ptr::copy_nonoverlapping(run.words.as_ptr(), into, run.words.len());
                along += run.words.len();
            }
            self.host.device().unmap_memory(staging);
        }

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

        unsafe {
            self.once(|device, command| {
                for (handle, regions) in &grouped {
                    device.cmd_copy_buffer(command, source, *handle, regions);
                }
            })
        }
    }

    #[must_use]
    pub const fn copying(&self) -> (Duration, u64) {
        (self.copying, self.copies)
    }

    #[must_use]
    pub fn page(&self, index: usize) -> Option<usize> {
        let held = self.buffers.get(index)?;
        (held.page > 0).then_some((held.page / 4) as usize)
    }

    #[must_use]
    pub fn bound(&self, index: usize) -> usize {
        self.buffers.get(index).map_or(0, |held| {
            held.pages.iter().map(|&(_, _, bytes)| bytes).sum::<u64>() as usize / 4
        })
    }

    pub fn bind(&mut self, index: usize, at: usize, words: usize) -> Result<(), Error> {
        let page = self.buffers.get(index).map_or(0, |held| held.page);
        if page == 0 {
            return match self.buffers.get(index) {
                Some(_) => Err(Error::NoSparse),
                None => Err(Error::NoSuchBinding {
                    index,
                    held: self.buffers.len(),
                }),
            };
        }
        let (from, bytes) = (
            (at * size_of::<u32>()) as u64,
            (words * size_of::<u32>()) as u64,
        );
        let end = from.saturating_add(bytes);
        let held = self.buffers.get(index).ok_or(Error::NoSuchBinding {
            index,
            held: self.buffers.len(),
        })?;
        if from % page != 0 || bytes % page != 0 || end > held.bytes {
            return Err(Error::Overrun {
                index,
                needed: (end / size_of::<u32>() as u64) as usize,
                held: (held.bytes / size_of::<u32>() as u64) as usize,
            });
        }
        if bytes == 0 {
            return Ok(());
        }

        self.unbind(index, at, words)?;

        unsafe {
            let device = self.host.device();
            let held = self.buffers.get(index).ok_or(Error::NoSuchBinding {
                index,
                held: self.buffers.len(),
            })?;
            let requirements = device.get_buffer_memory_requirements(held.handle);
            let kind = self
                .host
                .suits(requirements, vk::MemoryPropertyFlags::DEVICE_LOCAL)
                .ok_or(Error::NoMemory { bytes })?;
            let allocate = vk::MemoryAllocateInfo::default()
                .allocation_size(bytes)
                .memory_type_index(kind);
            let memory = device.allocate_memory(&allocate, None)?;

            let binds = [vk::SparseMemoryBind::default()
                .resource_offset(from)
                .size(bytes)
                .memory(memory)
                .memory_offset(0)];
            let one = [vk::SparseBufferMemoryBindInfo::default()
                .buffer(held.handle)
                .binds(&binds)];
            let info = [vk::BindSparseInfo::default().buffer_binds(&one)];

            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let held = device.queue_bind_sparse(self.host.queue(), &info, fence);
            if held.is_err() {
                device.destroy_fence(fence, None);
                device.free_memory(memory, None);
                held?;
            }
            let waited = device.wait_for_fences(&[fence], true, u64::MAX);
            device.destroy_fence(fence, None);
            waited?;

            if let Some(slot) = self.buffers.get_mut(index) {
                slot.pages.push((from, memory, bytes));
            }
        }
        Ok(())
    }

    pub fn unbind(&mut self, index: usize, at: usize, words: usize) -> Result<(), Error> {
        let (from, bytes) = (
            (at * size_of::<u32>()) as u64,
            (words * size_of::<u32>()) as u64,
        );
        let end = from.saturating_add(bytes);
        let Some(held) = self.buffers.get(index) else {
            return Err(Error::NoSuchBinding {
                index,
                held: self.buffers.len(),
            });
        };
        let going: Vec<(u64, vk::DeviceMemory, u64)> = held
            .pages
            .iter()
            .copied()
            .filter(|&(off, _, size)| off >= from && off + size <= end)
            .collect();
        if going.is_empty() {
            return Ok(());
        }

        unsafe {
            let device = self.host.device();
            let handle = held.handle;
            let binds: Vec<vk::SparseMemoryBind> = going
                .iter()
                .map(|&(off, _, size)| {
                    vk::SparseMemoryBind::default()
                        .resource_offset(off)
                        .size(size)
                        .memory(vk::DeviceMemory::null())
                        .memory_offset(0)
                })
                .collect();
            let one = [vk::SparseBufferMemoryBindInfo::default()
                .buffer(handle)
                .binds(&binds)];
            let info = [vk::BindSparseInfo::default().buffer_binds(&one)];
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            let sent = device.queue_bind_sparse(self.host.queue(), &info, fence);
            if sent.is_err() {
                device.destroy_fence(fence, None);
                sent?;
            }
            let waited = device.wait_for_fences(&[fence], true, u64::MAX);
            device.destroy_fence(fence, None);
            waited?;

            for &(_, memory, _) in &going {
                device.free_memory(memory, None);
            }
            if let Some(slot) = self.buffers.get_mut(index) {
                slot.pages
                    .retain(|&(off, _, size)| !(off >= from && off + size <= end));
            }
        }
        Ok(())
    }

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
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, handle, into, &region);
            })
        }?;
        self.copying += began.elapsed();
        self.copies += 1;
        let memory = self.staging.memory;
        unsafe { self.mapped_at(memory, 0, count) }
    }

    pub fn frame(&mut self, groups: &[Groups]) -> Result<Duration, Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        self.staged(&stages)
    }

    pub fn staged(&mut self, stages: &[&[Groups]]) -> Result<Duration, Error> {
        let ticket = self.submit(stages)?;
        self.wait(ticket)
    }

    pub fn submit(&mut self, stages: &[&[Groups]]) -> Result<Ticket, Error> {
        self.start(stages, &[], 1)
    }

    pub fn submit_reading(
        &mut self,
        stages: &[&[Groups]],
        index: usize,
        count: usize,
    ) -> Result<Ticket, Error> {
        self.bounded(index, count)?;
        self.start(stages, &[(index, count)], 1)
    }

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

    pub fn staged_reading_both(
        &mut self,
        stages: &[&[Groups]],
        first: (usize, usize),
        second: (usize, usize),
    ) -> Result<(Duration, Vec<u32>, Vec<u32>), Error> {
        let ticket = self.submit_reading_both(stages, first, second)?;
        self.wait_reading_both(ticket)
    }

    pub fn wait(&mut self, ticket: Ticket) -> Result<Duration, Error> {
        self.land(ticket).map(|(spent, _, _)| spent)
    }

    pub fn wait_reading(&mut self, ticket: Ticket) -> Result<(Duration, Vec<u32>), Error> {
        let (spent, mut every) = self.landed(ticket)?;
        Ok((
            spent,
            if every.is_empty() {
                Vec::new()
            } else {
                every.swap_remove(0)
            },
        ))
    }

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

    fn landed(&mut self, ticket: Ticket) -> Result<(Duration, Vec<Vec<u32>>), Error> {
        let (spent, counts, slot) = self.land(ticket)?;
        let Some(memory) = self.slots.get(slot).map(|slot| slot.staging.memory) else {
            return Err(Error::NoSuchTicket {
                ticket: ticket.get(),
                held: None,
            });
        };
        let mut every = Vec::with_capacity(counts.len());
        let mut at = 0_usize;
        for count in counts {
            every.push(unsafe { self.mapped_at(memory, at, count) }?);
            at += count.max(1);
        }
        Ok((spent, every))
    }

    pub fn frame_reading(
        &mut self,
        groups: &[Groups],
        index: usize,
        count: usize,
    ) -> Result<(Duration, Vec<u32>), Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        self.staged_reading(&stages, index, count)
    }

    pub fn staged_reading(
        &mut self,
        stages: &[&[Groups]],
        index: usize,
        count: usize,
    ) -> Result<(Duration, Vec<u32>), Error> {
        let ticket = self.submit_reading(stages, index, count)?;
        self.wait_reading(ticket)
    }

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

    fn at(&self, index: usize) -> Result<&Held, Error> {
        self.buffers.get(index).ok_or(Error::NoSuchBinding {
            index,
            held: self.buffers.len(),
        })
    }

    fn start(
        &mut self,
        stages: &[&[Groups]],
        out: &[(usize, usize)],
        times: u32,
    ) -> Result<Ticket, Error> {
        self.live()?;
        let Some((at, facing)) = self.free() else {
            return Err(Error::StillInFlight);
        };
        let mut copy = Vec::with_capacity(out.len());
        let mut wanted = 0_u64;
        for &(index, count) in out {
            let held = self.at(index)?;
            let bytes = (count * size_of::<u32>()) as u64;
            copy.push((held.handle, bytes));
            wanted += bytes.max(4);
        }
        if wanted > self.staging.bytes {
            return Err(Error::Overrun {
                index: out.len(),
                needed: (wanted as usize) / size_of::<u32>(),
                held: (self.staging.bytes as usize) / size_of::<u32>(),
            });
        }

        let began = Instant::now();
        self.run_times(at, facing, stages, &copy, times)?;
        self.tickets = self.tickets.wrapping_add(1);
        let flight = Flight {
            ticket: self.tickets,
            out: out.iter().map(|&(_, count)| count).collect(),
            began,
        };
        if let Some(slot) = self.slots.get_mut(at) {
            slot.flight = Some(flight);
        }
        Ok(Ticket(self.tickets))
    }

    fn free(&self) -> Option<(usize, Facing)> {
        self.slots
            .iter()
            .enumerate()
            .find(|(_, slot)| slot.flight.is_none())
            .map(|(at, slot)| {
                (
                    at,
                    Facing {
                        command: slot.command,
                        fence: slot.fence,
                        queries: slot.queries,
                        staging: slot.staging.handle,
                    },
                )
            })
    }

    fn flying(&self, ticket: Ticket) -> Option<usize> {
        self.slots.iter().position(|slot| {
            slot.flight
                .as_ref()
                .is_some_and(|held| held.ticket == ticket.get())
        })
    }

    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.flight.is_some())
            .count()
    }

    fn land(&mut self, ticket: Ticket) -> Result<(Duration, Vec<usize>, usize), Error> {
        self.live()?;
        let Some(at) = self.flying(ticket) else {
            let held = self
                .slots
                .iter()
                .find_map(|slot| slot.flight.as_ref())
                .map(|held| held.ticket);
            return Err(Error::NoSuchTicket {
                ticket: ticket.get(),
                held,
            });
        };
        let (began, out, fence) = {
            let Some(slot) = self.slots.get(at) else {
                return Err(Error::NoSuchTicket {
                    ticket: ticket.get(),
                    held: None,
                });
            };
            let Some(held) = slot.flight.as_ref() else {
                return Err(Error::NoSuchTicket {
                    ticket: ticket.get(),
                    held: None,
                });
            };
            (held.began, held.out.clone(), slot.fence)
        };

        self.await_fence(fence)?;
        if let Some(slot) = self.slots.get_mut(at) {
            slot.flight = None;
        }
        self.device = self.timed(at);
        self.spans = self.split(at);
        Ok((began.elapsed(), out, at))
    }

    pub fn frames(&mut self, groups: &[Groups], times: u32) -> Result<Duration, Error> {
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        self.staged_frames(&stages, times)
    }

    pub fn staged_frames(&mut self, stages: &[&[Groups]], times: u32) -> Result<Duration, Error> {
        let ticket = self.start(stages, &[], times.max(1))?;
        self.wait(ticket).map(|spent| spent / times.max(1))
    }

    #[must_use]
    pub const fn recording(&self) -> (Duration, u64) {
        (self.recording, self.recorded)
    }

    #[must_use]
    pub const fn reused(&self) -> u64 {
        self.reused
    }

    #[must_use]
    pub const fn on_device(&self) -> Option<Duration> {
        self.device
    }

    #[must_use]
    pub fn stage_times(&self) -> &[Duration] {
        &self.spans
    }

    fn split(&self, at: usize) -> Vec<Duration> {
        let Some(slot) = self.slots.get(at) else {
            return Vec::new();
        };
        let Some(pool) = slot.queries else {
            return Vec::new();
        };
        let stages = slot.stamped;
        if stages == 0 || stages > STAGES {
            return Vec::new();
        }
        let mut stamps = vec![0_u64; stages + 1];
        let held = unsafe {
            self.host.device().get_query_pool_results(
                pool,
                2,
                &mut stamps,
                vk::QueryResultFlags::TYPE_64,
            )
        };
        if held.is_err() {
            return Vec::new();
        }
        let (tick, _) = self.host.ticking();
        stamps
            .windows(2)
            .map(|two| {
                let (first, next) = (two.first().copied(), two.get(1).copied());
                let ticks = next.unwrap_or(0).saturating_sub(first.unwrap_or(0));
                Duration::from_nanos((ticks as f64 * f64::from(tick)) as u64)
            })
            .collect()
    }

    fn timed(&self, at: usize) -> Option<Duration> {
        let slot = self.slots.get(at)?;
        let pool = slot.queries?;
        let times = slot.held.as_ref().map_or(1, |held| held.times).max(1);
        let mut stamps = [0_u64; 2];
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

    fn run_times(
        &mut self,
        at: usize,
        facing: Facing,
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

        let commands = [facing.command];
        let submit = [vk::SubmitInfo::default().command_buffers(&commands)];

        let wanted = Recorded {
            stages: stages.iter().map(|one| one.to_vec()).collect(),
            out: out.to_vec(),
            times,
        };
        if self.slots.get(at).and_then(|slot| slot.held.as_ref()) == Some(&wanted) {
            unsafe {
                device.reset_fences(&[facing.fence])?;
                device.queue_submit(self.host.queue(), &submit, facing.fence)?;
            }
            self.reused += 1;
            return Ok(());
        }

        if let Some(slot) = self.slots.get_mut(at) {
            slot.held = None;
        }

        unsafe {
            device.reset_command_buffer(facing.command, vk::CommandBufferResetFlags::empty())?;
            let begin = vk::CommandBufferBeginInfo::default();
            device.begin_command_buffer(facing.command, &begin)?;

            if let Some(pool) = facing.queries {
                let held = u32::try_from(STAGES + 3).unwrap_or(3);
                device.cmd_reset_query_pool(facing.command, pool, 0, held);
                device.cmd_write_timestamp(
                    facing.command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    pool,
                    0,
                );
            }

            device.cmd_bind_descriptor_sets(
                facing.command,
                vk::PipelineBindPoint::COMPUTE,
                self.pipeline_layout,
                0,
                &[self.set],
                &[],
            );

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

            let profiling = facing
                .queries
                .filter(|_| running.len() <= STAGES)
                .map(|pool| (pool, times.saturating_sub(1)));
            if let Some(slot) = self.slots.get_mut(at) {
                slot.stamped = profiling.map_or(0, |_| running.len());
            }

            for repeat in 0..times {
                for (nth, stage) in running.iter().enumerate() {
                    if let Some((pool, last)) = profiling
                        && repeat == last
                    {
                        {
                            let at = u32::try_from(nth + 2).unwrap_or(2);
                            device.cmd_write_timestamp(
                                facing.command,
                                vk::PipelineStageFlags::TOP_OF_PIPE,
                                pool,
                                at,
                            );
                        }
                    }
                    for &(pass, count) in stage {
                        let &pipeline = self.pipelines.get(pass).ok_or(Error::WrongPasses {
                            given: pass,
                            held: self.pipelines.len(),
                        })?;
                        device.cmd_bind_pipeline(
                            facing.command,
                            vk::PipelineBindPoint::COMPUTE,
                            pipeline,
                        );
                        match count {
                            Groups::Fixed(count) | Groups::Counting(count) => {
                                device.cmd_dispatch(facing.command, count, 1, 1);
                            }
                            Groups::From { binding, at } => {
                                let held =
                                    self.buffers.get(binding).ok_or(Error::NoSuchBinding {
                                        index: binding,
                                        held: self.buffers.len(),
                                    })?;
                                let offset = (at * size_of::<u32>()) as u64;
                                device.cmd_dispatch_indirect(facing.command, held.handle, offset);
                            }
                        }
                    }

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
                            (vk::AccessFlags::empty(), vk::PipelineStageFlags::empty())
                        };
                        let barrier = [vk::MemoryBarrier::default()
                            .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                            .dst_access_mask(
                                vk::AccessFlags::SHADER_READ
                                    | vk::AccessFlags::SHADER_WRITE
                                    | reach,
                            )];
                        device.cmd_pipeline_barrier(
                            facing.command,
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

            if let Some((pool, _)) = profiling {
                let at = u32::try_from(running.len() + 2).unwrap_or(2);
                device.cmd_write_timestamp(
                    facing.command,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    pool,
                    at,
                );
            }

            let mut into = 0_u64;
            for &(handle, bytes) in out {
                let barrier = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)];
                device.cmd_pipeline_barrier(
                    facing.command,
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
                device.cmd_copy_buffer(facing.command, handle, facing.staging, &region);
                into += bytes.max(4);

                let barrier = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)];
                device.cmd_pipeline_barrier(
                    facing.command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::HOST,
                    vk::DependencyFlags::empty(),
                    &barrier,
                    &[],
                    &[],
                );
            }

            if let Some(pool) = facing.queries {
                device.cmd_write_timestamp(
                    facing.command,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    pool,
                    1,
                );
            }

            device.end_command_buffer(facing.command)?;
            took = began.elapsed();

            device.reset_fences(&[facing.fence])?;
            device.queue_submit(self.host.queue(), &submit, facing.fence)?;
        }

        if let Some(slot) = self.slots.get_mut(at) {
            slot.held = Some(wanted);
        }
        self.recording += took;
        self.recorded += 1;

        Ok(())
    }

    unsafe fn once<F>(&self, record: F) -> Result<(), Error>
    where
        F: FnOnce(&ash::Device, vk::CommandBuffer),
    {
        self.live()?;
        if self.slots.iter().any(|slot| slot.flight.is_some()) {
            return Err(Error::StillInFlight);
        }
        let device = self.host.device();
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
        self.await_fence(self.fence)
    }

    unsafe fn mapped_at(
        &self,
        memory: vk::DeviceMemory,
        at: usize,
        count: usize,
    ) -> Result<Vec<u32>, Error> {
        let from = (at * size_of::<u32>()) as u64;
        let bytes = ((count * size_of::<u32>()) as u64).max(4);
        unsafe {
            let mapped =
                self.host
                    .device()
                    .map_memory(memory, from, bytes, vk::MemoryMapFlags::empty())?;
            let mut out = vec![0_u32; count];
            std::ptr::copy_nonoverlapping(mapped.cast::<u32>(), out.as_mut_ptr(), count);
            self.host.device().unmap_memory(memory);
            Ok(out)
        }
    }
}

impl Drop for Resident<'_> {
    fn drop(&mut self) {
        if self.host.wedged() {
            return;
        }
        let device = self.host.device();
        unsafe {
            let _ = device.device_wait_idle();
            device.destroy_fence(self.fence, None);
            for slot in &self.slots {
                device.destroy_fence(slot.fence, None);
                if let Some(pool) = slot.queries {
                    device.destroy_query_pool(pool, None);
                }
            }
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
            let staged = self.slots.iter().map(|slot| &slot.staging);
            for held in self
                .buffers
                .iter()
                .chain(std::iter::once(&self.staging))
                .chain(staged)
            {
                device.destroy_buffer(held.handle, None);
                device.free_memory(held.memory, None);
                for &(_, memory, _) in &held.pages {
                    device.free_memory(memory, None);
                }
            }
        }
    }
}
