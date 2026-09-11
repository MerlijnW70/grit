#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{LOOKING, PARAMS, PIXELS, STATE};
use grit::sand::{
    self, AIR, Grid, RUNNING_RIGHT, SAND, View, WALL, WATER, material_of, running_of,
};
use grit::sand::{BINDINGS, WEIGHTS};
use grit::{Groups, Host};

const WIDE: u32 = 512;
const DEEP: u32 = 256;

const STEPS: u32 = 100;

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

fn poured(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let at = (row * grid.wide + column) as usize;
            let floor = row + 1 >= grid.deep;
            let side = column == 0 || column + 1 >= grid.wide;
            let slab = (32..96).contains(&row) && (100..400).contains(&column);

            cells[at] = if floor || side {
                WALL
            } else if slab {
                SAND
            } else {
                AIR
            };
        }
    }
    cells
}

fn grains(cells: &[u32]) -> usize {
    cells.iter().filter(|&&cell| cell == SAND).count()
}

fn lowest(grid: Grid, cells: &[u32]) -> u32 {
    (0..grid.deep)
        .rev()
        .find(|&row| {
            (0..grid.wide)
                .any(|column| cells.get((row * grid.wide + column) as usize) == Some(&SAND))
        })
        .unwrap_or(0)
}

#[test]
fn the_sand_falls_where_the_host_says_and_no_grain_is_lost() {
    let Some(host) = device("sand") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    println!("{} {width} {} {}", host.name(), grid.wide, grid.deep);

    let tick = grit::sand::settle(width, grid).unwrap();
    let bump = frame_counter(width).unwrap();

    let modules: Vec<&[u32]> = vec![&bump, &tick];
    let cells = grid.cells();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = 8;
    sizes[1] = cells;

    let mut resident = host.resident(&sizes, &modules).unwrap();

    let start = poured(grid);
    let before = grains(&start);
    resident.write(0, &[u32::MAX]).unwrap();
    resident.write(1, &start).unwrap();

    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];

    let mut want = start.clone();
    let mut moved = 0_u32;

    for step in 0..STEPS {
        resident.frame(&groups).unwrap();
        want = sand::settle_reference(grid, step, &want);

        let ran = resident.read(1, cells).unwrap();
        if let Some(at) = ran
            .iter()
            .zip(&want)
            .position(|(there, here)| there != here)
        {
            let (row, column) = (at as u32 / WIDE, at as u32 % WIDE);
            panic!("{step} {column} {row} {} {}", ran[at], want[at]);
        }

        assert_eq!(grains(&ran), before);
        moved = lowest(grid, &ran);
    }

    let settled = lowest(grid, &want);
    println!("{before} {STEPS} {moved} {}", grid.deep);
    assert!(settled > 96);
    assert!(settled + 1 < DEEP);
}

fn colour(material: u32) -> u32 {
    match material {
        WATER => 0x0024_6FA8,
        SAND => 0x00D9_B36A,
        WALL => 0x0044_4A52,
        _ => 0x0011_1622,
    }
}

fn column(grid: Grid, material: u32) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    let left = grid.wide / 2 - COLUMN / 2;
    for row in 0..grid.deep {
        for at in 0..grid.wide {
            let edge = row + 1 >= grid.deep || at == 0 || at + 1 >= grid.wide;
            let stack = (8..8 + TALL).contains(&row) && (left..left + COLUMN).contains(&at);
            cells[(row * grid.wide + at) as usize] = if edge {
                WALL
            } else if stack {
                material
            } else {
                AIR
            };
        }
    }
    cells
}

fn reach(grid: Grid, cells: &[u32], material: u32) -> u32 {
    (0..grid.wide)
        .filter(|&at| {
            (0..grid.deep).any(|row| {
                cells
                    .get((row * grid.wide + at) as usize)
                    .is_some_and(|&word| material_of(word) == material)
            })
        })
        .count()
        .try_into()
        .unwrap_or(0)
}

fn run(host: &Host, grid: Grid, start: &[u32], steps: u32, label: u32) -> (Vec<u32>, Vec<u32>) {
    let width = host.width();
    let tick = sand::settle(width, grid).unwrap();
    let bump = frame_counter(width).unwrap();
    let modules: Vec<&[u32]> = vec![&bump, &tick];

    let cells = grid.cells();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 8;
    sizes[STATE as usize] = cells;
    let mut resident = host.resident(&sizes, &modules).unwrap();
    resident.write(PARAMS as usize, &[u32::MAX; 8]).unwrap();
    resident.write(STATE as usize, start).unwrap();

    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];

    let mut want = start.to_vec();
    let mut before = start.to_vec();
    let mut ran = start.to_vec();
    for step in 0..steps {
        before = ran;
        resident.frame(&groups).unwrap();
        want = sand::settle_reference(grid, step, &want);
        ran = resident.read(STATE as usize, cells).unwrap();

        if let Some(at) = ran
            .iter()
            .zip(&want)
            .position(|(here, there)| here != there)
        {
            let (row, at_column) = (at as u32 / grid.wide, at as u32 % grid.wide);
            panic!("{label} {step} {at_column} {row} {} {}", ran[at], want[at]);
        }
    }
    (before, ran)
}

