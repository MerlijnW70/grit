#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use ash::vk;

use grit::compact::Plan;
use grit::host::{Limit, Limits, OPERATIONS};
use grit::{Error, Groups, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const MARK: u32 = 0x5EED_BEEF;

struct Listed {
    limits: vk::PhysicalDeviceLimits,
    api: u32,
    allocation: u64,
    width: u32,
    stages: vk::ShaderStageFlags,
    operations: vk::SubgroupFeatureFlags,
}

fn listed(named: &str) -> Option<Listed> {
    let entry = unsafe { ash::Entry::load() }.unwrap();
    let application = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
    let info = vk::InstanceCreateInfo::default().application_info(&application);
    let instance = unsafe { entry.create_instance(&info, None) }.unwrap();
    let found = unsafe { instance.enumerate_physical_devices() }.unwrap();
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
        if name != named || held.is_some() {
            continue;
        }
        let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
        let mut third = vk::PhysicalDeviceMaintenance3Properties::default();
        let mut chained = vk::PhysicalDeviceProperties2::default()
            .push_next(&mut subgroup)
            .push_next(&mut third);
        unsafe { instance.get_physical_device_properties2(physical, &mut chained) };
        held = Some(Listed {
            limits: properties.limits,
            api: properties.api_version,
            allocation: third.max_memory_allocation_size,
            width: subgroup.subgroup_size,
            stages: subgroup.supported_stages,
            operations: subgroup.supported_operations,
        });
    }
    unsafe { instance.destroy_instance(None) };
    held
}

fn listed_for(host: &grit::Host) -> Listed {
    let name = host.name().to_owned();
    let Some(listed) = listed(&name) else {
        panic!("Vulkan opened {name:?} and does not list it again");
    };
    listed
}

fn mark(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    let value = kernel.module().constant_u32(MARK)?;
    kernel.store_at(0, head, value)?;
    kernel.finish()
}

fn widest(width: u32) -> Plan {
    let per_block = sweep::cells(width);
    Plan::over(width, per_block * per_block).unwrap()
}

fn widest_binding(plan: &Plan) -> (usize, usize) {
    plan.sizes()
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|&(at, words)| (words, std::cmp::Reverse(at)))
        .unwrap()
}

#[test]
fn the_host_keeps_the_limits_vulkan_lists_for_its_device() {
    let Some(host) = device("the_host_keeps_the_limits_vulkan_lists_for_its_device") else {
        return;
    };
    let listed = listed_for(&host);
    let kept = host.limits();
    eprintln!("{}: {kept:?}", host.name());

    let range = u64::from(listed.limits.max_storage_buffer_range);
    assert_eq!(kept.storage_range, range);
    assert_eq!(kept.allocation, listed.allocation);
    assert_eq!(kept.groups, listed.limits.max_compute_work_group_count[0]);
    assert_eq!(kept.width, listed.width);
    assert_eq!(kept.width, host.width());
    assert_eq!(kept.stages, listed.stages);
    assert_eq!(kept.operations, listed.operations);
    assert_eq!(kept.api, listed.api);
    assert_eq!(host.ceiling(), range.min(listed.allocation));
    assert_eq!(kept.ceiling(), host.ceiling());

    assert!(
        kept.serves(),
        "{} was chosen and cannot run the kernels",
        host.name()
    );
    assert!(kept.stages.contains(vk::ShaderStageFlags::COMPUTE));
    assert!(kept.operations.contains(OPERATIONS));
    assert!(
        (
            vk::api_version_major(kept.api),
            vk::api_version_minor(kept.api)
        ) >= (1, 1)
    );
}

#[test]
fn the_widest_plan_is_refused_at_creation_where_its_binding_passes_the_ceiling() {
    let Some(host) =
        device("the_widest_plan_is_refused_at_creation_where_its_binding_passes_the_ceiling")
    else {
        return;
    };
    let name = host.name().to_owned();
    let listed = listed_for(&host);
    let plan = widest(host.width());
    let sizes = plan.sizes();
    let (at, words) = widest_binding(&plan);
    let bytes = u64::try_from(words).unwrap() * 4;
    let range = u64::from(listed.limits.max_storage_buffer_range);
    let fits = bytes <= range && bytes <= listed.allocation;
    eprintln!(
        "{name}: widest binding {at} holds {words} words, {bytes} bytes; range {range}, allocation {}",
        listed.allocation
    );

    let admitted = host.limits().admits(&sizes, None);
    if fits {
        assert!(admitted.is_ok(), "{name}: {admitted:?}");
    } else {
        let limit = if bytes > range {
            Limit::StorageBufferRange
        } else {
            Limit::MemoryAllocationSize
        };
        let allowed = if bytes > range {
            range
        } else {
            listed.allocation
        };
        let wanted = |refused: &Result<_, Error>| {
            matches!(
                refused,
                Err(Error::TooLarge { binding, words: held, limit: named, allowed: most })
                    if *binding == at && *held == words && *named == limit && *most == allowed
            )
        };
        assert!(wanted(&admitted), "{name}: {admitted:?}");
        let made = host.resident(&sizes, &[]).map(|_| ());
        assert!(wanted(&made), "{name}: {made:?}");
    }

    if name.contains("Radeon") {
        assert!(
            !fits,
            "{name}: the widest plan was expected to pass the ceiling"
        );
        assert_eq!(bytes, 4_294_967_296);
        assert_eq!(range, 4_294_967_295);
    }
    if name.contains("4080") {
        assert!(fits, "{name}: the widest plan was expected to fit");
        assert_eq!(bytes, 536_870_912);
    }
}

