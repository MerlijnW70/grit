#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ash::vk;

use grit::compact::Plan;
use grit::host::{API, OPERATIONS};
use grit::netlist::Netlist;
use grit::rule::{Kind, Pass};
use grit::sand::{Grid, View};

const CAPABILITY: u32 = 17;

fn declared(module: &[u32]) -> (u32, Vec<u32>) {
    let mut found = Vec::new();
    let mut at = 5;
    while at < module.len() {
        let word = module[at];
        let length = (word >> 16) as usize;
        if word & 0xFFFF == CAPABILITY {
            found.push(module[at + 1]);
        }
        at += length.max(1);
    }
    (module[1], found)
}

fn operation(capability: u32) -> Option<vk::SubgroupFeatureFlags> {
    match capability {
        61 => Some(vk::SubgroupFeatureFlags::BASIC),
        62 => Some(vk::SubgroupFeatureFlags::VOTE),
        63 => Some(vk::SubgroupFeatureFlags::ARITHMETIC),
        64 => Some(vk::SubgroupFeatureFlags::BALLOT),
        65 => Some(vk::SubgroupFeatureFlags::SHUFFLE),
        66 => Some(vk::SubgroupFeatureFlags::SHUFFLE_RELATIVE),
        67 => Some(vk::SubgroupFeatureFlags::CLUSTERED),
        68 => Some(vk::SubgroupFeatureFlags::QUAD),
        _ => None,
    }
}

fn engine(subgroup: u32) -> Vec<Vec<u32>> {
    let mut modules = Vec::new();
    for side in [64_u32, 4096] {
        let grid = Grid::of(side, side);
        let Ok(plan) = Plan::over(
            subgroup,
            grid.quads().div_ceil(grit::sweep::cells(subgroup)),
        ) else {
            continue;
        };
        let both = grit::live::both(subgroup, grid, &plan, Some(View::of(64, 64))).unwrap();
        modules.extend(both.passes);
        for coarse in [false, true] {
            let kept = grit::live::keeping(subgroup, &plan, coarse).unwrap();
            for stage in kept.before.into_iter().chain(kept.after) {
                modules.extend(stage.into_iter().map(|(module, _)| module));
            }
        }
        modules.push(grit::sand::settle(subgroup, grid).unwrap());
        for pass in [Pass::Ahead, Pass::Back] {
            for kind in KINDS {
                modules.push(grit::rule::built(subgroup, grid, pass, kind).unwrap());
            }
        }
    }
    modules.push(grit::sweep::sweep(subgroup).unwrap());
    let shape = Netlist::over(4, 2, 8, 16).unwrap();
    modules.push(grit::marking::marked(subgroup, shape).unwrap());
    modules.push(grit::marking::turned_on(subgroup, shape).unwrap());
    modules
}

const KINDS: [Kind; 7] = [
    Kind::Product,
    Kind::Sum,
    Kind::Turn,
    Kind::Bound,
    Kind::Smooth,
    Kind::Living,
    Kind::Twinned,
];

#[test]
fn the_engine_kernels_use_exactly_the_subgroup_operations_a_device_is_chosen_for() {
    for subgroup in [4_u32, 8, 16, 32, 64] {
        let modules = engine(subgroup);
        assert!(modules.len() > 20, "{subgroup}: {} modules", modules.len());
        let mut used = vk::SubgroupFeatureFlags::empty();
        for module in &modules {
            let (version, capabilities) = declared(module);
            assert!(version <= 0x0001_0300, "{subgroup}: SPIR-V {version:#x}");
            for capability in capabilities {
                if let Some(flag) = operation(capability) {
                    used |= flag;
                }
            }
        }
        assert_eq!(used, OPERATIONS, "{subgroup}");
    }
}

#[test]
fn spir_v_one_three_is_what_vulkan_one_one_takes() {
    assert_eq!(API, vk::API_VERSION_1_1);
    let module = grit::sweep::sweep(32).unwrap();
    assert_eq!(declared(&module).0, 0x0001_0300);
}
