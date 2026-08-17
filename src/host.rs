//! Several modules over buffers that never leave the device.
//!
//! # Why this exists, when `runner` is right there
//!
//! `simdr`'s own `runner` is a **measurement** harness, and every entry point it offers is shaped
//! by that. `Gpu::run_grid` uploads, dispatches, downloads. `Scanner::scan` and `Reducer::sum` hold
//! their pipelines and buffers across calls but still take `&[f32]` and hand back a `Vec` — what
//! they keep is the *setup*, not the *data*. `Gpu::run_chain` runs several modules in order and
//! round-trips the whole thing. `Session` is the one that keeps device-local buffers across calls,
//! and it holds exactly one module.
//!
//! None of that is a gap in `runner`. `decisions/DR-0008` in that repository priced a round trip at
//! about a hundred microseconds before any arithmetic, and every one of those entry points exists
//! to put that price where a caller can see it.
//!
//! A simulation is the other shape. Sixteen million cells of state, five kernels a frame, sixty
//! frames a second — the state must **never** cross the bus, and only the picture may. That is one
//! primitive `runner` does not have, and it is this file: *N pipelines, one descriptor set, buffers
//! that outlive the frame, and one submission that records every pass with a barrier between.*
//!
//! # What it deliberately does not do
//!
//! No swapchain, no graphics pipeline, no vertex. The picture is a storage buffer the last pass
//! writes and the host copies to a window. That is the same arrangement the deleted `demo` used and
//! it costs a copy of the framebuffer per frame — about three milliseconds for eight megabytes on
//! this machine, which at 1920 × 1080 is the entire budget's worth of headroom and no more.
//! Presentation is where this would go next if the frame time ever needs it.

use std::ffi::{CStr, c_char};

use ash::vk;

