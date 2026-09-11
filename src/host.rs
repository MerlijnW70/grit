use std::ffi::{CStr, c_char};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ash::vk;

#[derive(Debug)]
pub enum Error {
    NoLoader(ash::LoadingError),
    Vulkan(vk::Result),
    NoComputeDevice,
    NoSuchDevice {
        wanted: String,
        present: Vec<String>,
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
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLoader(why) => write!(out, "{why}"),
            Self::Vulkan(code) => write!(out, "{code:?}"),
            Self::NoSparse => write!(out, "{self:?}"),
            Self::NoComputeDevice => write!(out, "{self:?}"),
            Self::NoSuchDevice { wanted, present } => {
                write!(out, "{wanted:?} {present:?}")
            }
            Self::NoMemory { bytes } => write!(out, "{bytes}"),
            Self::NoSuchBinding { index, held } => {
                write!(out, "{index} {held}")
            }
            Self::Overrun {
                index,
                needed,
                held,
            } => write!(out, "{index} {needed} {held}"),
            Self::Staged { needed, staging } => write!(out, "{needed} {staging}"),
            Self::WrongPasses { given, held } => {
                write!(out, "{given} {held}")
            }
            Self::StillInFlight => write!(out, "{self:?}"),
            Self::Wedged { after } => write!(out, "{after:?}"),
            Self::WrongOutputs { asked, wanted } => {
                write!(out, "{asked} {wanted}")
            }
            Self::NoSuchTicket { ticket, held } => match held {
                Some(one) => write!(out, "{ticket} {one}"),
                None => write!(out, "{ticket}"),
            },
        }
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(code: vk::Result) -> Self {
        Self::Vulkan(code)
    }
}

pub struct Host {
    entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    name: String,
    width: u32,
    ceiling: u64,
    tick: f32,
    timing: bool,
    wedged: AtomicBool,
    sparse: bool,
    int64: bool,
}

impl Host {
    pub fn open() -> Result<Option<Self>, Error> {
        Self::matching(std::env::var("GRIT_DEVICE").ok().as_deref())
    }

    pub fn matching(pattern: Option<&str>) -> Result<Option<Self>, Error> {
        let entry = match unsafe { ash::Entry::load() } {
            Ok(entry) => entry,
            Err(_) => return Ok(None),
        };

        let application =
            vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
        let create = vk::InstanceCreateInfo::default().application_info(&application);

        let instance = match unsafe { entry.create_instance(&create, None) } {
            Ok(instance) => instance,
            Err(_) => return Ok(None),
        };

        match unsafe { Self::on(entry, instance, pattern) } {
            Ok(host) => Ok(Some(host)),
            Err(why) => Err(why),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
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
    pub const fn ceiling(&self) -> u64 {
        self.ceiling
    }

    unsafe fn on(
        entry: ash::Entry,
        instance: ash::Instance,
        pattern: Option<&str>,
    ) -> Result<Self, Error> {
        let candidates = unsafe { instance.enumerate_physical_devices() }?;

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

        let chosen = compute
            .iter()
            .copied()
            .filter(|&(physical, _)| {
                wanted
                    .as_deref()
                    .is_none_or(|wanted| named(physical).to_lowercase().contains(wanted))
            })
            .max_by_key(|&(physical, _)| {
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                u8::from(properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU)
            });

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
        let width = unsafe { subgroup_width(&instance, physical) };
        let ceiling = unsafe { allocation_ceiling(&instance, physical) };
        let (tick, timing) = unsafe { timestamps(&instance, physical, family) };

        let priorities = [1.0_f32];
        let has = unsafe { instance.get_physical_device_features(physical) };
        let wanted = vk::PhysicalDeviceFeatures::default()
            .sparse_binding(has.sparse_binding == 1)
            .sparse_residency_buffer(has.sparse_binding == 1 && has.sparse_residency_buffer == 1)
            .shader_int64(has.shader_int64 == 1);
        let sparse = wanted.sparse_binding == 1 && wanted.sparse_residency_buffer == 1;
        let int64 = wanted.shader_int64 == 1;

        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let create = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_features(&wanted);

        let device = match unsafe { instance.create_device(physical, &create, None) } {
            Ok(device) => device,
            Err(code) => {
                unsafe { instance.destroy_instance(None) };
                return Err(code.into());
            }
        };
        let queue = unsafe { device.get_device_queue(family, 0) };

        Ok(Self {
            entry,
            instance,
            device,
            queue,
            family,
            memory,
            name,
            width,
            ceiling,
            tick,
            timing,
            sparse,
            int64,
            wedged: AtomicBool::new(false),
        })
    }

    pub(crate) fn wedge(&self) {
        self.wedged.store(true, Ordering::SeqCst);
    }

    pub(crate) fn wedged(&self) -> bool {
        self.wedged.load(Ordering::SeqCst)
    }

    pub(crate) const fn ticking(&self) -> (f32, bool) {
        (self.tick, self.timing)
    }

    #[must_use]
    pub const fn sparse(&self) -> bool {
        self.sparse
    }

    #[must_use]
    pub const fn int64(&self) -> bool {
        self.int64
    }

    pub(crate) const fn device(&self) -> &ash::Device {
        &self.device
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
    fn drop(&mut self) {
        if self.wedged() {
            return;
        }
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
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

unsafe fn allocation_ceiling(instance: &ash::Instance, physical: vk::PhysicalDevice) -> u64 {
    let mut third = vk::PhysicalDeviceMaintenance3Properties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut third);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    let addressable = u64::from(properties.properties.limits.max_storage_buffer_range);
    third.max_memory_allocation_size.min(addressable)
}

unsafe fn subgroup_width(instance: &ash::Instance, physical: vk::PhysicalDevice) -> u32 {
    let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut subgroup);
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    subgroup.subgroup_size
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
