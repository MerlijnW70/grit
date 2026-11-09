#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ash::vk;

use grit::host::{API, Limits, OPERATIONS};

const EVERY: vk::SubgroupFeatureFlags = vk::SubgroupFeatureFlags::from_raw(
    vk::SubgroupFeatureFlags::BASIC.as_raw()
        | vk::SubgroupFeatureFlags::VOTE.as_raw()
        | vk::SubgroupFeatureFlags::ARITHMETIC.as_raw()
        | vk::SubgroupFeatureFlags::BALLOT.as_raw()
        | vk::SubgroupFeatureFlags::SHUFFLE.as_raw()
        | vk::SubgroupFeatureFlags::SHUFFLE_RELATIVE.as_raw()
        | vk::SubgroupFeatureFlags::CLUSTERED.as_raw()
        | vk::SubgroupFeatureFlags::QUAD.as_raw(),
);

fn device(api: u32, stages: vk::ShaderStageFlags, operations: vk::SubgroupFeatureFlags) -> Limits {
    let core = vk::PhysicalDeviceProperties {
        api_version: api,
        ..Default::default()
    };
    let subgroup = vk::PhysicalDeviceSubgroupProperties::default()
        .subgroup_size(32)
        .supported_stages(stages)
        .supported_operations(operations);
    let third = vk::PhysicalDeviceMaintenance3Properties::default();
    Limits::of(&core, &subgroup, &third)
}

const COMPUTE: vk::ShaderStageFlags = vk::ShaderStageFlags::COMPUTE;

#[test]
fn a_device_without_subgroup_arithmetic_is_not_chosen() {
    let lacking = EVERY & !vk::SubgroupFeatureFlags::ARITHMETIC;
    assert!(!device(vk::API_VERSION_1_3, COMPUTE, lacking).serves());
    assert!(device(vk::API_VERSION_1_3, COMPUTE, EVERY).serves());
}

#[test]
fn the_kernels_need_basic_vote_and_arithmetic_and_nothing_else() {
    assert_eq!(
        OPERATIONS,
        vk::SubgroupFeatureFlags::BASIC
            | vk::SubgroupFeatureFlags::VOTE
            | vk::SubgroupFeatureFlags::ARITHMETIC
    );
    assert!(device(API, COMPUTE, OPERATIONS).serves());
    for missing in [
        vk::SubgroupFeatureFlags::BASIC,
        vk::SubgroupFeatureFlags::VOTE,
        vk::SubgroupFeatureFlags::ARITHMETIC,
    ] {
        let offered = OPERATIONS & !missing;
        assert!(
            !device(API, COMPUTE, offered).serves(),
            "{offered:?} was taken without {missing:?}"
        );
    }
}

#[test]
fn a_device_whose_subgroup_operations_stop_short_of_compute_is_not_chosen() {
    let stages = vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::VERTEX;
    assert!(!device(API, stages, EVERY).serves());
    assert!(!device(API, vk::ShaderStageFlags::empty(), EVERY).serves());
    assert!(device(API, stages | COMPUTE, EVERY).serves());
}

#[test]
fn a_device_below_vulkan_one_one_is_not_chosen() {
    assert_eq!(API, vk::API_VERSION_1_1);
    assert!(!device(vk::API_VERSION_1_0, COMPUTE, EVERY).serves());
    assert!(!device(vk::make_api_version(0, 1, 0, 999), COMPUTE, EVERY).serves());
    assert!(device(vk::API_VERSION_1_1, COMPUTE, EVERY).serves());
    assert!(device(vk::make_api_version(0, 2, 0, 0), COMPUTE, EVERY).serves());
    assert!(!device(vk::make_api_version(1, 1, 3, 0), COMPUTE, EVERY).serves());
}

#[test]
fn limits_carry_what_the_properties_say() {
    let mut core = vk::PhysicalDeviceProperties {
        api_version: vk::API_VERSION_1_2,
        ..Default::default()
    };
    core.limits.max_storage_buffer_range = 4_294_967_295;
    core.limits.max_compute_work_group_count = [65_535, 7, 9];
    let subgroup = vk::PhysicalDeviceSubgroupProperties::default()
        .subgroup_size(64)
        .supported_stages(COMPUTE)
        .supported_operations(OPERATIONS);
    let third = vk::PhysicalDeviceMaintenance3Properties::default()
        .max_memory_allocation_size(2_147_483_648);
    let limits = Limits::of(&core, &subgroup, &third);

    assert_eq!(limits.storage_range, 4_294_967_295);
    assert_eq!(limits.allocation, 2_147_483_648);
    assert_eq!(limits.ceiling(), 2_147_483_648);
    assert_eq!(limits.groups, 65_535);
    assert_eq!(limits.width, 64);
    assert_eq!(limits.stages, COMPUTE);
    assert_eq!(limits.operations, OPERATIONS);
    assert_eq!(limits.api, vk::API_VERSION_1_2);
    assert!(limits.serves());
}
