use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ash::vk;

use crate::ops::Dots;
use crate::resident::Groups;
use crate::resident::faults::{self, Step};

/// The subgroup operations the kernels Grit builds use. The engine's kernels
/// declare `GroupNonUniform`, `GroupNonUniformVote` for the `any` in
/// `gate::if_any`, and `GroupNonUniformArithmetic` for the sums, the exclusive
/// prefix sums and the maximum in `compact`, `live` and `sweep`; the rule and
/// circuit kernels declare none. `tests/operations.rs` holds the list to what
/// the built modules declare.
pub const OPERATIONS: vk::SubgroupFeatureFlags = vk::SubgroupFeatureFlags::from_raw(
    vk::SubgroupFeatureFlags::BASIC.as_raw()
        | vk::SubgroupFeatureFlags::VOTE.as_raw()
        | vk::SubgroupFeatureFlags::ARITHMETIC.as_raw(),
);

/// The Vulkan a device has to offer: the modules are SPIR-V 1.3, and the
/// subgroup properties are only there to read from 1.1.
pub const API: u32 = vk::API_VERSION_1_1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    StorageBufferRange,
    MemoryAllocationSize,
    WorkGroupCount,
}

impl std::fmt::Display for Limit {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::StorageBufferRange => "maxStorageBufferRange",
            Self::MemoryAllocationSize => "maxMemoryAllocationSize",
            Self::WorkGroupCount => "maxComputeWorkGroupCount[0]",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub storage_range: u64,
    pub allocation: u64,
    pub groups: u32,
    pub width: u32,
    pub stages: vk::ShaderStageFlags,
    pub operations: vk::SubgroupFeatureFlags,
    pub api: u32,
}

impl Limits {
    #[must_use]
    pub fn of(
        core: &vk::PhysicalDeviceProperties,
        subgroup: &vk::PhysicalDeviceSubgroupProperties<'_>,
        third: &vk::PhysicalDeviceMaintenance3Properties<'_>,
    ) -> Self {
        let [groups, _, _] = core.limits.max_compute_work_group_count;
        Self {
            storage_range: u64::from(core.limits.max_storage_buffer_range),
            allocation: third.max_memory_allocation_size,
            groups,
            width: subgroup.subgroup_size,
            stages: subgroup.supported_stages,
            operations: subgroup.supported_operations,
            api: core.api_version,
        }
    }

    /// Whether the kernels can run here at all: Vulkan 1.1 or later, and every
    /// subgroup operation they use offered in compute shaders.
    #[must_use]
    pub fn serves(&self) -> bool {
        let version = |api: u32| {
            (
                vk::api_version_variant(api),
                vk::api_version_major(api),
                vk::api_version_minor(api),
            )
        };
        let (variant, major, minor) = version(self.api);
        let (_, least_major, least_minor) = version(API);
        variant == 0
            && (major, minor) >= (least_major, least_minor)
            && self.stages.contains(vk::ShaderStageFlags::COMPUTE)
            && self.operations.contains(OPERATIONS)
    }

    #[must_use]
    pub fn ceiling(&self) -> u64 {
        self.allocation.min(self.storage_range)
    }

    /// Refuses the first binding the device cannot hold, before anything is
    /// made. A binding is described whole, so it may not pass the storage
    /// range; one that is not `vast` is also one allocation, so it may not
    /// pass the allocation size either. An empty binding is made one word,
    /// and one whose bytes do not fit a `u64` is refused as no memory can
    /// hold it, as resident creation always refused it.
    pub fn admits(&self, sizes: &[usize], vast: Option<usize>) -> Result<(), Error> {
        for (binding, &words) in sizes.iter().enumerate() {
            let refused = |limit, allowed| Error::TooLarge {
                binding,
                words,
                limit,
                allowed,
            };
            let Some(bytes) = u64::try_from(words.max(1))
                .ok()
                .and_then(|counted| counted.checked_mul(size_of::<u32>() as u64))
            else {
                return Err(Error::NoMemory { bytes: u64::MAX });
            };
            if bytes > self.storage_range {
                return Err(refused(Limit::StorageBufferRange, self.storage_range));
            }
            if Some(binding) != vast && bytes > self.allocation {
                return Err(refused(Limit::MemoryAllocationSize, self.allocation));
            }
        }
        Ok(())
    }

