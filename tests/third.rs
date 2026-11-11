#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use ash::vk;
use common::device;

use grit::compact::{Plan, TooBig};
use grit::host::{Limit, Limits, OPERATIONS};
use grit::layout::{
    BINDINGS, Dimension, GRAND, PARAMS, PIXELS, STATE, TOTALS, WINDOW_BITS, Windows,
};
use grit::live;
use grit::sand::{AIR, Grid, SAND, WALL, WATER};
use grit::sand3::{self, Grid3, settle_reference3};
use grit::{Error, Groups, Host, Resident, sweep};

const RANGE: u64 = 4_294_967_295;

const RTX_4080_HEAP: u64 = 16_827_547_648;

const LAPTOP_HEAP: u64 = 8 << 30;

fn profile(allocation: u64) -> Limits {
    Limits {
        storage_range: RANGE,
        allocation,
        groups: u32::MAX,
        width: 32,
        stages: vk::ShaderStageFlags::COMPUTE,
        operations: OPERATIONS,
        api: vk::API_VERSION_1_3,
    }
}

fn rtx_4080() -> Limits {
    profile(u64::MAX)
}

fn laptop() -> Limits {
    profile(2 << 30)
}

fn plan_of(width: u32, grid: Grid) -> Result<Plan, TooBig> {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
}

/// The smallest solid world whose plan scans three levels deep at `width`:
/// a block holds `8 * width` octs and two levels reach that many cubed, so
/// the world holds twice that, `16 * (8 * width)^3` cells.
fn smallest(width: u32) -> Grid {
    let side = sweep::cells(width);
    Grid::solid(4 * side, 2 * side, 2 * side).unwrap()
}

#[test]
fn the_smallest_world_three_levels_deep_has_twice_the_octs_two_levels_reach() {
    for width in [4_u32, 8, 16, 32, 64] {
        let per_block = sweep::cells(width);
        let grid = smallest(width);
        assert_eq!(
            u64::from(grid.blocks()),
            2 * u64::from(per_block).pow(3),
            "{width}"
        );
        let plan = plan_of(width, grid).unwrap();
        assert_eq!(plan.levels(), 3, "{width}");
        assert_eq!(plan.supers(), 2, "{width}");
        let half = Grid::solid(2 * per_block, 2 * per_block, 2 * per_block).unwrap();
        assert_eq!(plan_of(width, half).unwrap().levels(), 2, "{width}");

        let two = per_block * per_block;
        let at = |blocks: u32| Plan::over_in(width, blocks, Dimension::Three);
        assert_eq!(at(two).unwrap().levels(), 2);
        assert_eq!(at(two + 1).unwrap().levels(), 3);
        let most = u64::from(per_block).pow(3);
        if let Ok(most) = u32::try_from(most) {
            let fits = at(most);
            let words = u64::from(most) * u64::from(per_block) * 8;
            if words <= Plan::MOST_WORDS {
                assert_eq!(fits.unwrap().levels(), 3);
            } else {
                assert!(matches!(fits, Err(TooBig::Cells { .. })), "{width}");
            }
            if let Some(past) = most.checked_add(1) {
                assert!(matches!(at(past), Err(TooBig::Blocks { .. })), "{width}");
            }
        }
        assert!(
            Plan::over(width, two + 1).is_err(),
            "a flat plan keeps two levels"
        );
    }
}

#[test]
fn a_third_level_lays_the_totals_of_the_groups_of_tiles_past_the_tiles() {
    for width in [4_u32, 8, 16, 32, 64] {
        let plan = plan_of(width, smallest(width)).unwrap();
        let per_block = plan.per_block as usize;
        assert_eq!(plan.upper() as usize, plan.supers() as usize * per_block);
        assert!(plan.upper() >= plan.tiles);
        assert_eq!(
            plan.sizes()[TOTALS as usize],
            plan.upper() as usize + per_block
        );
        assert!(plan.supers() <= plan.per_block);
        let two = Plan::over_in(width, plan.per_block * plan.per_block, Dimension::Three).unwrap();
        assert_eq!(two.sizes()[TOTALS as usize], per_block);
    }
}

