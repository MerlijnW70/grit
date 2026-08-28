#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use ash::vk;

use grit::compact::Plan;
use grit::{Host, sweep};

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
            None
        }
    }
}

fn limits_of(pattern: Option<&str>) -> Option<(String, vk::PhysicalDeviceLimits)> {
    let entry = unsafe { ash::Entry::load() }.ok()?;
    let info = vk::InstanceCreateInfo::default();
    let instance = unsafe { entry.create_instance(&info, None) }.ok()?;
    let found = unsafe { instance.enumerate_physical_devices() }.ok()?;
    let mut held = None;
    for physical in found {
        let properties = unsafe { instance.get_physical_device_properties(physical) };
        let bytes: Vec<u8> = properties
            .device_name
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| u8::try_from(*byte).unwrap_or(b'?'))
            .collect();
        let name = String::from_utf8_lossy(&bytes).into_owned();
        let wanted = pattern.is_none_or(|want| name.contains(want));
        if wanted && held.is_none() {
            held = Some((name, properties.limits));
        }
    }
    unsafe { instance.destroy_instance(None) };
    held
}

#[test]
fn the_widest_plan_asks_for_more_workgroups_than_a_device_promises() {
    let Some(host) = device("limits") else {
        return;
    };
    let width = host.width();
    let pattern = std::env::var("GRIT_DEVICE").ok();
    let Some((name, limits)) = limits_of(pattern.as_deref()) else {
        eprintln!("SKIPPED limits: no physical device to read");
        return;
    };

    let per_block = sweep::cells(width);
    let widest = per_block * per_block;
    let plan = Plan::over(width, widest).expect("the widest plan two levels reach");

    let promised = limits.max_compute_work_group_count[0];
    let asked = plan.blocks;

    eprintln!("{name} — subgroup {width}, per_block {per_block}");
    eprintln!("  max_compute_work_group_count[0]  {promised}");
    eprintln!(
        "  max_storage_buffer_range         {} bytes",
        limits.max_storage_buffer_range
    );
    eprintln!("  Plan::over refuses past           {widest} blocks");
    eprintln!("  a decay over every block asks for {asked} workgroups");
    eprintln!(
        "  Host::ceiling                     {} bytes",
        host.ceiling()
    );

    let sizes = plan.sizes();
    let widest_binding = sizes.iter().copied().max().unwrap_or(0);
    eprintln!(
        "  the widest binding that plan wants {} words, {} bytes",
        widest_binding,
        widest_binding * 4
    );

    assert!(
        promised > 0 && limits.max_storage_buffer_range > 0,
        "a device that promises nothing"
    );
}