    /// [`Limits::admits`], and then refuses bindings whose bytes together
    /// pass `heap`, the device's own memory, before anything is made.
    pub fn admits_on(&self, sizes: &[usize], vast: Option<usize>, heap: u64) -> Result<(), Error> {
        self.admits(sizes, vast)?;
        let mut bytes = 0_u64;
        for &words in sizes {
            let held = u64::try_from(words.max(1))
                .ok()
                .and_then(|counted| counted.checked_mul(size_of::<u32>() as u64))
                .ok_or(Error::NoMemory { bytes: u64::MAX })?;
            bytes = bytes
                .checked_add(held)
                .ok_or(Error::NoMemory { bytes: u64::MAX })?;
        }
        if bytes > heap {
            return Err(Error::TooMuchMemory { bytes, heap });
        }
        Ok(())
    }

    /// Refuses the first pass whose workgroup count the device does not
    /// promise to dispatch. A `From` count is written on the device and is
    /// not known here.
    pub fn dispatchable(&self, stages: &[&[Groups]]) -> Result<(), Error> {
        let passes = stages.iter().flat_map(|stage| stage.iter());
        for (pass, &count) in passes.enumerate() {
            if let Groups::Fixed(groups) | Groups::Counting(groups) = count
                && groups > self.groups
            {
                return Err(Error::TooManyGroups {
                    pass,
                    groups,
                    allowed: self.groups,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Error {
    NoLoader(ash::LoadingError),
    NoInstance(vk::Result),
    NoDeviceList(vk::Result),
    NoDevice {
        name: String,
        code: vk::Result,
    },
    Vulkan(vk::Result),
    NoComputeDevice,
    NoSuchDevice {
        wanted: String,
        present: Vec<String>,
    },
    Unsuited {
        name: String,
        api: u32,
        stages: vk::ShaderStageFlags,
        operations: vk::SubgroupFeatureFlags,
    },
    TooLarge {
        binding: usize,
        words: usize,
        limit: Limit,
        allowed: u64,
    },
    TooManyGroups {
        pass: usize,
        groups: u32,
        allowed: u32,
    },
    TooMuchMemory {
        bytes: u64,
        heap: u64,
    },
    NoMemory {
        bytes: u64,
    },
    NoSuchBinding {
        index: usize,
        held: usize,
    },
    Overrun {
        index: usize,
        needed: usize,
        held: usize,
    },
    Staged {
        needed: usize,
        staging: usize,
    },
    NoSparse,
    StillInFlight,
    NoSuchTicket {
        ticket: u64,
        held: Option<u64>,
    },
    WrongOutputs {
        asked: usize,
        wanted: usize,
    },
    Wedged {
        after: Duration,
    },
    WrongPasses {
        given: usize,
        held: usize,
    },
    Strayed {
        at: usize,
        was: u32,
        now: u32,
    },
    PoolFull {
        slots: u32,
    },
    LostWrite,
    Lane(simdr::lanes::LaneError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLoader(why) => {
                write!(out, "the Vulkan loader could not be used: {why}")
            }
            Self::NoInstance(code) => {
                write!(out, "Vulkan refused to create an instance: {code:?}")
            }
            Self::NoDeviceList(code) => {
                write!(
                    out,
                    "Vulkan could not list the devices on this machine: {code:?}"
                )
            }
            Self::NoDevice { name, code } => {
                write!(out, "Vulkan refused to open the device {name:?}: {code:?}")
            }
            Self::Vulkan(code) => write!(out, "a Vulkan call failed with {code:?}"),
            Self::NoSparse => write!(
                out,
                "the device does not offer sparse residency for buffers"
            ),
            Self::NoComputeDevice => {
                write!(
                    out,
                    "no Vulkan device on this machine offers a compute queue"
                )
            }
            Self::NoSuchDevice { wanted, present } => {
                write!(out, "no Vulkan device name contains {wanted:?}; ")?;
                if present.is_empty() {
                    write!(out, "this machine has no compute device")
                } else {
                    write!(out, "the compute devices here are {present:?}")
                }
            }
            Self::Unsuited {
                name,
                api,
                stages,
                operations,
            } => write!(
                out,
                "the device {name:?} cannot run the kernels: they need Vulkan {}.{} and the subgroup operations {OPERATIONS:?} in compute shaders, and it offers Vulkan {}.{} with {operations:?} in {stages:?}",
                vk::api_version_major(API),
                vk::api_version_minor(API),
                vk::api_version_major(*api),
                vk::api_version_minor(*api),
            ),
            Self::TooLarge {
                binding,
                words,
                limit,
                allowed,
            } => write!(
                out,
                "binding {binding} asks for {words} words, more than the device's {limit} of {allowed} bytes lets one binding hold"
            ),
            Self::TooManyGroups {
                pass,
                groups,
                allowed,
            } => write!(
                out,
                "pass {pass} asks for {groups} workgroups, more than the device's {} of {allowed} lets one dispatch hold",
                Limit::WorkGroupCount
            ),
            Self::TooMuchMemory { bytes, heap } => write!(
                out,
                "the bindings ask for {bytes} bytes together, more than the {heap} bytes of the device's own memory"
            ),
            Self::NoMemory { bytes } => write!(
                out,
                "no memory type on the device can hold a buffer of {bytes} bytes"
            ),
            Self::NoSuchBinding { index, held } => write!(
                out,
                "binding {index} does not exist; the resident holds {held} bindings"
            ),
            Self::Overrun {
                index,
                needed,
                held,
            } => write!(
                out,
                "index {index} asks for {needed} words where only {held} are held"
            ),
            Self::Staged { needed, staging } => write!(
                out,
                "a transfer of {needed} words does not fit the staging buffer of {staging} words"
            ),
            Self::WrongPasses { given, held } => write!(
                out,
                "the stages name {given} passes, which does not fit the {held} passes the resident holds"
            ),
            Self::StillInFlight => write!(
                out,
                "a submission is still in flight, so the resident cannot take this one yet"
            ),
            Self::Wedged { after } => write!(
                out,
                "the device did not finish a submission within {after:?} and is treated as wedged"
            ),
            Self::WrongOutputs { asked, wanted } => write!(
                out,
                "the submission carried {asked} outputs where {wanted} were wanted"
            ),
            Self::NoSuchTicket { ticket, held } => match held {
                Some(one) => write!(
                    out,
                    "ticket {ticket} is not in flight; ticket {one} is the one in flight"
                ),
                None => write!(out, "ticket {ticket} is not in flight, and no ticket is"),
            },
            Self::Strayed { at, was, now } => write!(
                out,
                "cell {at} did not come back to where it started: it was {was} and is now {now}"
            ),
            Self::PoolFull { slots } => write!(
                out,
                "the brick pool of {slots} slots is full: a frame wanted a brick it could not hold, and the world stopped at the frame before"
            ),
            Self::LostWrite => write!(
                out,
                "a frame wrote a uniform brick that held no slot, and the write was dropped"
            ),
            Self::Lane(why) => write!(out, "a kernel could not be built: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(code: vk::Result) -> Self {
        Self::Vulkan(code)
    }
}

impl From<simdr::lanes::LaneError> for Error {
    fn from(why: simdr::lanes::LaneError) -> Self {
        Self::Lane(why)
    }
}

pub struct Host {
    entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    /// Held across every submission, because a queue is one.
    ///
    /// Vulkan asks that host access to a queue be externally synchronised,
    /// and nothing in Rust says so: the handle is Sync, so two residents made
    /// from one host will submit from two threads and be allowed to. What that
    /// costs is measurable -- two of six parallel runs of `tests/dispatch.rs`
    /// at a subgroup of 16 came back with a pass having run once too often.
    ordered: std::sync::Mutex<()>,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    name: String,
    limits: Limits,
    tick: f32,
    timing: bool,
    wedged: AtomicBool,
    owing: AtomicUsize,
    sparse: bool,
    strict: bool,
    int64: bool,
    #[cfg(feature = "window")]
    physical: vk::PhysicalDevice,
    #[cfg(feature = "window")]
    presents: bool,
    dots: Dots,
}

/// The instance extensions a host that shows its pictures in a window asks
/// for, when the loader offers all of them, and whether it did.
#[cfg(feature = "window")]
fn window_instance(entry: &ash::Entry, presenting: bool) -> (Vec<*const c_char>, bool) {
    const WANTED: [&CStr; 2] = [ash::khr::surface::NAME, ash::khr::win32_surface::NAME];
    let offered = if presenting {
        unsafe { entry.enumerate_instance_extension_properties(None) }.unwrap_or_default()
    } else {
        Vec::new()
    };
    let windowed = presenting
        && WANTED.iter().all(|wanted| {
            offered.iter().any(|one| {
                one.extension_name_as_c_str()
                    .is_ok_and(|name| name == *wanted)
            })
        });
    let names = if windowed {
        WANTED.iter().map(|name| name.as_ptr()).collect()
    } else {
        Vec::new()
    };
    (names, windowed)
}

#[cfg(not(feature = "window"))]
fn window_instance(_: &ash::Entry, _: bool) -> (Vec<*const c_char>, bool) {
    (Vec::new(), false)
}

/// The device extensions a device that presents asks for, when `windowed`
/// and the device offers them.
#[cfg(feature = "window")]
fn window_device(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    windowed: bool,
) -> Vec<*const c_char> {
    let presents = windowed && {
        let offered =
            unsafe { instance.enumerate_device_extension_properties(physical) }.unwrap_or_default();
        offered.iter().any(|one| {
            one.extension_name_as_c_str()
                .is_ok_and(|name| name == ash::khr::swapchain::NAME)
        })
    };
    if presents {
        vec![ash::khr::swapchain::NAME.as_ptr()]
    } else {
        Vec::new()
    }
}

#[cfg(not(feature = "window"))]
fn window_device(_: &ash::Instance, _: vk::PhysicalDevice, _: bool) -> Vec<*const c_char> {
    Vec::new()
}

impl Host {
    pub fn open() -> Result<Option<Self>, Error> {
        let pattern =
            std::env::var_os("GRIT_DEVICE").map(|held| held.to_string_lossy().into_owned());
        Self::matching(pattern.as_deref())
    }

    pub fn matching(pattern: Option<&str>) -> Result<Option<Self>, Error> {
        Self::made(pattern, false)
    }

    /// [`Host::open`], on an instance that can make a Win32 window's surface
    /// and a device that can present to it, where the loader and the device
    /// offer that; [`Host::presents`] tells whether they did. Everything
    /// else is as [`Host::open`] makes it.
    #[cfg(feature = "window")]
    pub fn open_presenting() -> Result<Option<Self>, Error> {
        let pattern =
            std::env::var_os("GRIT_DEVICE").map(|held| held.to_string_lossy().into_owned());
        Self::made(pattern.as_deref(), true)
    }

    fn made(pattern: Option<&str>, presenting: bool) -> Result<Option<Self>, Error> {
        let _forming = crate::resident::forming();
        let entry = match unsafe { ash::Entry::load() } {
            Ok(entry) => entry,
            Err(ash::LoadingError::LibraryLoadFailure(_)) => return Ok(None),
            Err(why) => return Err(Error::NoLoader(why)),
        };

        let application =
            vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
        let (names, windowed) = window_instance(&entry, presenting);
        let create = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&names);

        let instance = match unsafe { entry.create_instance(&create, None) } {
            Ok(instance) => instance,
            Err(vk::Result::ERROR_INCOMPATIBLE_DRIVER) => return Ok(None),
            Err(code) => return Err(Error::NoInstance(code)),
        };

        unsafe { Self::on(entry, instance, pattern, windowed) }.map(Some)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.limits.width
    }

    #[must_use]
    pub const fn limits(&self) -> Limits {
        self.limits
    }

    #[must_use]
    pub fn heap(&self) -> u64 {
        let held = &self.memory;
        held.memory_heaps
            .iter()
            .take(held.memory_heap_count as usize)
            .filter(|one| one.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
            .map(|one| one.size)
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn ceiling(&self) -> u64 {
        self.limits.ceiling()
    }

    unsafe fn on(
        entry: ash::Entry,
        instance: ash::Instance,
        pattern: Option<&str>,
        windowed: bool,
    ) -> Result<Self, Error> {
        let candidates = match unsafe { instance.enumerate_physical_devices() } {
            Ok(candidates) => candidates,
            Err(code) => {
                unsafe { instance.destroy_instance(None) };
                return Err(Error::NoDeviceList(code));
            }
        };

        let compute: Vec<(vk::PhysicalDevice, u32)> = candidates
            .into_iter()
            .filter_map(|physical| {
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let family = families.iter().position(|family| {
                    family.queue_flags.contains(vk::QueueFlags::COMPUTE) && family.queue_count > 0
                })?;
                u32::try_from(family).ok().map(|family| (physical, family))
            })
            .collect();

        if compute.is_empty() {
            unsafe { instance.destroy_instance(None) };
            return Err(Error::NoComputeDevice);
        }

        let named = |physical| unsafe { name_of(&instance, physical) };
        let wanted = pattern.map(str::to_lowercase);

        let matching: Vec<(vk::PhysicalDevice, u32)> = compute
            .iter()
            .copied()
            .filter(|&(physical, _)| {
                wanted
                    .as_deref()
                    .is_none_or(|wanted| named(physical).to_lowercase().contains(wanted))
            })
            .collect();
        let chosen = matching
            .iter()
            .copied()
            .filter(|&(physical, _)| unsafe { limits_of(&instance, physical) }.serves())
            .max_by_key(|&(physical, _)| {
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                u8::from(properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU)
            });

        if chosen.is_none()
            && let Some(&(physical, _)) = matching.first()
        {
            let name = named(physical);
            let limits = unsafe { limits_of(&instance, physical) };
            unsafe { instance.destroy_instance(None) };
            return Err(Error::Unsuited {
                name,
                api: limits.api,
                stages: limits.stages,
                operations: limits.operations,
            });
        }

        let Some((physical, family)) = chosen else {
            let present = compute
                .iter()
                .map(|&(physical, _)| named(physical))
                .collect();
            unsafe { instance.destroy_instance(None) };
            return Err(Error::NoSuchDevice {
                wanted: pattern.unwrap_or_default().to_owned(),
                present,
            });
        };

        let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
        let name = unsafe { name_of(&instance, physical) };
        let limits = unsafe { limits_of(&instance, physical) };
        let (tick, timing) = unsafe { timestamps(&instance, physical, family) };

        let priorities = [1.0_f32];
        let has = unsafe { instance.get_physical_device_features(physical) };
        let wanted = vk::PhysicalDeviceFeatures::default()
            .sparse_binding(has.sparse_binding == 1)
            .sparse_residency_buffer(has.sparse_binding == 1 && has.sparse_residency_buffer == 1)
            .shader_int64(has.shader_int64 == 1);
        let sparse = wanted.sparse_binding == 1 && wanted.sparse_residency_buffer == 1;
        let int64 = wanted.shader_int64 == 1;
        let strict = sparse && {
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            properties.sparse_properties.residency_non_resident_strict == 1
        };

        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let mut extensions = window_device(&instance, physical, windowed);
        #[cfg(feature = "window")]
        let presents = !extensions.is_empty();
        let dots = unsafe { dots_of(&instance, physical) };
        let mut dotting = vk::PhysicalDeviceShaderIntegerDotProductFeatures::default()
            .shader_integer_dot_product(true);
        let mut create = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_features(&wanted);
        if dots == Dots::Native {
            extensions.push(ash::khr::shader_integer_dot_product::NAME.as_ptr());
            create = create.push_next(&mut dotting);
        }
        let create = create.enabled_extension_names(&extensions);

        let made =
            unsafe { instance.create_device(physical, &create, faults::callbacks(Step::Device)) };
        let device = match made {
            Ok(device) => {
                faults::made(Step::Device, device.handle());
                device
            }
            Err(code) => {
                unsafe { instance.destroy_instance(None) };
                return Err(Error::NoDevice { name, code });
            }
        };
        let queue = unsafe { device.get_device_queue(family, 0) };

        Ok(Self {
            entry,
            instance,
            device,
            queue,
            ordered: std::sync::Mutex::new(()),
            family,
            memory,
            name,
            limits,
            tick,
            timing,
            sparse,
            strict,
            int64,
            #[cfg(feature = "window")]
            physical,
            #[cfg(feature = "window")]
            presents,
            dots,
            wedged: AtomicBool::new(false),
            owing: AtomicUsize::new(0),
        })
    }

    pub(crate) fn wedge(&self) {
        self.wedged.store(true, Ordering::SeqCst);
    }

    pub(crate) fn wedged(&self) -> bool {
        self.wedged.load(Ordering::SeqCst)
    }

    /// A finished resident now exists and may submit work. It owes the host
    /// until its drop has seen all of that work finish.
    pub(crate) fn owe(&self) {
        self.owing.fetch_add(1, Ordering::SeqCst);
    }

    /// A finished resident was freed after it saw all its work finish. Each
    /// finished resident owes once and settles at most once, in its drop.
    pub(crate) fn settle(&self) {
        self.owing.fetch_sub(1, Ordering::SeqCst);
    }

    pub(crate) const fn ticking(&self) -> (f32, bool) {
        (self.tick, self.timing)
    }

    #[must_use]
    pub const fn sparse(&self) -> bool {
        self.sparse
    }

    #[must_use]
    pub const fn strict(&self) -> bool {
        self.strict
    }

    #[must_use]
    pub const fn int64(&self) -> bool {
        self.int64
    }

    /// Whether the host was made by [`Host::open_presenting`] and its device
    /// can present to a window.
    #[cfg(feature = "window")]
    #[must_use]
    pub const fn presents(&self) -> bool {
        self.presents
    }

    #[must_use]
    pub const fn dots(&self) -> Dots {
        self.dots
    }

    #[cfg(feature = "window")]
    pub(crate) const fn entry(&self) -> &ash::Entry {
        &self.entry
    }

    #[cfg(feature = "window")]
    pub(crate) const fn instance(&self) -> &ash::Instance {
        &self.instance
    }

    #[cfg(feature = "window")]
    pub(crate) const fn physical(&self) -> vk::PhysicalDevice {
        self.physical
    }

    pub(crate) const fn device(&self) -> &ash::Device {
        &self.device
    }

    /// The turn to submit, held until the guard is dropped.
    ///
    /// A poisoned lock is taken anyway: it guards an order, not a value, and a
    /// panic in one thread is no reason for every later submission to refuse.
    pub(crate) fn ordered(&self) -> std::sync::MutexGuard<'_, ()> {
        self.ordered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) const fn queue(&self) -> vk::Queue {
        self.queue
    }

    pub(crate) const fn family(&self) -> u32 {
        self.family
    }

    pub(crate) fn suits(
        &self,
        requirements: vk::MemoryRequirements,
        wanted: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        (0..self.memory.memory_type_count).find(|&index| {
            let suitable = requirements.memory_type_bits & (1 << index) != 0;
            let offered = self
                .memory
                .memory_types
                .get(index as usize)
                .is_some_and(|kind| kind.property_flags.contains(wanted));
            suitable && offered
        })
    }
}

impl Drop for Host {
    /// A wedged host is left alive while any resident still owes it: one
    /// that left work on the device unfinished, or one that was never
    /// dropped. Once every resident has seen its own work finish, nothing is
    /// pending, and the device and instance are destroyed as on a healthy
    /// host, so no driver thread is left running while the process exits.
    fn drop(&mut self) {
        if self.wedged() && self.owing.load(Ordering::SeqCst) > 0 {
            return;
        }
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(faults::callbacks(Step::Device));
            faults::gone(Step::Device, self.device.handle());
            self.instance.destroy_instance(None);
        }
        let _ = &self.entry;
    }
}

unsafe fn name_of(instance: &ash::Instance, physical: vk::PhysicalDevice) -> String {
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let bytes: &[c_char] = &properties.device_name;
    let name = unsafe { CStr::from_ptr(bytes.as_ptr()) };
    name.to_string_lossy().into_owned()
}

/// What the device promises. The 1.1 query is made only where the device is
/// 1.1 or later; an older one keeps empty subgroup properties, so it is not
/// chosen.
unsafe fn limits_of(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Limits {
    let core = unsafe { instance.get_physical_device_properties(physical) };
    let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
    let mut third = vk::PhysicalDeviceMaintenance3Properties::default();
    let version = |api: u32| (vk::api_version_major(api), vk::api_version_minor(api));
    if version(core.api_version) >= version(API) {
        let mut properties = vk::PhysicalDeviceProperties2::default()
            .push_next(&mut subgroup)
            .push_next(&mut third);
        unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    }
    Limits::of(&core, &subgroup, &third)
}

unsafe fn dots_of(instance: &ash::Instance, physical: vk::PhysicalDevice) -> Dots {
    let offered = unsafe { instance.enumerate_device_extension_properties(physical) }
        .unwrap_or_default()
        .iter()
        .any(|one| {
            one.extension_name_as_c_str()
                .is_ok_and(|name| name == ash::khr::shader_integer_dot_product::NAME)
        });
    if !offered {
        return Dots::Exact;
    }
    let mut dotting = vk::PhysicalDeviceShaderIntegerDotProductFeatures::default();
    let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut dotting);
    unsafe { instance.get_physical_device_features2(physical, &mut features) };
    if dotting.shader_integer_dot_product == vk::TRUE {
        Dots::Native
    } else {
        Dots::Exact
    }
}

unsafe fn timestamps(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    family: u32,
) -> (f32, bool) {
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let bits = families
        .get(family as usize)
        .map_or(0, |one| one.timestamp_valid_bits);
    (properties.limits.timestamp_period, bits > 0)
}