#[test]
fn a_frame_three_levels_deep_wipes_scans_and_gathers_through_the_third_level() {
    let width = 4;
    let grid = smallest(width);
    let plan = plan_of(width, grid).unwrap();
    for coarse in [false, true] {
        let (_, stages) = live::arranged3(width, grid, &plan, coarse).unwrap();
        let groups: Vec<Groups> = stages.iter().flatten().copied().collect();
        assert!(groups.contains(&Groups::Fixed(plan.supers() + 1)));
        assert!(groups.contains(&Groups::Fixed(plan.supers())));
        let held = live::keeping(width, &plan, coarse).unwrap();
        assert_eq!(held.before[0].len(), 5);
        let scans = live::fine_scans(width, &plan).unwrap();
        assert_eq!(scans.len(), 3);
    }
    let two = Plan::over_in(width, plan.per_block * plan.per_block, Dimension::Three).unwrap();
    assert!(live::totals_wiped(width, &two).unwrap().is_none());
    assert_eq!(live::fine_scans(width, &two).unwrap().len(), 2);
    assert_eq!(live::coarse_scans(width, &two).unwrap().len(), 1);
    assert_eq!(live::coarse_scans(width, &plan).unwrap().len(), 2);
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 20 {
        0..8 => AIR,
        8..12 => WATER,
        12..17 => SAND,
        _ => WALL,
    };
    if kind == WALL {
        return WALL;
    }
    kind + ((((roll >> 8) % 128) as u32) << 2)
}

/// One cell in 1024 drawn at random, the rest air: octs in every tile and
/// every group of tiles move, and most sleep.
fn sparse(cells: usize, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..cells)
        .map(|_| {
            if next(&mut state).is_multiple_of(1024) {
                word(&mut state)
            } else {
                AIR
            }
        })
        .collect()
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding != PIXELS as usize {
            held.write(binding, &vec![0_u32; words]).unwrap();
        }
    }
}

struct Pair<'host> {
    live: Resident<'host>,
    full: Resident<'host>,
    stages: Vec<Vec<Groups>>,
    whole: [Groups; 2],
    plan: Plan,
    reference: Grid3,
    want: Vec<u32>,
    frame: u32,
}

impl<'host> Pair<'host> {
    fn new(host: &'host Host, width: u32, grid: Grid, plan: Plan, coarse: bool) -> Self {
        let (passes, stages) = live::arranged3(width, grid, &plan, coarse).unwrap();
        let built = sand3::settle(width, grid, host.limits().groups).unwrap();
        let tick = live::tick(width).unwrap();
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&plan.bound(), &modules).unwrap();
        let whole = Plan::over_in(width, plan.blocks, Dimension::Three).unwrap();
        let modules: Vec<&[u32]> = vec![&tick, &built.words];
        let full = host.resident(&whole.sizes(), &modules).unwrap();
        Self {
            live,
            full,
            stages,
            whole: [Groups::Fixed(1), built.spread.dispatch()],
            plan,
            reference: Grid3::new(grid.wide, grid.deep, grid.tall).unwrap(),
            want: Vec::new(),
            frame: 0,
        }
    }

    fn restart(&mut self, grid: Grid, start: &[u32]) {
        let bound = self.plan.bound();
        clear_all(&mut self.live, &bound);
        live::write_state(&mut self.live, self.plan.windows(), 0, start).unwrap();
        self.live
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
            .unwrap();
        live::seed(&mut self.live, grid, &self.plan).unwrap();
        let sizes = Plan::over_in(self.plan.subgroup, self.plan.blocks, Dimension::Three)
            .unwrap()
            .sizes();
        clear_all(&mut self.full, &sizes);
        self.full.write(STATE as usize, start).unwrap();
        self.full
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])
            .unwrap();
        self.want = start.to_vec();
        self.frame = 0;
    }

    /// One frame of each, and the first cell where the listed world, the
    /// whole grid or the CPU parts from the others.
    fn step(&mut self, label: &str) -> u32 {
        let cells = self.reference.cells();
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        self.live.staged(&groups).unwrap();
        let listed = live::read_state(&mut self.live, self.plan.windows(), cells).unwrap();
        let grand = self.live.read(GRAND as usize, 1).unwrap()[0];
        let stages: Vec<&[Groups]> = self.whole.iter().map(core::slice::from_ref).collect();
        let (_, whole) = self
            .full
            .staged_reading(&stages, STATE as usize, cells)
            .unwrap();
        self.want = settle_reference3(self.reference, self.frame, &self.want);
        for (what, got) in [("whole grid", &whole), ("listed", &listed)] {
            if let Some(at) = (0..cells).find(|&at| got[at] != self.want[at]) {
                let count = (0..cells).filter(|&at| got[at] != self.want[at]).count();
                panic!(
                    "{label}: frame {} cell {at}: {what} {:#x}, reference {:#x}; {count} cells differ",
                    self.frame, got[at], self.want[at]
                );
            }
        }
        self.frame += 1;
        grand
    }
}

