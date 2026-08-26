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
//! **That hundred is a round trip that moves data**, which is what `runner`'s entry points all do,
//! and this crate quoted it for years as the price of a bare submit and fence. `tests/floor.rs`
//! prices the two apart: a submission holding nothing is 35-42 µs here, one that copies a megabyte back
//! is 95. Both make the same argument, and only one of them is what a resident frame pays.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
    /// A submission was started while one was still in flight.
    ///
    /// One command buffer and one fence, so one at a time. Waiting here instead of saying so would
    /// make a caller's overlap silently disappear the day it stopped fitting — which is the failure
    /// this is for, since nothing else about it is visible.
    StillInFlight,
    /// Waited on a submission that was not the one outstanding.
    ///
    /// A ticket is only good until it is waited on, and only for the submission that made it.
    NoSuchTicket {
        /// What was waited on.
        ticket: u64,
        /// What is outstanding, if anything is.
        held: Option<u64>,
    },
    /// A frame's answers asked for in a shape it was not submitted with.
    ///
    /// A frame copies out the bindings it was started with and no others, so waiting for two out of
    /// one is a caller's mistake rather than a device's — and handing back an empty second would be
    /// a wrong answer that reads like a right one.
    WrongOutputs {
        /// How many the frame copied.
        asked: usize,
        /// How many were wanted.
        wanted: usize,
    },
    /// A submission did not land inside the patience, so the device is presumed lost.
    ///
    /// Vulkan offers no way to cancel a submission or to take a fence back. Work handed to the
    /// queue is the device's until it finishes, and where it never finishes the command buffer, the
    /// fence and every buffer bound to them stay the device's for good. So this is not an error a
    /// caller retries past: the [`Resident`](crate::Resident) that raised it is poisoned, every
    /// later call on it says this again, and both teardowns leak rather than destroy resources a
    /// submission still holds.
    ///
    /// It exists because a fence waited on with no timeout is a wait with no way out: if the
    /// submission never signals, the calling thread parks inside the driver and the process has no
    /// remaining move. Bounding it turns that into an error a caller can act on.
    ///
    /// **It was first written for a failure it turned out not to explain.** `hyperia`'s mutation
    /// gate was losing sandboxes to test processes that outlived their kill, and this looked like
    /// the cause. It was not: those processes were building kernels a thousand times too large
    /// after a `/`→`*` mutant, and escaping `taskkill /T` because their intermediate parent had
    /// already exited. The bound is still right, and it fixed none of that.
    Wedged {
        /// How long the fence was waited on before it was given up on.
        after: Duration,
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
            Self::StillInFlight => write!(out, "a submission is still in flight"),
            Self::Wedged { after } => write!(
                out,
                "a submission did not land in {after:?}; the device is presumed lost"
            ),
            Self::WrongOutputs { asked, wanted } => {
                write!(out, "the frame copied {asked} buffer(s) and {wanted} were wanted")
            }
            Self::NoSuchTicket { ticket, held } => match held {
                Some(one) => write!(out, "waited on {ticket}, and {one} is outstanding"),
                None => write!(out, "waited on {ticket}, and nothing is outstanding"),
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
    ceiling: u64,
    /// Whether a submission has been given up on, which nothing ever takes back.
    ///
    /// One flag on the host rather than one per `Resident`, because what it guards is the device:
    /// a queue with work on it that will never finish is not a thing one resident can be poisoned
    /// about while its neighbours carry on.
    wedged: AtomicBool,
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

    /// How much memory this device has of its own, in bytes.
    ///
    /// The largest `DEVICE_LOCAL` heap it reports. On a discrete card that is its memory; on an
    /// integrated one it is a share of the system's, and the number is the driver's claim either
    /// way rather than a measurement.
    ///
    /// # Why this is not [`Host::ceiling`]
    ///
    /// Because they answer different questions and are far apart. The ceiling is the largest single
    /// *allocation* — about four gigabytes on the card this was written against, where the heap is
    /// sixteen. A caller sizing a cache wants this one; a caller sizing a buffer wants the other.
    /// `hyperia` was choosing a slot count from a constant because there was no way to ask.
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

    /// The largest single buffer this device will both allocate and address.
    ///
    /// The smaller of `maxMemoryAllocationSize` and `maxStorageBufferRange`: one bounds what
    /// `vkAllocateMemory` will hand back and the other bounds what one binding can reach, and a
    /// storage buffer has to satisfy both.
    ///
    /// **A caller with more to hold than this has to hold it in pieces**, and the number is not
    /// guessable — on this machine's parts it is 3.997 GiB and 2.00 GiB, and neither is a round
    /// fraction of the card. `hyperia::fit` is what turns it into a layout.
    #[must_use]
    pub const fn ceiling(&self) -> u64 {
        self.ceiling
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
        // SAFETY: as above.
        let ceiling = unsafe { allocation_ceiling(&instance, physical) };

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
            ceiling,
            wedged: AtomicBool::new(false),
        })
    }

    /// Say that a submission was given up on. Nothing takes this back.
    pub(crate) fn wedge(&self) {
        self.wedged.store(true, Ordering::SeqCst);
    }

    /// Whether a submission has been given up on.
    pub(crate) fn wedged(&self) -> bool {
        self.wedged.load(Ordering::SeqCst)
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
        // **A wedged device is leaked on purpose.** `device_wait_idle` waits for every queue to
        // drain and a wedged one never does, so this is the second place a process would park in
        // the driver and never come out — and `destroy_device` while work is still outstanding is
        // undefined behaviour rather than a tidy-up. Leaking an instance the process is about to
        // drop anyway is the only ending here that is neither a hang nor undefined. `Error::Wedged`.
        if self.wedged() {
            return;
        }
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

/// The largest single buffer this device will both allocate and address.
///
/// # Safety
///
/// As [`name_of`].
unsafe fn allocation_ceiling(instance: &ash::Instance, physical: vk::PhysicalDevice) -> u64 {
    let mut third = vk::PhysicalDeviceMaintenance3Properties::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut third);
    // SAFETY: the caller's contract, and `properties` borrows `third` for the call.
    unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
    let addressable = u64::from(properties.properties.limits.max_storage_buffer_range);
    third.max_memory_allocation_size.min(addressable)
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
