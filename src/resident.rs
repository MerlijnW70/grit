use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ash::vk;

use crate::host::{Error, Host};
use crate::written::{Written, written};

/// Counts every object resident construction makes and frees, and the
/// host's device, both as this crate calls create and destroy and as the
/// driver allocates and frees host memory through the callbacks passed with
/// them, and fails a chosen
/// construction step on demand, so the failure paths can be tested without
/// exhausting a device. It exists only with the `faults` feature; without it
/// the same calls are empty, the callbacks are none, and nothing of it is
/// compiled.
///
/// The counts, the callbacks and the armed step belong to the calling thread.
#[cfg(feature = "faults")]
pub mod faults {
    use std::alloc::{Layout, alloc, dealloc};
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

    use ash::vk;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Step {
        Buffer,
        Memory,
        Bind,
        SetLayout,
        Pool,
        Set,
        PipelineLayout,
        Shader,
        Pipeline,
        Commands,
        CommandBuffers,
        Fence,
        Queries,
        Device,
    }

    /// Every kind counted: the construction steps and the host's device.
    const KINDS: usize = 14;

    impl Step {
        /// The steps of resident construction. The device is made by the host
        /// and lives longer than any resident, so it is counted apart.
        pub const ALL: [Self; 13] = [
            Self::Buffer,
            Self::Memory,
            Self::Bind,
            Self::SetLayout,
            Self::Pool,
            Self::Set,
            Self::PipelineLayout,
            Self::Shader,
            Self::Pipeline,
            Self::Commands,
            Self::CommandBuffers,
            Self::Fence,
            Self::Queries,
        ];

        const fn slot(self) -> usize {
            self as usize
        }
    }

    /// What construction did with one kind of object. `made` and `gone`
    /// count the creates and destroys this crate called; `allocations` and
    /// `outstanding` count the host memory the driver itself took and has
    /// not given back through the allocation callbacks passed with them, so
    /// they move only when the driver really destroys an object.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Count {
        pub made: u64,
        pub gone: u64,
        pub allocations: u64,
        pub outstanding: i64,
    }

    #[derive(Default)]
    struct Tally {
        allocations: AtomicU64,
        outstanding: AtomicI64,
    }

    struct Hooks {
        tallies: [Tally; KINDS],
        callbacks: [vk::AllocationCallbacks<'static>; KINDS],
    }

    thread_local! {
        static ARMED: Cell<Option<(Step, u32)>> = const { Cell::new(None) };
        static FIRED: Cell<bool> = const { Cell::new(false) };
        static COUNTS: RefCell<[(u64, u64); KINDS]> = const { RefCell::new([(0, 0); KINDS]) };
        static HOOKS: &'static Hooks = hooks();
    }

    fn hooks() -> &'static Hooks {
        let held: &'static mut Hooks = Box::leak(Box::new(Hooks {
            tallies: Default::default(),
            callbacks: [vk::AllocationCallbacks::default(); KINDS],
        }));
        for (callbacks, tally) in held.callbacks.iter_mut().zip(held.tallies.iter()) {
            *callbacks = vk::AllocationCallbacks::default()
                .user_data(std::ptr::from_ref(tally).cast_mut().cast())
                .pfn_allocation(Some(allocation))
                .pfn_reallocation(Some(reallocation))
                .pfn_free(Some(free));
        }
        held
    }

    /// The callbacks every create and destroy of `step` passes, so the
    /// driver's own host allocations for those objects are counted.
    pub(crate) fn callbacks(step: Step) -> Option<&'static vk::AllocationCallbacks<'static>> {
        HOOKS.with(|hooks| hooks.callbacks.get(step.slot()))
    }

    const HEAD: usize = 2 * size_of::<usize>();

    fn shape(size: usize, alignment: usize) -> Option<(Layout, usize)> {
        let alignment = alignment.max(HEAD).checked_next_power_of_two()?;
        let whole = size.checked_add(alignment)?;
        Layout::from_size_align(whole, alignment)
            .ok()
            .map(|layout| (layout, alignment))
    }

    unsafe extern "system" fn allocation(
        user: *mut c_void,
        size: usize,
        alignment: usize,
        _: vk::SystemAllocationScope,
    ) -> *mut c_void {
        let Some((layout, alignment)) = shape(size, alignment) else {
            return std::ptr::null_mut();
        };
        let base = unsafe { alloc(layout) };
        if base.is_null() {
            return base.cast();
        }
        unsafe {
            let given = base.add(alignment);
            given
                .sub(HEAD)
                .cast::<[usize; 2]>()
                .write([size, alignment]);
            if let Some(tally) = user.cast::<Tally>().cast_const().as_ref() {
                tally.allocations.fetch_add(1, Ordering::Relaxed);
                tally.outstanding.fetch_add(1, Ordering::Relaxed);
            }
            given.cast()
        }
    }

    unsafe extern "system" fn free(user: *mut c_void, memory: *mut c_void) {
        if memory.is_null() {
            return;
        }
        unsafe {
            let given = memory.cast::<u8>();
            let [size, alignment] = given.sub(HEAD).cast::<[usize; 2]>().read();
            if let Some((layout, _)) = shape(size, alignment) {
                dealloc(given.sub(alignment), layout);
            }
            if let Some(tally) = user.cast::<Tally>().cast_const().as_ref() {
                tally.outstanding.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    unsafe extern "system" fn reallocation(
        user: *mut c_void,
        original: *mut c_void,
        size: usize,
        alignment: usize,
        scope: vk::SystemAllocationScope,
    ) -> *mut c_void {
        unsafe {
            if original.is_null() {
                return allocation(user, size, alignment, scope);
            }
            if size == 0 {
                free(user, original);
                return std::ptr::null_mut();
            }
            let moved = allocation(user, size, alignment, scope);
            if moved.is_null() {
                return moved;
            }
            let [held, _] = original.cast::<u8>().sub(HEAD).cast::<[usize; 2]>().read();
            std::ptr::copy_nonoverlapping(
                original.cast::<u8>(),
                moved.cast::<u8>(),
                held.min(size),
            );
            free(user, original);
            moved
        }
    }

    /// Fails the `nth` time, counting from nought, that `step` is reached.
    pub fn arm(step: Step, nth: u32) {
        ARMED.set(Some((step, nth)));
        FIRED.set(false);
    }

    pub fn disarm() {
        ARMED.set(None);
    }

    /// Whether the armed step was reached and failed since it was armed.
    #[must_use]
    pub fn fired() -> bool {
        FIRED.get()
    }

    pub fn reset() {
        disarm();
        FIRED.set(false);
        COUNTS.with_borrow_mut(|counts| *counts = [(0, 0); KINDS]);
        HOOKS.with(|hooks| {
            for tally in &hooks.tallies {
                tally.allocations.store(0, Ordering::Relaxed);
                tally.outstanding.store(0, Ordering::Relaxed);
            }
        });
    }

    #[must_use]
    pub fn count(step: Step) -> Count {
        let (made, gone) =
            COUNTS.with_borrow(|counts| counts.get(step.slot()).copied().unwrap_or_default());
        let (allocations, outstanding) = HOOKS.with(|hooks| {
            hooks.tallies.get(step.slot()).map_or((0, 0), |tally| {
                (
                    tally.allocations.load(Ordering::Relaxed),
                    tally.outstanding.load(Ordering::Relaxed),
                )
            })
        });
        Count {
            made,
            gone,
            allocations,
            outstanding,
        }
    }

    pub(crate) fn enter(step: Step) -> Result<(), vk::Result> {
        let Some((armed, nth)) = ARMED.get() else {
            return Ok(());
        };
        if armed != step {
            return Ok(());
        }
        if nth > 0 {
            ARMED.set(Some((armed, nth - 1)));
            return Ok(());
        }
        ARMED.set(None);
        FIRED.set(true);
        Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)
    }

    fn tally(step: Step, handle: impl vk::Handle, made: bool) {
        if handle.as_raw() == 0 {
            return;
        }
        COUNTS.with_borrow_mut(|counts| {
            if let Some((creates, destroys)) = counts.get_mut(step.slot()) {
                if made {
                    *creates += 1;
                } else {
                    *destroys += 1;
                }
            }
        });
    }

    pub(crate) fn made(step: Step, handle: impl vk::Handle) {
        tally(step, handle, true);
    }

    pub(crate) fn gone(step: Step, handle: impl vk::Handle) {
        tally(step, handle, false);
    }
}

#[cfg(not(feature = "faults"))]
pub(crate) mod faults {
    use ash::vk;

    #[derive(Clone, Copy)]
    pub(crate) enum Step {
        Buffer,
        Memory,
        Bind,
        SetLayout,
        Pool,
        Set,
        PipelineLayout,
        Shader,
        Pipeline,
        Commands,
        CommandBuffers,
        Fence,
        Queries,
        Device,
    }

    #[inline(always)]
    pub(crate) const fn enter(_: Step) -> Result<(), vk::Result> {
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn made(_: Step, _: impl vk::Handle) {}

    #[inline(always)]
    pub(crate) fn gone(_: Step, _: impl vk::Handle) {}

    #[inline(always)]
    pub(crate) const fn callbacks(_: Step) -> Option<&'static vk::AllocationCallbacks<'static>> {
        None
    }
}

use faults::Step;

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
    kind: vk::MemoryPropertyFlags,
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
    room: u64,
}

const WORD: u64 = size_of::<u32>() as u64;

const INDIRECT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    from: u64,
    bytes: u64,
    end: u64,
}