const FRAMES: u32 = 10;

/// The widths no wider than the device's subgroup that its subgroup holds
/// whole, narrowest first: a workgroup that narrow is one subgroup with
/// that many lanes.
fn narrow(host: &Host) -> Vec<u32> {
    [4_u32, 8]
        .into_iter()
        .filter(|&width| width <= host.width() && host.width().is_multiple_of(width))
        .collect()
}

#[test]
fn the_listed_frame_equals_the_whole_grid_three_levels_deep() {
    let Some(host) = device("the_listed_frame_equals_the_whole_grid_three_levels_deep") else {
        return;
    };
    for width in narrow(&host) {
        let grid = smallest(width);
        let plan = plan_of(width, grid).unwrap();
        assert_eq!(plan.levels(), 3);
        assert!(!plan.windows().is_split());
        for coarse in [false, true] {
            let mut pair = Pair::new(&host, width, grid, plan, coarse);
            for seed in [1_u64, 0x5EED] {
                let start = sparse(grid.cells(), seed);
                pair.restart(grid, &start);
                let label = format!(
                    "{} width {width} {}x{}x{} coarse {coarse} seed {seed:#x}",
                    host.name(),
                    grid.wide,
                    grid.deep,
                    grid.tall
                );
                let mut fewest = u32::MAX;
                let mut most = 0;
                for _ in 0..FRAMES {
                    let listed = pair.step(&label);
                    fewest = fewest.min(listed);
                    most = most.max(listed);
                }
                assert_ne!(pair.want, start, "{label}: nothing moved");
                assert!(
                    most > plan.per_block * plan.per_block * plan.per_block / 64,
                    "{label}: the list never reached past the first group of tiles"
                );
                println!(
                    "{label}: {FRAMES} frames equal, {} blocks in {} tiles in {} groups, listed {fewest}..{most} of {}",
                    plan.blocks,
                    plan.tiles,
                    plan.supers(),
                    grid.blocks()
                );
            }
        }
    }
}

#[test]
fn release_plans_split_state_at_four_gibibytes_into_windows_of_two() {
    let width = 32;
    let cube = |side: u32| plan_of(width, Grid::solid(side, side, side).unwrap()).unwrap();

    let held = cube(512);
    assert!(!held.windows().is_split());
    assert_eq!(held.bound(), held.sizes().to_vec());
    assert_eq!(held.windows().bindings(), BINDINGS);

    let large = cube(1024);
    let windows = large.windows();
    assert_eq!(WINDOW_BITS, 29);
    assert_eq!(windows, Windows::of(1 << 30));
    assert_eq!(windows.count(), 2);
    assert_eq!(windows.bits(), WINDOW_BITS);
    let bound = large.bound();
    assert_eq!(bound.len(), BINDINGS as usize + 1);
    assert_eq!(bound[STATE as usize], 1 << 29);
    assert_eq!(bound[BINDINGS as usize], 1 << 29);
    assert_eq!(windows.place((1 << 29) - 1), (STATE, (1 << 29) - 1));
    assert_eq!(windows.place(1 << 29), (BINDINGS, 0));

    let widest = plan_of(width, Grid::solid(2048, 1024, 1024).unwrap()).unwrap();
    assert_eq!(widest.windows().count(), 4);
    assert_eq!(widest.bound().len(), BINDINGS as usize + 3);

    let under = Windows::of((1 << 30) - 1);
    assert!(!under.is_split());
    assert!(Windows::of(1 << 30).is_split());

    for side in [16_u32, 64, 256] {
        let grid = Grid::solid(side, side, side).unwrap();
        assert_eq!(
            live::settle3(width, grid).unwrap(),
            live::settle3_in(
                &grit::rule3::sand::Sand,
                width,
                grid,
                Windows::of(grid.cells())
            )
            .unwrap()
        );
    }
}

#[test]
fn a_flat_plan_keeps_its_state_whole_and_its_spare() {
    for width in [4_u32, 32] {
        let per_block = sweep::cells(width);
        let plan = Plan::over(width, per_block * per_block).unwrap();
        assert!(!plan.windows().is_split());
        assert_eq!(plan.sizes()[STATE as usize], plan.grid + plan.quads * 4);
    }
}