#[test]
fn the_widest_plan_is_checked_against_the_workgroups_a_device_promises() {
    let Some(host) = device("the_widest_plan_is_checked_against_the_workgroups_a_device_promises")
    else {
        return;
    };
    let listed = listed_for(&host);
    let promised = listed.limits.max_compute_work_group_count[0];
    let plan = widest(host.width());
    let groups = plan.groups();
    let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();

    let checked = host.limits().dispatchable(&stages);
    if plan.blocks <= promised {
        assert!(checked.is_ok(), "{checked:?}");
    } else {
        assert!(
            matches!(checked, Err(Error::TooManyGroups { pass: 0, groups, allowed })
                if groups == plan.blocks && allowed == promised),
            "{checked:?}"
        );
    }
}

#[test]
fn a_binding_past_the_storage_range_is_refused_before_anything_is_allocated() {
    let Some(host) =
        device("a_binding_past_the_storage_range_is_refused_before_anything_is_allocated")
    else {
        return;
    };
    let range = host.limits().storage_range;
    let over = usize::try_from(range / 4 + 1).unwrap();
    let huge = 16 << 30;
    let module = mark(host.width()).unwrap();

    let refused = |made: Result<(), Error>, binding: usize, words: usize| {
        assert!(
            matches!(
                made,
                Err(Error::TooLarge { binding: at, words: held, limit: Limit::StorageBufferRange, allowed })
                    if at == binding && held == words && allowed == range
            ),
            "{made:?}"
        );
    };

    refused(host.resident(&[over], &[]).map(|_| ()), 0, over);
    refused(host.resident(&[4, huge], &[&module]).map(|_| ()), 1, huge);
    refused(host.resident_direct(&[4, over], &[]).map(|_| ()), 1, over);
    refused(
        host.resident_near(&[over, 4], &[], 1 << 20).map(|_| ()),
        0,
        over,
    );
    if host.sparse() {
        refused(host.resident_vast(&[4, over], &[], 1).map(|_| ()), 1, over);
    }

    let mut held = host.resident(&[4], &[&module]).unwrap();
    held.write(0, &[0; 4]).unwrap();
    held.frame(&[Groups::Fixed(1)]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![MARK]);
}

#[test]
fn a_dispatch_above_the_workgroup_limit_is_refused_when_it_is_recorded() {
    let Some(host) = device("a_dispatch_above_the_workgroup_limit_is_refused_when_it_is_recorded")
    else {
        return;
    };
    let limit = host.limits().groups;
    assert_eq!(
        limit,
        listed_for(&host).limits.max_compute_work_group_count[0]
    );
    let module = mark(host.width()).unwrap();
    let mut held = host.resident(&[4], &[&module]).unwrap();
    held.write(0, &[0; 4]).unwrap();

    let Some(past) = limit.checked_add(1) else {
        assert_eq!(limit, u32::MAX);
        let most = [Groups::Fixed(u32::MAX)];
        assert!(host.limits().dispatchable(&[&most]).is_ok());
        eprintln!(
            "{}: maxComputeWorkGroupCount[0] is {limit}, so no Groups::Fixed count passes it",
            host.name()
        );
        return;
    };

    for refused in [Groups::Fixed(past), Groups::Counting(past)] {
        let made = held.frame(&[refused]);
        assert!(
            matches!(made, Err(Error::TooManyGroups { pass: 0, groups, allowed })
                if groups == past && allowed == limit),
            "{made:?}"
        );
        assert_eq!(held.in_flight(), 0);
        assert_eq!(held.read(0, 1).unwrap(), vec![0]);
    }

    let most = [Groups::Fixed(limit)];
    assert!(host.limits().dispatchable(&[&most]).is_ok());
    held.frame(&[Groups::Fixed(1)]).unwrap();
    assert_eq!(held.read(0, 1).unwrap(), vec![MARK]);
}

#[test]
fn limits_built_from_the_listed_properties_equal_the_hosts() {
    let Some(host) = device("limits_built_from_the_listed_properties_equal_the_hosts") else {
        return;
    };
    let listed = listed_for(&host);
    let core = vk::PhysicalDeviceProperties {
        api_version: listed.api,
        limits: listed.limits,
        ..Default::default()
    };
    let subgroup = vk::PhysicalDeviceSubgroupProperties::default()
        .subgroup_size(listed.width)
        .supported_stages(listed.stages)
        .supported_operations(listed.operations);
    let third = vk::PhysicalDeviceMaintenance3Properties::default()
        .max_memory_allocation_size(listed.allocation);
    let built: Limits = Limits::of(&core, &subgroup, &third);
    assert_eq!(built, host.limits());
}