fn span(
    at: usize,
    count: usize,
    limit: u64,
    refused: impl FnOnce(usize) -> Error,
) -> Result<Span, Error> {
    let bytes_of = |words: usize| u64::try_from(words).ok()?.checked_mul(WORD);
    let found = bytes_of(at).and_then(|from| {
        let bytes = bytes_of(count)?;
        let end = from.checked_add(bytes)?;
        (from < limit && end <= limit).then_some(Span { from, bytes, end })
    });
    found.ok_or_else(|| refused(at.saturating_add(count)))
}

fn words_in(bytes: u64) -> usize {
    usize::try_from(bytes / WORD).unwrap_or(usize::MAX)
}

fn staged(at: usize, count: usize, room: u64) -> Result<Span, Error> {
    span(at, count, room, |needed| Error::Staged {
        needed,
        staging: words_in(room),
    })
}

/// Lets one thread of the process at a time open a host or build kernels.
///
/// lavapipe keeps the vector width it compiles for in one variable of the
/// process. Every compile first sets it to the driver's default and only then
/// to `LP_NATIVE_VECTOR_WIDTH`, and opening a device and compiling a kernel
/// both read it. A host opened while another thread compiles can so report a
/// subgroup its kernels never run at, and a kernel compiled while another
/// thread compiles can run a subgroup narrower than the host reports; either
/// way every subgroup sum it builds is wrong. A host is opened, and a resident
/// is built, under this lock, and each pipeline of the resident is dispatched
/// once with no group before it is handed out, so a driver that compiles at
/// the first dispatch compiles then.
static FORMING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What [`crate::Host`] holds while it opens a device, and a resident while
/// it is built.
pub(crate) fn forming() -> std::sync::MutexGuard<'static, ()> {
    FORMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Resident<'host> {
    host: &'host Host,
    direct: bool,
    built: bool,
    buffers: Vec<Held>,
    staging: Held,
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    pipeline_layout: vk::PipelineLayout,
    shaders: Vec<vk::ShaderModule>,
    pipelines: Vec<vk::Pipeline>,
    writes: Vec<Written>,
    commands: vk::CommandPool,
    transfer: vk::CommandBuffer,
    fence: vk::Fence,
    sending: AtomicBool,
    stranded: Vec<(vk::Fence, vk::DeviceMemory)>,
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
        self.limits().admits(sizes, vast)?;
        let mut lengths = Vec::with_capacity(sizes.len());
        for &count in sizes {
            let counted = span(0, count.max(1), u64::MAX, |_| Error::NoMemory {
                bytes: u64::MAX,
            })?;
            lengths.push(counted.bytes);
        }
        let largest = lengths
            .iter()
            .enumerate()
            .filter(|&(at, _)| Some(at) != vast)
            .map(|(_, &bytes)| bytes)
            .max()
            .unwrap_or(WORD);

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

        let _forming = forming();
        let mut made = Resident::building(self, direct, sizes.len());
        unsafe {
            for (at, &bytes) in lengths.iter().enumerate() {
                if Some(at) == vast {
                    made.buffers.push(self.allocate_vast(bytes, usage_of)?);
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
                made.buffers.push(match held {
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
            for at in 0..=SLOTS {
                let held = match self.allocate(
                    largest,
                    usage,
                    plain | vk::MemoryPropertyFlags::HOST_CACHED,
                ) {
                    Ok(held) => held,
                    Err(_) => self.allocate(largest, usage, plain)?,
                };
                if at < SLOTS {
                    made.slots.push(Slot::staged(held));
                } else {
                    made.staging = held;
                }
            }
            made.cached = made
                .slots
                .iter()
                .map(|slot| &slot.staging)
                .chain(std::iter::once(&made.staging))
                .all(|held| held.kind.contains(vk::MemoryPropertyFlags::HOST_CACHED));

            made.describe()?;
            made.compile(modules)?;
            made.writes = modules.iter().map(|words| written(words)).collect();
            made.record()?;
            made.fence = self.fence()?;
            for slot in &mut made.slots {
                slot.fence = self.fence()?;
                slot.queries = self.queries();
            }
        }
        made.built = true;
        self.owe();
        unsafe { made.warm() }?;
        Ok(made)
    }

    /// A buffer bound to fresh memory of the kind `wanted`, or an error with
    /// nothing left made: a buffer whose memory cannot be had or bound is
    /// destroyed, and memory that cannot be bound is freed.
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
        faults::enter(Step::Buffer)?;
        let handle = unsafe {
            self.device()
                .create_buffer(&create, faults::callbacks(Step::Buffer))
        }?;
        faults::made(Step::Buffer, handle);
        let unmade = |why: Error| {
            unsafe {
                self.device()
                    .destroy_buffer(handle, faults::callbacks(Step::Buffer))
            };
            faults::gone(Step::Buffer, handle);
            why
        };

        let requirements = unsafe { self.device().get_buffer_memory_requirements(handle) };
        let Some(kind) = self.suits(requirements, wanted) else {
            return Err(unmade(Error::NoMemory { bytes }));
        };

        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(kind);
        let memory = faults::enter(Step::Memory)
            .and_then(|()| unsafe {
                self.device()
                    .allocate_memory(&allocate, faults::callbacks(Step::Memory))
            })
            .map_err(|code| unmade(code.into()))?;
        faults::made(Step::Memory, memory);
        faults::enter(Step::Bind)
            .and_then(|()| unsafe { self.device().bind_buffer_memory(handle, memory, 0) })
            .map_err(|code| {
                unsafe {
                    self.device()
                        .free_memory(memory, faults::callbacks(Step::Memory))
                };
                faults::gone(Step::Memory, memory);
                unmade(code.into())
            })?;

        Ok(Held {
            handle,
            memory,
            bytes: bytes.max(4),
            page: 0,
            pages: Vec::new(),
            visible: wanted.contains(vk::MemoryPropertyFlags::HOST_VISIBLE),
            kind: wanted,
        })
    }

    unsafe fn fence(&self) -> Result<vk::Fence, Error> {
        faults::enter(Step::Fence)?;
        let fence = unsafe {
            self.device().create_fence(
                &vk::FenceCreateInfo::default(),
                faults::callbacks(Step::Fence),
            )
        }?;
        faults::made(Step::Fence, fence);
        Ok(fence)
    }

    /// A timestamp pool, or none when the device has no timestamps or the
    /// pool cannot be made: timing is optional and never fails construction.
    unsafe fn queries(&self) -> Option<vk::QueryPool> {
        if !self.ticking().1 {
            return None;
        }
        let create = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(u32::try_from(STAGES + 3).unwrap_or(3));
        let made = faults::enter(Step::Queries).ok().and_then(|()| {
            unsafe {
                self.device()
                    .create_query_pool(&create, faults::callbacks(Step::Queries))
            }
            .ok()
        });
        if let Some(pool) = made {
            faults::made(Step::Queries, pool);
        }
        made
    }

    unsafe fn allocate_vast(&self, bytes: u64, usage: vk::BufferUsageFlags) -> Result<Held, Error> {
        let create = vk::BufferCreateInfo::default()
            .size(bytes.max(4))
            .usage(usage)
            .flags(vk::BufferCreateFlags::SPARSE_BINDING | vk::BufferCreateFlags::SPARSE_RESIDENCY)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        faults::enter(Step::Buffer)?;
        let handle = unsafe {
            self.device()
                .create_buffer(&create, faults::callbacks(Step::Buffer))
        }?;
        faults::made(Step::Buffer, handle);
        let requirements = unsafe { self.device().get_buffer_memory_requirements(handle) };
        Ok(Held {
            handle,
            memory: vk::DeviceMemory::null(),
            bytes: bytes.max(4),
            page: requirements.alignment.max(1),
            pages: Vec::new(),
            visible: false,
            kind: vk::MemoryPropertyFlags::empty(),
        })
    }
}

impl Held {
    const fn none() -> Self {
        Self {
            handle: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            bytes: 0,
            page: 0,
            pages: Vec::new(),
            visible: false,
            kind: vk::MemoryPropertyFlags::empty(),
        }
    }
}

impl Slot {
    fn staged(staging: Held) -> Self {
        Self {
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            staging,
            queries: None,
            held: None,
            flight: None,
            stamped: 0,
        }
    }
}

/// Construction fills a resident that starts with every handle null and owns
/// each object from the moment it is made, so a step that fails drops what
/// came before it through the one drop path. Destroying a null handle is a
/// no-op in Vulkan.
impl<'host> Resident<'host> {
    fn building(host: &'host Host, direct: bool, bindings: usize) -> Self {
        Self {
            host,
            direct,
            built: false,
            buffers: Vec::with_capacity(bindings),
            staging: Held::none(),
            layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            shaders: Vec::new(),
            pipelines: Vec::new(),
            writes: Vec::new(),
            commands: vk::CommandPool::null(),
            transfer: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            sending: AtomicBool::new(false),
            stranded: Vec::new(),
            slots: Vec::with_capacity(SLOTS),
            tickets: 0,
            cached: false,
            recording: Duration::ZERO,
            recorded: 0,
            copying: Duration::ZERO,
            copies: 0,
            pushing: Duration::ZERO,
            pushes: 0,
            poked: 0,
            patience: Resident::patience_of(std::env::var("GRIT_PATIENCE_SECS").ok().as_deref()),
            device: None,
            spans: Vec::new(),
            reused: 0,
        }
    }

    unsafe fn describe(&mut self) -> Result<(), Error> {
        let device = self.host.device();
        let count = u32::try_from(self.buffers.len()).unwrap_or(u32::MAX).max(1);

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
        faults::enter(Step::SetLayout)?;
        self.layout = unsafe {
            device.create_descriptor_set_layout(&create, faults::callbacks(Step::SetLayout))
        }?;
        faults::made(Step::SetLayout, self.layout);

        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(count)];
        let create = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes);
        faults::enter(Step::Pool)?;
        self.pool =
            unsafe { device.create_descriptor_pool(&create, faults::callbacks(Step::Pool)) }?;
        faults::made(Step::Pool, self.pool);

        let layouts = [self.layout];
        let allocate = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.pool)
            .set_layouts(&layouts);
        faults::enter(Step::Set)?;
        let sets = unsafe { device.allocate_descriptor_sets(&allocate) }?;
        self.set = sets.first().copied().unwrap_or_default();
        faults::made(Step::Set, self.set);

        let infos: Vec<[vk::DescriptorBufferInfo; 1]> = self
            .buffers
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
                    .dst_set(self.set)
                    .dst_binding(u32::try_from(index).unwrap_or(0))
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(info)
            })
            .collect();
        unsafe { device.update_descriptor_sets(&writes, &[]) };
        Ok(())
    }

    /// The pipeline layout and every module and pipeline are kept the moment
    /// they are made, so a pipeline that fails leaves its module, the layout
    /// and the pipelines before it to be freed with the rest.
    unsafe fn compile(&mut self, modules: &[&[u32]]) -> Result<(), Error> {
        let device = self.host.device();
        let layouts = [self.layout];
        let create = vk::PipelineLayoutCreateInfo::default().set_layouts(&layouts);
        faults::enter(Step::PipelineLayout)?;
        self.pipeline_layout = unsafe {
            device.create_pipeline_layout(&create, faults::callbacks(Step::PipelineLayout))
        }?;
        faults::made(Step::PipelineLayout, self.pipeline_layout);

        self.shaders.reserve(modules.len());
        self.pipelines.reserve(modules.len());
        let entry = c"main";

        for words in modules {
            let create = vk::ShaderModuleCreateInfo::default().code(words);
            faults::enter(Step::Shader)?;
            let shader =
                unsafe { device.create_shader_module(&create, faults::callbacks(Step::Shader)) }?;
            faults::made(Step::Shader, shader);
            self.shaders.push(shader);

            let stage = vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::COMPUTE)
                .module(shader)
                .name(entry);
            let create = [vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(self.pipeline_layout)];
            let built = match faults::enter(Step::Pipeline) {
                Ok(()) => unsafe {
                    device.create_compute_pipelines(
                        vk::PipelineCache::null(),
                        &create,
                        faults::callbacks(Step::Pipeline),
                    )
                },
                Err(code) => Err((Vec::new(), code)),
            };
            let (made, failed) = match built {
                Ok(made) => (made, None),
                Err((made, code)) => (made, Some(code)),
            };
            for pipeline in made {
                faults::made(Step::Pipeline, pipeline);
                self.pipelines.push(pipeline);
            }
            if let Some(code) = failed {
                return Err(code.into());
            }
        }
        Ok(())
    }

    /// Dispatches every pipeline once with no group, so a driver that
    /// compiles a pipeline at its first dispatch does it here, under
    /// [`FORMING`]. The dispatches are kept apart by the barrier a frame puts
    /// between its stages. A wedged host runs nothing, so it has nothing to
    /// compile.
    unsafe fn warm(&self) -> Result<(), Error> {
        if self.pipelines.is_empty() || self.host.wedged() {
            return Ok(());
        }
        let (layout, set) = (self.pipeline_layout, self.set);
        let pipelines = &self.pipelines;
        unsafe {
            self.once(|device, command| {
                device.cmd_bind_descriptor_sets(
                    command,
                    vk::PipelineBindPoint::COMPUTE,
                    layout,
                    0,
                    &[set],
                    &[],
                );
                let between = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)];
                for (nth, &pipeline) in pipelines.iter().enumerate() {
                    if nth > 0 {
                        device.cmd_pipeline_barrier(
                            command,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::PipelineStageFlags::COMPUTE_SHADER,
                            vk::DependencyFlags::empty(),
                            &between,
                            &[],
                            &[],
                        );
                    }
                    device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, pipeline);
                    device.cmd_dispatch(command, 0, 0, 0);
                }
            })
        }
    }

    unsafe fn record(&mut self) -> Result<(), Error> {
        let device = self.host.device();
        let create = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.host.family())
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        faults::enter(Step::Commands)?;
        self.commands =
            unsafe { device.create_command_pool(&create, faults::callbacks(Step::Commands)) }?;
        faults::made(Step::Commands, self.commands);

        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.commands)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(u32::try_from(SLOTS + 1).unwrap_or(2));
        faults::enter(Step::CommandBuffers)?;
        let mut spare = unsafe { device.allocate_command_buffers(&allocate) }?;
        spare
            .iter()
            .for_each(|&buffer| faults::made(Step::CommandBuffers, buffer));
        self.transfer = spare.pop().unwrap_or_default();
        for slot in &mut self.slots {
            slot.command = spare.pop().unwrap_or_default();
        }
        Ok(())
    }
}