#[test]
fn the_4080_and_an_8_gibibyte_profile_admit_1024_cubed_split_and_refuse_it_whole() {
    let plan = plan_of(32, Grid::solid(1024, 1024, 1024).unwrap()).unwrap();
    assert_eq!(plan.levels(), 3);
    let bound = plan.bound();
    let bytes: u64 = bound.iter().map(|&words| words as u64 * 4).sum();
    println!("1024^3: {bytes} bytes over {} bindings", bound.len());
    for (limits, heap) in [(rtx_4080(), RTX_4080_HEAP), (laptop(), LAPTOP_HEAP)] {
        assert!(limits.admits(&bound, None).is_ok());
        assert!(limits.admits_on(&bound, None, heap).is_ok());
        let whole = limits.admits(&plan.sizes(), None);
        assert!(
            matches!(
                whole,
                Err(Error::TooLarge { binding, limit: Limit::StorageBufferRange, allowed, .. })
                    if binding == STATE as usize && allowed == RANGE
            ),
            "{whole:?}"
        );
    }
}

#[test]
fn the_4080_admits_2048_by_1024_by_1024_and_the_8_gibibyte_profile_names_its_memory() {
    let plan = plan_of(32, Grid::solid(2048, 1024, 1024).unwrap()).unwrap();
    let bound = plan.bound();
    let bytes: u64 = bound.iter().map(|&words| words as u64 * 4).sum();
    println!(
        "2048x1024x1024: {bytes} bytes over {} bindings",
        bound.len()
    );
    assert!(rtx_4080().admits_on(&bound, None, RTX_4080_HEAP).is_ok());
    let refused = laptop().admits_on(&bound, None, LAPTOP_HEAP);
    assert!(
        matches!(refused, Err(Error::TooMuchMemory { bytes: asked, heap }) if asked == bytes && heap == LAPTOP_HEAP),
        "{refused:?}"
    );
    assert!(laptop().admits(&bound, None).is_ok());
    let said = refused.unwrap_err().to_string();
    assert!(
        said.contains(&bytes.to_string()) && said.contains("memory"),
        "{said}"
    );
}

#[test]
fn a_world_whose_state_passes_a_u32_index_is_refused_by_name() {
    let grid = Grid::solid(4096, 2048, 1024).unwrap();
    let refused = plan_of(32, grid);
    assert!(
        matches!(refused, Err(TooBig::Cells { cells, limit }) if cells == 1 << 33 && limit == 1 << 32),
        "{refused:?}"
    );
    let fits = plan_of(32, Grid::solid(2048, 2048, 1024).unwrap()).unwrap();
    assert_eq!(fits.state_words(), 1 << 32);
    let most = 512 * 512 * 512;
    assert!(matches!(
        Plan::over_in(64, most + 1, Dimension::Three),
        Err(TooBig::Blocks { blocks, limit }) if blocks == most + 1 && limit == most
    ));
}

/// The bytes the valley's engine and its picture at `screen` take on `grid`
/// at a subgroup of 32, laid over bindings.
fn pictured(grid: Grid, screen: grit::render3::Screen) -> Vec<usize> {
    let plan = plan_of(32, grid).unwrap();
    let layout = grit::render3::Layout::new(grid, screen, 32);
    let sized = layout.sized(live::sizes3(&grit::rule3::nature::Nature, &plan));
    plan.windows().laid(&sized)
}

#[test]
fn the_picture_decides_what_memory_the_largest_worlds_need() {
    let full = grit::render3::Screen {
        wide: 1920,
        high: 1080,
    };
    let half = grit::render3::Screen {
        wide: 960,
        high: 540,
    };
    let bytes = |sizes: &[usize]| sizes.iter().map(|&words| words as u64 * 4).sum::<u64>();
    let cube = Grid::solid(1024, 1024, 1024).unwrap();
    let wide = Grid::solid(2048, 1024, 1024).unwrap();
    for (grid, screen) in [(cube, full), (cube, half), (wide, full)] {
        println!(
            "{}x{}x{} with a {}x{} picture: {} bytes",
            grid.wide,
            grid.deep,
            grid.tall,
            screen.wide,
            screen.high,
            bytes(&pictured(grid, screen))
        );
    }
    let cube_full = pictured(cube, full);
    assert!(
        rtx_4080()
            .admits_on(&cube_full, None, RTX_4080_HEAP)
            .is_ok()
    );
    let wide_full = pictured(wide, full);
    assert!(rtx_4080().admits(&wide_full, None).is_ok());
    let refused = rtx_4080().admits_on(&wide_full, None, RTX_4080_HEAP);
    assert!(
        matches!(refused, Err(Error::TooMuchMemory { bytes: asked, heap }) if asked == bytes(&wide_full) && heap == RTX_4080_HEAP),
        "{refused:?}"
    );
}
