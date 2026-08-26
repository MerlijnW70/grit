//! Falling sand, held to the host and to physics.
//!
//! # Three things a wrong sand simulation still looks like
//!
//! It looks like sand. Grains move down, piles form, and the picture is entirely convincing while
//! the rule is wrong — so the checks here are the ones a convincing picture cannot satisfy:
//!
//! * **Conservation.** Not one grain appears or disappears, over a hundred steps. A Margolus
//!   partition gives that by construction, so a count that drifts means the partition is not what
//!   the kernel thinks it is — two invocations writing the same cell, which is the one failure this
//!   whole arrangement exists to make impossible.
//! * **The host, step for step.** Every cell, every frame, bit for bit. The rule is four ordered
//!   swaps and a reference that applied them in another order would agree about a heap and disagree
//!   about a grain.
//! * **That it actually falls.** A rule that did nothing would conserve perfectly and match a
//!   reference that also did nothing.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::{LOOKING, PARAMS, PIXELS, STATE};
use grit::sand::BINDINGS;
use grit::sand::{
    self, AIR, Grid, RUNNING_RIGHT, SAND, View, WALL, WATER, material_of, running_of,
};
use grit::{Groups, Host};

/// The world under test.
const WIDE: u32 = 512;
const DEEP: u32 = 256;

/// How many steps the check runs.
const STEPS: u32 = 100;

/// Open a device, or say why not and hand back nothing.
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

/// A grid worth stepping: a floor, a wall down one side, and a slab of sand up top.
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

/// How many grains there are.
fn grains(cells: &[u32]) -> usize {
    cells.iter().filter(|&&cell| cell == SAND).count()
}

/// How far down the lowest grain has reached.
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
    println!(
        "{} — subgroup {width}, {}×{} cells",
        host.name(),
        grid.wide,
        grid.deep
    );

    let tick = grit::sand::settle(width, grid).expect("settle");
    let bump = frame_counter(width).expect("counter");

    // The frame counter first, so the phase the settle reads is this step's rather than the last.
    let modules: Vec<&[u32]> = vec![&bump, &tick];
    let cells = grid.cells();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = 8;
    sizes[1] = cells;

    let mut resident = host.resident(&sizes, &modules).expect("allocated");

    let start = poured(grid);
    let before = grains(&start);
    resident.write(0, &[u32::MAX]).expect("params");
    resident.write(1, &start).expect("state");

    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];

    let mut want = start.clone();
    let mut moved = 0_u32;

    for step in 0..STEPS {
        resident.frame(&groups).expect("dispatched");
        want = sand::settle_reference(grid, step, &want);

        let ran = resident.read(1, cells).expect("state");
        if let Some(at) = ran
            .iter()
            .zip(&want)
            .position(|(there, here)| there != here)
        {
            let (row, column) = (at as u32 / WIDE, at as u32 % WIDE);
            panic!(
                "step {step}: cell ({column}, {row}) holds {} and the host says {}",
                ran[at], want[at]
            );
        }

        assert_eq!(
            grains(&ran),
            before,
            "step {step}: {} grains, and {before} were poured — a cell was written twice",
            grains(&ran)
        );
        moved = lowest(grid, &ran);
    }

    let settled = lowest(grid, &want);
    println!(
        "  {before} grains, {STEPS} steps, the lowest reached row {moved} of {}",
        grid.deep
    );
    assert!(
        settled > 96,
        "the sand never left the slab it was poured in, so the rule does nothing"
    );
    assert!(
        settled + 1 < DEEP,
        "sand reached the floor row, which is wall and cannot hold it"
    );
}

/// What each material comes out as, in the order `paint` asks its three questions.
fn colour(material: u32) -> u32 {
    match material {
        WATER => 0x0024_6FA8,
        SAND => 0x00D9_B36A,
        WALL => 0x0044_4A52,
        _ => 0x0011_1622,
    }
}

/// A floor, two sides, and a column of `material` standing above the middle of it.
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

/// How many columns hold at least one cell of `material`.
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

/// Step a world on the device for `steps` frames, checking the host every one, and hand back the
/// last two states so the caller can ask whether anything is still moving.
fn run(host: &Host, grid: Grid, start: &[u32], steps: u32, label: &str) -> (Vec<u32>, Vec<u32>) {
    let width = host.width();
    let tick = sand::settle(width, grid).expect("settle");
    let bump = frame_counter(width).expect("counter");
    let modules: Vec<&[u32]> = vec![&bump, &tick];

    let cells = grid.cells();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 8;
    sizes[STATE as usize] = cells;
    let mut resident = host.resident(&sizes, &modules).expect("allocated");
    resident
        .write(PARAMS as usize, &[u32::MAX; 8])
        .expect("params");
    resident.write(STATE as usize, start).expect("state");

    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];

    let mut want = start.to_vec();
    let mut before = start.to_vec();
    let mut ran = start.to_vec();
    for step in 0..steps {
        before = ran;
        resident.frame(&groups).expect("dispatched");
        want = sand::settle_reference(grid, step, &want);
        ran = resident.read(STATE as usize, cells).expect("state");

        if let Some(at) = ran
            .iter()
            .zip(&want)
            .position(|(here, there)| here != there)
        {
            let (row, at_column) = (at as u32 / grid.wide, at as u32 % grid.wide);
            panic!(
                "{label} step {step}: cell ({at_column}, {row}) holds {} and the host says {}",
                ran[at], want[at]
            );
        }
    }
    (before, ran)
}

/// How wide the column of material starts, and how tall.
const COLUMN: u32 = 8;
const TALL: u32 = 16;