const COLUMN: u32 = 8;
const TALL: u32 = 16;

const BASIN: u32 = 64;
const HIGH: u32 = 32;

const SETTLING: u32 = 400;

fn depths(grid: Grid, cells: &[u32], material: u32) -> Vec<u32> {
    (0..grid.wide)
        .map(|at| {
            (0..grid.deep)
                .filter(|&row| {
                    cells
                        .get((row * grid.wide + at) as usize)
                        .is_some_and(|&word| material_of(word) == material)
                })
                .count()
                .try_into()
                .unwrap_or(0)
        })
        .collect()
}

#[test]
fn water_goes_flat_where_sand_heaps_up() {
    let Some(host) = device("water") else {
        return;
    };
    let grid = Grid::of(BASIN, HIGH);
    println!(
        "{} {} {} {}",
        host.name(),
        host.width(),
        grid.wide,
        grid.deep
    );

    let poured = COLUMN * TALL;
    let floor = grid.wide - 2;
    let (_, heaped) = run(&host, grid, &column(grid, SAND), SETTLING, SAND);
    let (_, settled) = run(&host, grid, &column(grid, WATER), SETTLING, WATER);

    let sand_wide = reach(grid, &heaped, SAND);
    let water_wide = reach(grid, &settled, WATER);
    let lying = depths(grid, &settled, WATER);
    let wet: Vec<u32> = lying.iter().copied().filter(|&deep| deep > 0).collect();
    let (shallowest, deepest) = (
        wet.iter().copied().min().unwrap_or(0),
        wet.iter().copied().max().unwrap_or(0),
    );
    let moving = settled
        .iter()
        .filter(|&&word| running_of(word) != 0)
        .count();

    println!("{poured} {COLUMN} {sand_wide} {water_wide} {floor}");
    println!("{shallowest} {deepest} {moving}");

    assert_eq!(
        heaped.iter().filter(|&&w| material_of(w) == SAND).count() as u32,
        poured
    );
    assert_eq!(
        settled.iter().filter(|&&w| material_of(w) == WATER).count() as u32,
        poured
    );

    assert!(water_wide > sand_wide * 2);
    assert_eq!(water_wide, floor);
    assert!(deepest - shallowest <= 1);

    assert_eq!(moving, 0);

    for phase in 0..2 {
        let again = sand::settle_reference(grid, phase, &settled);
        let differs = again
            .iter()
            .zip(&settled)
            .position(|(there, here)| there != here);
        assert!(differs.is_none());
    }
}

#[test]
fn the_window_shows_the_cells_it_is_pointed_at() {
    let Some(host) = device("view") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(64, 32);
    let view = View::of(16, 8);
    let module = sand::paint(width, grid, view).unwrap();
    let modules: Vec<&[u32]> = vec![&module];

    let cells = grid.cells();
    let pixels = view.pixels();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 8;
    sizes[STATE as usize] = cells;
    sizes[PIXELS as usize] = pixels;
    let mut resident = host.resident(&sizes, &modules).unwrap();

    let world: Vec<u32> = (0..cells)
        .map(|at| match (at * 7 + at / 13) % 6 {
            0 | 3 => SAND,
            1 => WALL,
            2 => WATER,
            4 => RUNNING_RIGHT,
            _ => AIR,
        })
        .collect();
    resident.write(PARAMS as usize, &[0; 8]).unwrap();
    resident.write(STATE as usize, &world).unwrap();

    let groups = [Groups::Fixed(
        u32::try_from(pixels).unwrap_or(u32::MAX).div_ceil(width),
    )];

    for corner in [(0, 0), (17, 9), (grid.wide - 3, 6), (30, grid.deep - 5)] {
        resident
            .write_at(PARAMS as usize, LOOKING, &[corner.0, corner.1])
            .unwrap();
        let (_, picture) = resident
            .frame_reading(&groups, PIXELS as usize, pixels)
            .unwrap();

        for down in 0..view.deep {
            for across in 0..view.wide {
                let at = (corner.0 + across) % grid.wide;
                let row = (corner.1 + down) % grid.deep;
                let want = colour(material_of(world[(row * grid.wide + at) as usize]));
                let seat = (down * view.wide + across) as usize;
                assert_eq!(picture[seat], want);
            }
        }
    }

    println!(
        "{} {} {} {} {pixels}",
        grid.wide, grid.deep, view.wide, view.deep
    );

    assert!(pixels < cells);
}

fn frame_counter(subgroup: u32) -> Result<Vec<u32>, simdr::lanes::LaneError> {
    use simdr::kernel::{Kernel, Shape};
    use simdr::lanes::U32;

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();

    let now = kernel.load_at(0, slot)?;
    let next = kernel.module().i_add(element, now, one)?;
    kernel.store_at(0, slot, next)?;
    kernel.finish()
}

#[test]
fn the_materials_are_in_the_order_the_rule_needs() {
    assert_eq!(WEIGHTS, [AIR, WATER, SAND, WALL]);
    for pair in WEIGHTS.windows(2) {
        assert!(pair[0] < pair[1]);
    }
    assert_eq!(*WEIGHTS.last().unwrap(), WALL);
}