/// What went wrong, in the caller's terms.
#[derive(Debug)]
pub enum Error {
    /// The Vulkan loader is not there.
    NoLoader(ash::LoadingError),
    /// Vulkan answered, and said no.
    Vulkan(vk::Result),
    /// No physical device offers a compute queue.
    NoComputeDevice,
    /// A device was asked for by name and none matched.
    NoSuchDevice {
        /// What was asked for.
        wanted: String,
        /// What is actually here.
        present: Vec<String>,
    },
    /// No memory type satisfies what a buffer needs.
    NoMemory {
        /// How many bytes were wanted.
        bytes: u64,
    },
    /// A binding index past the last buffer.
    NoSuchBinding {
        /// What was asked for.
        index: usize,
        /// How many there are.
        held: usize,
    },
    /// More words than the buffer holds.
    Overrun {
        /// The binding.
        index: usize,
        /// Words asked for.
        needed: usize,
        /// Words the buffer has.
        held: usize,
    },
    /// A pass count that does not match the pipelines held.
    WrongPasses {
        /// Workgroup counts given.
        given: usize,
        /// Pipelines held.
        held: usize,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLoader(why) => write!(out, "no Vulkan loader: {why}"),
            Self::Vulkan(code) => write!(out, "Vulkan said {code:?}"),
            Self::NoComputeDevice => write!(out, "no physical device offers a compute queue"),
            Self::NoSuchDevice { wanted, present } => {
                write!(out, "no device matching {wanted:?}; here: {present:?}")
            }
            Self::NoMemory { bytes } => write!(out, "no memory type holds {bytes} bytes"),
            Self::NoSuchBinding { index, held } => {
                write!(out, "binding {index} of {held}")
            }
            Self::Overrun {
                index,
                needed,
                held,
            } => write!(out, "binding {index} wants {needed} words and holds {held}"),
            Self::WrongPasses { given, held } => {
                write!(out, "{given} workgroup counts for {held} passes")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<vk::Result> for Error {
    fn from(code: vk::Result) -> Self {
        Self::Vulkan(code)
    }
}

/// A device, opened once and kept.
pub struct Host {
    /// Kept because the device outlives nothing else and must be destroyed after it.
    entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    name: String,
    width: u32,
}

impl Host {
    /// Open a compute device, preferring a discrete one, or `None` if Vulkan is not here at all.
    ///
    /// `GRIT_DEVICE` selects by substring, the way `SIMDR_DEVICE` does next door.
    ///
    /// # Errors
    ///
    /// [`Error::NoComputeDevice`] if nothing offers a compute queue, [`Error::NoSuchDevice`] if a
    /// pattern matched nothing, otherwise whatever Vulkan said.
    pub fn open() -> Result<Option<Self>, Error> {
        // SAFETY: `Entry::load` reads the loader the platform installed. Nothing else has run yet.
        let entry = match unsafe { ash::Entry::load() } {
            Ok(entry) => entry,
            Err(_) => return Ok(None),
        };

        let application =
            vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
        let create = vk::InstanceCreateInfo::default().application_info(&application);

        // SAFETY: `create` borrows `application`, which outlives this call.
        let instance = match unsafe { entry.create_instance(&create, None) } {
            Ok(instance) => instance,
            Err(_) => return Ok(None),
        };

        // SAFETY: the instance is live for the rest of this function, and is destroyed on every
        // path that does not hand it to a `Host`.
        match unsafe {
            Self::on(
                entry,
                instance,
                std::env::var("GRIT_DEVICE").ok().as_deref(),
            )
        } {
            Ok(host) => Ok(Some(host)),
            Err(why) => Err(why),
        }
    }

    /// This device's name, as it reports it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many lanes a subgroup has here — the number every `simdr` module is built for.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Choose a device and create it.
    ///
    /// # Safety
    ///
    /// `instance` must be live and not yet handed to anything else.
    unsafe fn on(
        entry: ash::Entry,
        instance: ash::Instance,
        pattern: Option<&str>,
    ) -> Result<Self, Error> {
        // SAFETY: the caller's contract.
        let candidates = unsafe { instance.enumerate_physical_devices() }?;

        let compute: Vec<(vk::PhysicalDevice, u32)> = candidates
            .into_iter()
            .filter_map(|physical| {
                // SAFETY: `physical` came from this instance's own enumeration.
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let family = families.iter().position(|family| {
                    family.queue_flags.contains(vk::QueueFlags::COMPUTE) && family.queue_count > 0
                })?;
                u32::try_from(family).ok().map(|family| (physical, family))
            })
            .collect();

        if compute.is_empty() {
            // SAFETY: nothing was created from it.
            unsafe { instance.destroy_instance(None) };
            return Err(Error::NoComputeDevice);
        }

        // SAFETY: as above.
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
            // A discrete part when nothing was asked for, and among the matches when something was.
            .max_by_key(|&(physical, _)| {
                // SAFETY: as above.
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                u8::from(properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU)
            });

        let Some((physical, family)) = chosen else {
            let present = compute
                .iter()
                .map(|&(physical, _)| named(physical))
                .collect();
            // SAFETY: as above.
            unsafe { instance.destroy_instance(None) };
            return Err(Error::NoSuchDevice {
                wanted: pattern.unwrap_or_default().to_owned(),
                present,
            });
        };

        // SAFETY: a handle this instance enumerated.
        let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
        // SAFETY: as above.
        let name = unsafe { name_of(&instance, physical) };
        // SAFETY: as above.
        let width = unsafe { subgroup_width(&instance, physical) };

        let priorities = [1.0_f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let create = vk::DeviceCreateInfo::default().queue_create_infos(&queues);

        // SAFETY: `create` borrows `queues`, which outlives the call.
        let device = match unsafe { instance.create_device(physical, &create, None) } {
            Ok(device) => device,
            Err(code) => {
                // SAFETY: no device was created from it.
                unsafe { instance.destroy_instance(None) };
                return Err(code.into());
            }
        };
        // SAFETY: the family and index are the ones just created.
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
        })
    }

    /// The device, for the layer that allocates over it.
    pub(crate) const fn device(&self) -> &ash::Device {
        &self.device
    }

    /// The queue every submission goes to.
    pub(crate) const fn queue(&self) -> vk::Queue {
        self.queue
    }

    /// The queue family the command pool belongs to.
    pub(crate) const fn family(&self) -> u32 {
        self.family
    }

    /// Which memory type suits `requirements` and offers `wanted`.
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
        // SAFETY: every `Resident` borrows the `Host`, so none can be alive here.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
        // The loader is kept alive until here on purpose: `instance` is a function table into it.
        let _ = &self.entry;
    }
}

/// A device's name, or a placeholder if it declines to give one.
///
/// # Safety
///
/// `physical` must be a handle `instance` enumerated.
unsafe fn name_of(instance: &ash::Instance, physical: vk::PhysicalDevice) -> String {
    // SAFETY: the caller's contract.
    let properties = unsafe { instance.get_physical_device_properties(physical) };
    let bytes: &[c_char] = &properties.device_name;
    // SAFETY: Vulkan guarantees this array is NUL-terminated.
    let name = unsafe { CStr::from_ptr(bytes.as_ptr()) };
    name.to_string_lossy().into_owned()
}

/// How many lanes a subgroup has on this device.
///
/// # Safety
///
/// As [`name_of`].
unsafe fn subgroup_width(instance: &ash::Instance, physical: vk::PhysicalDevice) -> u32 {
    let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut subgroup);
    // SAFETY: the caller's contract, and `properties` borrows `subgroup` for the call.
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    subgroup.subgroup_size
}