/// The basin both materials are dropped into.
const BASIN: u32 = 64;
const HIGH: u32 = 32;

/// How long each is given to find its shape.
const SETTLING: u32 = 400;

/// How deep the water lies in each column.
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
        "{} — subgroup {}, {}×{} cells",
        host.name(),
        host.width(),
        grid.wide,
        grid.deep
    );

    let poured = COLUMN * TALL;
    let floor = grid.wide - 2;
    let (_, heaped) = run(&host, grid, &column(grid, SAND), SETTLING, "sand");
    let (_, settled) = run(&host, grid, &column(grid, WATER), SETTLING, "water");

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

    println!(
        "  {poured} cells from a {COLUMN}-wide column: sand reached {sand_wide} columns, water {water_wide} of {floor}"
    );
    println!("  water lies {shallowest} to {deepest} deep, and {moving} cells are still moving");

    // Conserved, both of them. A rule that leaked would spread beautifully and mean nothing.
    assert_eq!(
        heaped.iter().filter(|&&w| material_of(w) == SAND).count() as u32,
        poured,
        "sand was created or lost"
    );
    assert_eq!(
        settled.iter().filter(|&&w| material_of(w) == WATER).count() as u32,
        poured,
        "water was created or lost"
    );

    // **The claim of the pass.** Sand has an angle of repose and stops at a heap a little wider than
    // the column it came from; water has none and reaches the walls. Same rule, same block, same
    // four falls — what differs is one number and one extra read.
    assert!(
        water_wide > sand_wide * 2,
        "water reached {water_wide} columns and sand {sand_wide} — the fluid is behaving like a solid"
    );
    assert_eq!(
        water_wide, floor,
        "water stopped short of the walls, so it did not find its level"
    );
    assert!(
        deepest - shallowest <= 1,
        "the water lies {shallowest} deep in one place and {deepest} in another, which is not level"
    );

    // **And that it stops, which it used not to.** A hundred and twenty-eight cells do not divide
    // into sixty-two columns, so four of them have no level place to be — and they used to pace back
    // and forth for ever, because a rule reading one 2×2 block cannot tell them from water that
    // ought to flow. What it can read is its own past, and `sand::TIRED` is three bits of it: at
    // `sand::PATIENCE` a cell stops asking for a direction. This bound was `moving * 16 < poured`
    // and is now nought.
    assert_eq!(moving, 0, "{moving} of {poured} cells are still running");

    // The same claim without the loophole. No direction is a snapshot: a cell may be about to be
    // given one on the very next frame, and counting words would not see it. So step the settled
    // grid again on the host — **in both phases**, because the partition's origin alternates and a
    // cell with nowhere to go this frame may have somewhere the next — and demand it come back
    // unchanged. Nothing that cannot move is the whole of what "the work list reaches nought" means.
    for phase in 0..2 {
        let again = sand::settle_reference(grid, phase, &settled);
        let differs = again
            .iter()
            .zip(&settled)
            .position(|(there, here)| there != here);
        assert!(
            differs.is_none(),
            "a further step in phase {phase} moved cell {:?}, so the water has not come to rest",
            differs.unwrap_or(0)
        );
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
    let module = sand::paint(width, grid, view).expect("paint");
    let modules: Vec<&[u32]> = vec![&module];

    let cells = grid.cells();
    let pixels = view.pixels();
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 8;
    sizes[STATE as usize] = cells;
    sizes[PIXELS as usize] = pixels;
    let mut resident = host.resident(&sizes, &modules).expect("allocated");

    // Something with no symmetry, so a window at the wrong corner cannot look right by accident —
    // and running water among it, because a runner has to come out the same blue a still cell does.
    let world: Vec<u32> = (0..cells)
        .map(|at| match (at * 7 + at / 13) % 6 {
            0 | 3 => SAND,
            1 => WALL,
            2 => WATER,
            4 => RUNNING_RIGHT,
            _ => AIR,
        })
        .collect();
    resident.write(PARAMS as usize, &[0; 8]).expect("params");
    resident.write(STATE as usize, &world).expect("state");

    let groups = [Groups::Fixed(
        u32::try_from(pixels).unwrap_or(u32::MAX).div_ceil(width),
    )];

    // The last two wrap: a window is allowed to hang off the right edge and off the bottom, because
    // the grid is a torus and every other address in this crate already relies on it.
    for corner in [(0, 0), (17, 9), (grid.wide - 3, 6), (30, grid.deep - 5)] {
        resident
            .write_at(PARAMS as usize, LOOKING, &[corner.0, corner.1])
            .expect("looking");
        let (_, picture) = resident
            .frame_reading(&groups, PIXELS as usize, pixels)
            .expect("painted");

        for down in 0..view.deep {
            for across in 0..view.wide {
                let at = (corner.0 + across) % grid.wide;
                let row = (corner.1 + down) % grid.deep;
                let want = colour(material_of(world[(row * grid.wide + at) as usize]));
                let seat = (down * view.wide + across) as usize;
                assert_eq!(
                    picture[seat], want,
                    "at corner {corner:?}, pixel ({across}, {down}) should be cell ({at}, {row})"
                );
            }
        }
    }

    println!(
        "  four corners of a {}×{} world through a {}×{} window, {pixels} pixels each",
        grid.wide, grid.deep, view.wide, view.deep
    );

    // **The window is smaller than the world, which is the whole point of the pass.** A viewport
    // that painted everything would pass every check above and still cost the readback it exists
    // to avoid.
    assert!(
        pixels < cells,
        "the window holds the whole grid, so nothing was saved by looking through it"
    );
}

/// One word, incremented on the device — the same trick `src/bin/spike.rs` uses.
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