impl Resident<'_> {
    pub const PATIENCE: Duration = Duration::from_secs(10);

    /// The bound a new resident starts with, given what the environment named.
    ///
    /// Ten seconds is a thousand times the longest honest frame on a card, and
    /// it is not that on a software implementation under load, where a search
    /// that is working can take minutes. A bound crossed by a dispatch that
    /// was going to finish reports a device lost that was never lost, so the
    /// number is settable from outside. Anything that is not a count of
    /// seconds above nought is ignored rather than argued with.
    #[must_use]
    pub fn patience_of(named: Option<&str>) -> Duration {
        named
            .and_then(|held| held.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map_or(Self::PATIENCE, Duration::from_secs)
    }

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

    fn idle(&self) -> Result<(), Error> {
        self.live()?;
        if self.slots.iter().any(|slot| slot.flight.is_some()) {
            return Err(Error::StillInFlight);
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

    /// Whether every submission this resident made and has not seen finish
    /// finishes within its patience, and never less than the default bound:
    /// a frame in flight, a transfer whose wait ran out, and a sparse bind
    /// left behind on a wedged host. A bound that declared the host wedged
    /// may have been far shorter than the work, which can still be running.
    fn drained(&self) -> bool {
        let mut pending: Vec<vk::Fence> = self
            .slots
            .iter()
            .filter(|slot| slot.flight.is_some())
            .map(|slot| slot.fence)
            .collect();
        if self.sending.load(Ordering::SeqCst) {
            pending.push(self.fence);
        }
        pending.extend(self.stranded.iter().map(|&(fence, _)| fence));
        if pending.is_empty() {
            return true;
        }
        let bound = self.patience.max(Self::PATIENCE);
        let patience = u64::try_from(bound.as_nanos()).unwrap_or(u64::MAX);
        let waited = unsafe { self.host.device().wait_for_fences(&pending, true, patience) };
        waited.is_ok()
    }

    #[must_use]
    pub fn bindings(&self) -> usize {
        self.buffers.len()
    }

    /// The device's buffer of binding `index`, for work recorded outside the
    /// resident on the same queue.
    #[cfg(feature = "window")]
    pub(crate) fn buffer(&self, index: usize) -> Option<vk::Buffer> {
        self.buffers.get(index).map(|held| held.handle)
    }

    #[must_use]
    pub fn passes(&self) -> usize {
        self.pipelines.len()
    }

    /// What each pass may write, in pass order, as read from its module.
    #[must_use]
    pub fn writes(&self) -> &[Written] {
        &self.writes
    }

    #[must_use]
    pub const fn pushing(&self) -> (Duration, u64, u64) {
        (self.pushing, self.pushes, self.poked)
    }

    pub fn write(&mut self, index: usize, words: &[u32]) -> Result<(), Error> {
        self.live()?;
        let began = std::time::Instant::now();
        let out = self.written(index, 0, words);
        self.pushing += began.elapsed();
        self.pushes += 1;
        self.poked += u64::from(*out.as_ref().unwrap_or(&false));
        out.map(|_| ())
    }

    fn within(&self, index: usize, at: usize, count: usize) -> Result<(&Held, Span), Error> {
        let held = self.at(index)?;
        let found = span(at, count, held.bytes, |needed| Error::Overrun {
            index,
            needed,
            held: words_in(held.bytes),
        })?;
        Ok((held, found))
    }

    fn written(&mut self, index: usize, at: usize, words: &[u32]) -> Result<bool, Error> {
        let (held, into) = self.within(index, at, words.len())?;
        if into.bytes == 0 {
            return Ok(false);
        }
        let (handle, memory, visible) = (held.handle, held.memory, held.visible);
        self.idle()?;
        if visible {
            return unsafe { self.poke(memory, into, words) }.map(|()| true);
        }
        let from = staged(0, words.len(), self.staging.bytes)?;
        let staging = self.staging.memory;

        unsafe {
            let mapped = self.host.device().map_memory(
                staging,
                from.from,
                from.bytes,
                vk::MemoryMapFlags::empty(),
            )?;
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped.cast::<u32>(), words.len());
            self.host.device().unmap_memory(staging);
        }

        let region = [vk::BufferCopy::default()
            .src_offset(from.from)
            .dst_offset(into.from)
            .size(into.bytes)];
        let source = self.staging.handle;
        unsafe {
            self.once(|device, command| device.cmd_copy_buffer(command, source, handle, &region))
                .map(|()| false)
        }
    }

    unsafe fn poke(
        &self,
        memory: vk::DeviceMemory,
        into: Span,
        words: &[u32],
    ) -> Result<(), Error> {
        unsafe {
            let mapped = self.host.device().map_memory(
                memory,
                into.from,
                into.bytes,
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
        self.written(index, at, words).map(|_| ())
    }

    pub fn write_many(&mut self, runs: &[Run<'_>]) -> Result<(), Error> {
        self.live()?;

        let mut taken = Vec::with_capacity(runs.len());
        let mut total = 0_usize;
        for run in runs {
            let (held, into) = self.within(run.index, run.at, run.words.len())?;
            if into.bytes == 0 {
                continue;
            }
            total = total.checked_add(run.words.len()).ok_or(Error::Staged {
                needed: usize::MAX,
                staging: words_in(self.staging.bytes),
            })?;
            taken.push((held.handle, into, run.words));
        }
        if taken.is_empty() {
            return Ok(());
        }
        let whole = staged(0, total, self.staging.bytes)?;
        self.idle()?;

        let staging = self.staging.memory;
        unsafe {
            let mapped = self.host.device().map_memory(
                staging,
                whole.from,
                whole.bytes,
                vk::MemoryMapFlags::empty(),
            )?;
            let mut along = 0_usize;
            for &(_, _, words) in &taken {
                let into = mapped.cast::<u32>().add(along);
                std::ptr::copy_nonoverlapping(words.as_ptr(), into, words.len());
                along += words.len();
            }
            self.host.device().unmap_memory(staging);
        }

        let source = self.staging.handle;
        let mut grouped: Vec<(vk::Buffer, Vec<vk::BufferCopy>)> = Vec::new();
        let mut along = whole.from;
        for &(handle, into, _) in &taken {
            let region = vk::BufferCopy::default()
                .src_offset(along)
                .dst_offset(into.from)
                .size(into.bytes);
            along += into.bytes;
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
        let (held, window) = self.within(index, at, words)?;
        let Span { from, bytes, .. } = window;
        if from % page != 0 || bytes % page != 0 {
            return Err(Error::Overrun {
                index,
                needed: at.saturating_add(words),
                held: words_in(held.bytes),
            });
        }
        if bytes == 0 {
            return Ok(());
        }
        self.idle()?;

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

            let fence = match device.create_fence(&vk::FenceCreateInfo::default(), None) {
                Ok(fence) => fence,
                Err(code) => {
                    device.free_memory(memory, None);
                    return Err(code.into());
                }
            };
            let ordered = self.host.ordered();
            let held = device.queue_bind_sparse(self.host.queue(), &info, fence);
            drop(ordered);
            if held.is_err() {
                device.destroy_fence(fence, None);
                device.free_memory(memory, None);
                held?;
            }
            let waited = self.await_fence(fence);
            if self.host.wedged() {
                self.stranded.push((fence, memory));
                return waited;
            }
            device.destroy_fence(fence, None);
            if waited.is_err() {
                device.free_memory(memory, None);
                return waited;
            }

            if let Some(slot) = self.buffers.get_mut(index) {
                slot.pages.push((from, memory, bytes));
            }
        }
        Ok(())
    }

    pub fn unbind(&mut self, index: usize, at: usize, words: usize) -> Result<(), Error> {
        let (held, window) = self.within(index, at, words)?;
        let going: Vec<(u64, vk::DeviceMemory, u64)> = held
            .pages
            .iter()
            .copied()
            .filter(|&(off, _, size)| {
                off >= window.from && off.checked_add(size).is_some_and(|end| end <= window.end)
            })
            .collect();
        if going.is_empty() {
            return Ok(());
        }
        self.idle()?;

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
            let ordered = self.host.ordered();
            let sent = device.queue_bind_sparse(self.host.queue(), &info, fence);
            drop(ordered);
            if sent.is_err() {
                device.destroy_fence(fence, None);
                sent?;
            }
            let waited = self.await_fence(fence);
            if self.host.wedged() {
                self.stranded.push((fence, vk::DeviceMemory::null()));
                return waited;
            }
            device.destroy_fence(fence, None);
            waited?;

            for &(_, memory, _) in &going {
                device.free_memory(memory, None);
            }
            if let Some(slot) = self.buffers.get_mut(index) {
                slot.pages.retain(|held| !going.contains(held));
            }
        }
        Ok(())
    }

    pub fn read(&mut self, index: usize, count: usize) -> Result<Vec<u32>, Error> {
        self.live()?;
        let (held, from) = self.within(index, 0, count)?;
        let handle = held.handle;
        let into = staged(0, count, self.staging.bytes)?;
        if from.bytes == 0 {
            return Ok(Vec::new());
        }

        let region = [vk::BufferCopy::default()
            .src_offset(from.from)
            .dst_offset(into.from)
            .size(from.bytes)];
        let began = std::time::Instant::now();
        let staging = self.staging.handle;
        unsafe {
            self.once(|device, command| {
                device.cmd_copy_buffer(command, handle, staging, &region);
                to_host(device, command);
            })
        }?;
        self.copying += began.elapsed();
        self.copies += 1;
        let (memory, room) = (self.staging.memory, self.staging.bytes);
        unsafe { self.mapped_at(memory, room, 0, count) }
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
        let Some((memory, room)) = self
            .slots
            .get(slot)
            .map(|slot| (slot.staging.memory, slot.staging.bytes))
        else {
            return Err(Error::NoSuchTicket {
                ticket: ticket.get(),
                held: None,
            });
        };
        let mut every = Vec::with_capacity(counts.len());
        let mut at = 0_usize;
        for count in counts {
            every.push(unsafe { self.mapped_at(memory, room, at, count) }?);
            at = at.saturating_add(count);
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
        self.within(index, 0, count).map(|_| ())
    }

    fn indirect(&self, stages: &[&[Groups]]) -> Result<(), Error> {
        for &count in stages.iter().flat_map(|stage| stage.iter()) {
            if let Groups::From { binding, at } = count {
                self.within(binding, at, INDIRECT)?;
            }
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
        let mut wanted = 0_usize;
        for &(index, count) in out {
            let (held, from) = self.within(index, 0, count)?;
            copy.push((held.handle, from.bytes));
            wanted = wanted.saturating_add(count);
        }
        staged(0, wanted, facing.room)?;

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
                        room: slot.staging.bytes,
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
        self.indirect(stages)?;
        self.host.limits().dispatchable(stages)?;

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
            let ordered = self.host.ordered();
            unsafe {
                device.reset_fences(&[facing.fence])?;
                device.queue_submit(self.host.queue(), &submit, facing.fence)?;
            }
            drop(ordered);
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
            after_earlier(device, facing.command);

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

            let running = running(stages);
            let mut carrying = carried(&running, &self.writes, times).into_iter();
            let profiling = facing
                .queries
                .filter(|_| running.len() <= STAGES)
                .map(|pool| (pool, times.saturating_sub(1)));
            if let Some(slot) = self.slots.get_mut(at) {
                slot.stamped = profiling.map_or(0, |_| running.len());
            }

            for repeat in 0..times {
                for (nth, (_, stage)) in running.iter().enumerate() {
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
                                let (held, from) = self.within(binding, at, INDIRECT)?;
                                device.cmd_dispatch_indirect(
                                    facing.command,
                                    held.handle,
                                    from.from,
                                );
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
                        let (reach, stage) = if carrying.next().unwrap_or(true) {
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
                if bytes == 0 {
                    continue;
                }
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

                let region = [vk::BufferCopy::default().dst_offset(into).size(bytes)];
                device.cmd_copy_buffer(facing.command, handle, facing.staging, &region);
                into = into.saturating_add(bytes);

                to_host(device, facing.command);
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

            let ordered = self.host.ordered();
            device.reset_fences(&[facing.fence])?;
            device.queue_submit(self.host.queue(), &submit, facing.fence)?;
            drop(ordered);
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
        self.idle()?;
        let device = self.host.device();
        unsafe {
            device.reset_command_buffer(self.transfer, vk::CommandBufferResetFlags::empty())?;
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(self.transfer, &begin)?;
            after_earlier(device, self.transfer);
            record(device, self.transfer);
            device.end_command_buffer(self.transfer)?;

            let commands = [self.transfer];
            let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
            let ordered = self.host.ordered();
            device.reset_fences(&[self.fence])?;
            device.queue_submit(self.host.queue(), &submit, self.fence)?;
            self.sending.store(true, Ordering::SeqCst);
            drop(ordered);
        }
        self.await_fence(self.fence)?;
        self.sending.store(false, Ordering::SeqCst);
        Ok(())
    }

    unsafe fn mapped_at(
        &self,
        memory: vk::DeviceMemory,
        room: u64,
        at: usize,
        count: usize,
    ) -> Result<Vec<u32>, Error> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let from = staged(at, count, room)?;
        unsafe {
            let mapped = self.host.device().map_memory(
                memory,
                from.from,
                from.bytes,
                vk::MemoryMapFlags::empty(),
            )?;
            let mut out = vec![0_u32; count];
            std::ptr::copy_nonoverlapping(mapped.cast::<u32>(), out.as_mut_ptr(), count);
            self.host.device().unmap_memory(memory);
            Ok(out)
        }
    }
}

type Running = Vec<(usize, Vec<(usize, Groups)>)>;

fn running(stages: &[&[Groups]]) -> Running {
    let mut pass = 0;
    let mut out = Vec::with_capacity(stages.len());
    for (at, stage) in stages.iter().enumerate() {
        let mut held = Vec::new();
        for &count in *stage {
            if !matches!(count, Groups::Fixed(0) | Groups::Counting(0)) {
                held.push((pass, count));
            }
            pass += 1;
        }
        if !held.is_empty() {
            out.push((at, held));
        }
    }
    out
}

struct Since {
    every: bool,
    bindings: Vec<usize>,
}

impl Since {
    fn ran(&mut self, writes: Option<&Written>) {
        match writes {
            Some(Written::Only(bindings)) => {
                for &binding in bindings {
                    if !self.bindings.contains(&binding) {
                        self.bindings.push(binding);
                    }
                }
            }
            Some(Written::Every) | None => self.every = true,
        }
    }

    fn holds(&self, binding: usize) -> bool {
        self.every || self.bindings.contains(&binding)
    }

    fn clear(&mut self) {
        self.every = false;
        self.bindings.clear();
    }
}

/// For each barrier between two running stages, in recording order, whether
/// it carries the indirect stage and access: only when a `From` pass of the
/// stage it opens reads a binding some pass may have written since the last
/// barrier that carried them. The frame starts behind one that does.
fn carried(running: &Running, writes: &[Written], times: u32) -> Vec<bool> {
    let mut since = Since {
        every: false,
        bindings: Vec::new(),
    };
    let mut out = Vec::new();
    for repeat in 0..times {
        for (nth, (_, stage)) in running.iter().enumerate() {
            for &(pass, _) in stage {
                since.ran(writes.get(pass));
            }
            let next = if nth + 1 < running.len() {
                running.get(nth + 1)
            } else if repeat + 1 < times {
                running.first()
            } else {
                None
            };
            if let Some((_, next)) = next {
                let needed = next.iter().any(|&(_, count)| match count {
                    Groups::From { binding, .. } => since.holds(binding),
                    Groups::Fixed(_) | Groups::Counting(_) => false,
                });
                if needed {
                    since.clear();
                }
                out.push(needed);
            }
        }
    }
    out
}

/// The barriers of a recording of `stages` repeated `times` that carry the
/// indirect stage and access, each named by the repeat and the index in
/// `stages` of the stage it opens, given what each pass may write.
#[must_use]
pub fn indirect_barriers(
    stages: &[&[Groups]],
    writes: &[Written],
    times: u32,
) -> Vec<(u32, usize)> {
    let running = running(stages);
    let carrying = carried(&running, writes, times);
    let opened = (0..times).flat_map(|repeat| running.iter().map(move |&(at, _)| (repeat, at)));
    opened
        .skip(1)
        .zip(carrying)
        .filter_map(|(opens, carries)| carries.then_some(opens))
        .collect()
}

fn to_host(device: &ash::Device, command: vk::CommandBuffer) {
    let barrier = [vk::MemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::HOST_READ)];
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::HOST,
            vk::DependencyFlags::empty(),
            &barrier,
            &[],
            &[],
        );
    }
}

fn after_earlier(device: &ash::Device, command: vk::CommandBuffer) {
    let barrier = [vk::MemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE | vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(
            vk::AccessFlags::SHADER_READ
                | vk::AccessFlags::SHADER_WRITE
                | vk::AccessFlags::INDIRECT_COMMAND_READ
                | vk::AccessFlags::TRANSFER_READ
                | vk::AccessFlags::TRANSFER_WRITE,
        )];
    unsafe {
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::COMPUTE_SHADER | vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::COMPUTE_SHADER
                | vk::PipelineStageFlags::DRAW_INDIRECT
                | vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &barrier,
            &[],
            &[],
        );
    }
}

impl Drop for Resident<'_> {
    /// A finished resident may have work on the device. On a healthy host it
    /// waits for the device first. On a wedged host it waits, within a bound,
    /// for its own unfinished work only, since the device may never be idle:
    /// work that finishes in time is freed as usual, and work that does not
    /// is left alive with everything it uses, and the host with it. One still
    /// being built has never submitted anything, so it frees what it made at
    /// once, wedged or not.
    fn drop(&mut self) {
        let wedged = self.built && self.host.wedged();
        if wedged && !self.drained() {
            return;
        }
        let device = self.host.device();
        unsafe {
            if self.built && !wedged {
                let _ = device.device_wait_idle();
            }
            device.destroy_fence(self.fence, faults::callbacks(Step::Fence));
            faults::gone(Step::Fence, self.fence);
            for slot in &self.slots {
                device.destroy_fence(slot.fence, faults::callbacks(Step::Fence));
                faults::gone(Step::Fence, slot.fence);
                if let Some(pool) = slot.queries {
                    device.destroy_query_pool(pool, faults::callbacks(Step::Queries));
                    faults::gone(Step::Queries, pool);
                }
            }
            device.destroy_command_pool(self.commands, faults::callbacks(Step::Commands));
            if self.commands != vk::CommandPool::null() {
                faults::gone(Step::CommandBuffers, self.transfer);
                for slot in &self.slots {
                    faults::gone(Step::CommandBuffers, slot.command);
                }
            }
            faults::gone(Step::Commands, self.commands);
            for &pipeline in &self.pipelines {
                device.destroy_pipeline(pipeline, faults::callbacks(Step::Pipeline));
                faults::gone(Step::Pipeline, pipeline);
            }
            for &shader in &self.shaders {
                device.destroy_shader_module(shader, faults::callbacks(Step::Shader));
                faults::gone(Step::Shader, shader);
            }
            device.destroy_pipeline_layout(
                self.pipeline_layout,
                faults::callbacks(Step::PipelineLayout),
            );
            faults::gone(Step::PipelineLayout, self.pipeline_layout);
            device.destroy_descriptor_pool(self.pool, faults::callbacks(Step::Pool));
            if self.pool != vk::DescriptorPool::null() {
                faults::gone(Step::Set, self.set);
            }
            faults::gone(Step::Pool, self.pool);
            device.destroy_descriptor_set_layout(self.layout, faults::callbacks(Step::SetLayout));
            faults::gone(Step::SetLayout, self.layout);
            let staged = self.slots.iter().map(|slot| &slot.staging);
            for held in self
                .buffers
                .iter()
                .chain(std::iter::once(&self.staging))
                .chain(staged)
            {
                device.destroy_buffer(held.handle, faults::callbacks(Step::Buffer));
                faults::gone(Step::Buffer, held.handle);
                device.free_memory(held.memory, faults::callbacks(Step::Memory));
                faults::gone(Step::Memory, held.memory);
                for &(_, memory, _) in &held.pages {
                    device.free_memory(memory, None);
                }
            }
            for &(fence, memory) in &self.stranded {
                device.destroy_fence(fence, None);
                device.free_memory(memory, None);
            }
        }
        if self.built {
            self.host.settle();
        }
    }
}
