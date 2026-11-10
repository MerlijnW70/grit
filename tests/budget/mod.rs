#![allow(dead_code, clippy::panic)]

use std::sync::{Mutex, MutexGuard, PoisonError};

use ash::vk;
use grit::{Error, Host, Resident};

pub const MIB: u64 = 1 << 20;

pub const SLACK: u64 = 16 * MIB;

static ALONE: Mutex<()> = Mutex::new(());

/// Heap usage is counted for the whole process, so every test in a binary
/// that reads it holds this lock while it makes anything.
pub fn alone() -> MutexGuard<'static, ()> {
    ALONE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bytes this process holds in the device-local heaps of the device
/// named `name`, read through a second instance with `VK_EXT_memory_budget`.
pub fn usage(name: &str) -> Result<u64, String> {
    let entry = unsafe { ash::Entry::load() }.map_err(|why| format!("no loader: {why}"))?;
    let application = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
    let create = vk::InstanceCreateInfo::default().application_info(&application);
    let instance = unsafe { entry.create_instance(&create, None) }
        .map_err(|code| format!("second instance: {code}"))?;
    let read = unsafe { usage_on(&instance, name) };
    unsafe { instance.destroy_instance(None) };
    read
}

unsafe fn usage_on(instance: &ash::Instance, name: &str) -> Result<u64, String> {
    let physicals = unsafe { instance.enumerate_physical_devices() }
        .map_err(|code| format!("enumerate: {code}"))?;
    let physical = physicals
        .into_iter()
        .find(|&physical| {
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            properties
                .device_name_as_c_str()
                .is_ok_and(|held| held.to_string_lossy() == name)
        })
        .ok_or_else(|| format!("the second instance has no device named {name:?}"))?;

    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }
        .map_err(|code| format!("extensions: {code}"))?;
    let budget = vk::EXT_MEMORY_BUDGET_NAME;
    if !extensions
        .iter()
        .any(|one| one.extension_name_as_c_str() == Ok(budget))
    {
        return Err(format!("{name} does not offer {budget:?}"));
    }

    let mut held = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
    let mut asked = vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut held);
    unsafe { instance.get_physical_device_memory_properties2(physical, &mut asked) };
    let heaps = asked.memory_properties;
    let count = heaps.memory_heap_count as usize;
    Ok(heaps
        .memory_heaps
        .iter()
        .zip(held.heap_usage.iter())
        .take(count)
        .filter(|(heap, _)| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|(_, &used)| used)
        .sum())
}

pub fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes / MIB)
}

/// Builds `control` once to warm the driver and to prove the heap usage
/// moves by at least half of `bytes` while it is held, then calls `failing`
/// three times. Every call must fail, and the device-local usage after the
/// third must be within `SLACK` of the usage before the first.
pub fn three_failures<'host>(
    host: &'host Host,
    bytes: u64,
    control: impl Fn() -> Result<Resident<'host>, Error>,
    failing: impl Fn() -> Result<Resident<'host>, Error>,
) {
    let name = host.name().to_owned();
    let before = match usage(&name) {
        Ok(used) => used,
        Err(why) => {
            crate::common::skip(why);
            return;
        }
    };
    let held = match control() {
        Ok(held) => held,
        Err(why) => panic!("the control resident of {} failed: {why}", mib(bytes)),
    };
    let holding = reading(&name);
    drop(held);
    let moved = holding.saturating_sub(before);
    eprintln!(
        "budget {name}: {} before, {} while the control of {} was held",
        mib(before),
        mib(holding),
        mib(bytes)
    );
    if moved < bytes / 2 {
        crate::common::skip(format!(
            "{name} reports {} of device-local heap usage moved while a resident of {} was held, so the budget extension does not count it here",
            mib(moved),
            mib(bytes)
        ));
        return;
    }

    let start = reading(&name);
    eprintln!("budget {name}: {} at the start", mib(start));
    for attempt in 1..=3 {
        let made = failing();
        let now = reading(&name);
        eprintln!(
            "budget {name}: attempt {attempt} -> {:?}; {} now",
            made.as_ref().err(),
            mib(now)
        );
        assert!(made.is_err(), "attempt {attempt} built a resident");
    }
    let end = reading(&name);
    eprintln!(
        "budget {name}: {} at the end, {} at the start",
        mib(end),
        mib(start)
    );
    assert!(
        end.abs_diff(start) <= SLACK,
        "device-local usage moved from {} to {} over three failed constructions",
        mib(start),
        mib(end)
    );
}

fn reading(name: &str) -> u64 {
    match usage(name) {
        Ok(used) => used,
        Err(why) => panic!("the budget read failed after it had worked: {why}"),
    }
}
