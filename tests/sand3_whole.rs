#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeSet;

use common::device;

use grit::layout::{BINDINGS, PARAMS, STATE};
use grit::live;
use grit::sand::{AIR, Grid, SAND, WALL, WATER};
use grit::sand3::{
    self, CORNERS, FLAT, Grid3, NO_GROUPS, Spread, UNEVEN, running_of, settle_oct,
    settle_reference3, x_first,
};
use grit::{Groups, Host};
use simdr::lanes::LaneError;

const WORLDS: [(u32, u32, u32); 4] = [(16, 16, 16), (32, 32, 32), (8, 16, 32), (64, 32, 16)];

const SEEDS: [u64; 4] = [1, 7, 0x5EED, 0xDEAD_BEEF];

const FRAMES: u32 = 64;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const MARK: u32 = 0xA5A5_A5A5;

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
    let bits = ((roll >> 8) % 128) as u32;
    kind + (bits << 2)
}

fn world(grid: Grid3, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells()).map(|_| word(&mut state)).collect()
}

fn grids(wide: u32, deep: u32, tall: u32) -> (Grid, Grid3) {
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let reference = Grid3::new(wide, deep, tall).unwrap();
    assert_eq!(
        (grid.wide, grid.deep, grid.tall),
        (reference.wide, reference.deep, reference.tall)
    );
    (grid, reference)
}

#[derive(Clone, Copy)]
enum Seam {
    X,
    Y,
    Z,
}

/// Random cells within two cells of one seam of `seam`'s axis, on both
/// sides, and air everywhere else.
fn on_seam(grid: Grid3, seam: Seam, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![AIR; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let (at, extent) = match seam {
                    Seam::X => (x, grid.wide),
                    Seam::Y => (y, grid.deep),
                    Seam::Z => (z, grid.tall),
                };
                let near = at < 2 || at + 2 >= extent;
                let roll = word(&mut state);
                if near {
                    cells[grid.index(x, y, z)] = roll;
                }
            }
        }
    }
    cells
}

struct Run<'host> {
    resident: grit::Resident<'host>,
    groups: [Groups; 2],
    cells: usize,
    padded: usize,
}

fn ready(host: &Host, grid: Grid, most: u32) -> Run<'_> {
    let width = host.width();
    let built = sand3::settle(width, grid, most).unwrap();
    assert!(built.spread.groups <= host.limits().groups);
    let tick = live::tick(width).unwrap();
    let modules: Vec<&[u32]> = vec![&tick, &built.words];
    let cells = grid.cells();
    let padded = cells + (grid.wide * grid.deep) as usize;
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 8;
    sizes[STATE as usize] = padded;
    Run {
        resident: host.resident(&sizes, &modules).unwrap(),
        groups: [Groups::Fixed(1), built.spread.dispatch()],
        cells,
        padded,
    }
}

/// Runs `frames` frames from `start` on the device and on the CPU, and
/// fails at the first frame and cell where they differ. Past the grid,
/// `STATE` holds a mark that no frame may touch.
fn agree(run: &mut Run<'_>, reference: Grid3, start: u32, cells: &[u32], label: &str) -> Vec<u32> {
    let mut held = cells.to_vec();
    held.resize(run.padded, MARK);
    run.resident
        .write(PARAMS as usize, &[start.wrapping_sub(1)])
        .unwrap();
    run.resident.write(STATE as usize, &held).unwrap();

    let mut want = cells.to_vec();
    for frame in (0..FRAMES).map(|step| start.wrapping_add(step)) {
        run.resident.frame(&run.groups).unwrap();
        want = settle_reference3(reference, frame, &want);
        let got = run.resident.read(STATE as usize, run.padded).unwrap();
        if let Some(at) = (0..run.cells).find(|&at| got[at] != want[at]) {
            let plane = (reference.wide * reference.deep) as usize;
            let (z, rest) = (at / plane, at % plane);
            let (y, x) = (
                rest / reference.wide as usize,
                rest % reference.wide as usize,
            );
            panic!(
                "{label}: frame {frame} (phase {}, x first {}) cell ({x}, {y}, {z}): device {:#x}, reference {:#x}",
                frame & 1,
                x_first(frame),
                got[at],
                want[at]
            );
        }
        if let Some(at) = (run.cells..run.padded).find(|&at| got[at] != MARK) {
            panic!(
                "{label}: frame {frame} wrote {:#x} at word {at}, past the grid's {} cells",
                got[at], run.cells
            );
        }
    }
    let params = run.resident.read(PARAMS as usize, 1).unwrap();
    assert_eq!(params[0], start.wrapping_add(FRAMES - 1));
    want
}

#[test]
fn the_frames_run_through_both_phases_and_both_axis_orders() {
    let seen: BTreeSet<(u32, bool)> = (0..FRAMES)
        .map(|frame| (frame & 1, x_first(frame)))
        .collect();
    assert_eq!(seen.len(), 4);
    let late: BTreeSet<(u32, bool)> = (0..FRAMES)
        .map(|step| (u32::MAX - 31).wrapping_add(step))
        .map(|frame| (frame & 1, x_first(frame)))
        .collect();
    assert_eq!(late.len(), 4);
}

fn mirrored(oct: [u32; CORNERS]) -> [u32; CORNERS] {
    let turned = |word: u32| {
        let way = running_of(word);
        if (way & 1) != ((way >> 1) & 1) {
            word ^ (4 << 2)
        } else {
            word
        }
    };
    let mut out = oct.map(turned);
    out.swap(1, 2);
    out.swap(5, 6);
    out
}

#[test]
fn the_z_first_step_is_the_mirror_of_the_x_first_step() {
    let mut state = 0x0C7_u64;
    for _ in 0..200_000 {
        let oct: [u32; CORNERS] = std::array::from_fn(|_| word(&mut state));
        assert_eq!(
            settle_oct(oct, false),
            mirrored(settle_oct(mirrored(oct), true)),
            "{oct:?}"
        );
        let wild: [u32; CORNERS] = std::array::from_fn(|_| next(&mut state) as u32);
        assert_eq!(
            settle_oct(wild, false),
            mirrored(settle_oct(mirrored(wild), true)),
            "{wild:?}"
        );
    }
}

#[test]
fn the_spread_covers_every_oct_once_and_never_passes_the_groups_allowed() {
    let sides = [2, 4, 8, 16, 32, 64];
    for wide in sides {
        for deep in sides {
            for tall in sides {
                let grid = Grid::solid(wide, deep, tall).unwrap();
                let octs = grid.blocks();
                for width in WIDTHS {
                    for most in [1, 2, 3, 5, 7, 64, 65_535, u32::MAX] {
                        let spread = Spread::of(grid, width, most).unwrap();
                        assert!(spread.groups >= 1 && spread.groups <= most);
                        assert!(spread.workgroup <= width);
                        assert!(spread.per.is_power_of_two());
                        assert_eq!(spread.stride() * spread.per, octs);
                        if spread.per > 1 {
                            assert!(spread.groups * 2 > most);
                        }
                        let mut seen = vec![0_u8; octs as usize];
                        for invocation in 0..spread.stride() {
                            for round in 0..spread.per {
                                seen[(invocation + round * spread.stride()) as usize] += 1;
                            }
                        }
                        assert!(seen.iter().all(|&count| count == 1));
                    }
                }
            }
        }
    }
}

fn named<T: std::fmt::Debug>(built: Result<T, LaneError>, why: &str) {
    match built {
        Err(LaneError::NoSuchForm { operation, because }) => {
            assert_eq!(operation, "sand3::settle");
            assert_eq!(because, why);
        }
        other => panic!("wanted {why:?}, got {other:?}"),
    }
}

#[test]
fn the_kernel_refuses_a_flat_grid_no_groups_and_uneven_sides_by_name() {
    named(sand3::settle(16, Grid::of(64, 64), u32::MAX), FLAT);
    named(Spread::of(Grid::of(64, 64), 16, u32::MAX), FLAT);
    let grid = Grid::solid(16, 16, 16).unwrap();
    named(sand3::settle(16, grid, 0), NO_GROUPS);
    let uneven = Grid {
        wide: 12,
        deep: 16,
        tall: 16,
    };
    named(sand3::settle(16, uneven, u32::MAX), UNEVEN);
    let huge = Grid {
        wide: 1 << 16,
        deep: 1 << 16,
        tall: 1 << 4,
    };
    named(sand3::settle(16, huge, u32::MAX), UNEVEN);
    assert!(matches!(
        sand3::settle(12, grid, u32::MAX),
        Err(LaneError::BadWidth { width: 12 })
    ));
    assert!(matches!(
        sand3::settle(128, grid, u32::MAX),
        Err(LaneError::NoMapping { lanes: 128, .. })
    ));
}

#[test]
fn the_kernel_builds_at_every_width_for_every_world_and_spread() {
    let mut shapes = WORLDS.to_vec();
    shapes.extend([(2, 2, 2), (4, 4, 4), (128, 128, 128), (256, 256, 256)]);
    for (wide, deep, tall) in shapes {
        let (grid, _) = grids(wide, deep, tall);
        for width in WIDTHS {
            for most in [1, 3, 65_535, u32::MAX] {
                let built = sand3::settle(width, grid, most).unwrap();
                assert_eq!(built.spread, Spread::of(grid, width, most).unwrap());
                assert!(!built.words.is_empty());
            }
        }
    }
}

#[test]
fn the_kernel_equals_the_reference_on_random_worlds() {
    let Some(host) = device("sand3 whole") else {
        return;
    };
    for (wide, deep, tall) in WORLDS {
        let (grid, reference) = grids(wide, deep, tall);
        let mut run = ready(&host, grid, host.limits().groups);
        for (index, seed) in (0_u32..).zip(SEEDS) {
            let start = world(reference, seed);
            let label = format!("{wide}x{deep}x{tall} seed {seed:#x}");
            let end = agree(&mut run, reference, index * 5, &start, &label);
            assert_ne!(end, start, "{label}: nothing moved");
        }
        println!(
            "{} width {} {wide}x{deep}x{tall}: {} frames x {} seeds equal",
            host.name(),
            host.width(),
            FRAMES,
            SEEDS.len()
        );
    }
}

#[test]
fn the_kernel_equals_the_reference_when_each_invocation_steps_several_octs() {
    let Some(host) = device("sand3 several") else {
        return;
    };
    for (wide, deep, tall) in WORLDS {
        let (grid, reference) = grids(wide, deep, tall);
        let full = Spread::of(grid, host.width(), u32::MAX).unwrap();
        for most in [full.groups / 4, 3, 1] {
            let mut run = ready(&host, grid, most);
            let spread = Spread::of(grid, host.width(), most).unwrap();
            assert!(spread.per > 1);
            let label = format!("{wide}x{deep}x{tall} most {most} per {}", spread.per);
            let start = world(reference, SEEDS[2]);
            agree(&mut run, reference, 0, &start, &label);
        }
    }
}

#[test]
fn the_kernel_equals_the_reference_where_the_frame_counter_wraps() {
    let Some(host) = device("sand3 wrap") else {
        return;
    };
    let (grid, reference) = grids(16, 16, 16);
    let mut run = ready(&host, grid, host.limits().groups);
    let start = world(reference, SEEDS[1]);
    agree(
        &mut run,
        reference,
        u32::MAX - 31,
        &start,
        "16x16x16 wrapping frame",
    );
}

#[test]
fn the_kernel_equals_the_reference_on_small_and_thin_worlds() {
    let Some(host) = device("sand3 small") else {
        return;
    };
    for (wide, deep, tall) in [
        (2, 2, 2),
        (4, 4, 4),
        (2, 8, 4),
        (8, 8, 2),
        (32, 2, 8),
        (2, 2, 64),
    ] {
        let (grid, reference) = grids(wide, deep, tall);
        let mut run = ready(&host, grid, host.limits().groups);
        for seed in SEEDS {
            let start = world(reference, seed);
            agree(
                &mut run,
                reference,
                0,
                &start,
                &format!("{wide}x{deep}x{tall}"),
            );
        }
    }
}

fn across(host: &Host, seam: Seam, name: &str) {
    for (wide, deep, tall) in WORLDS {
        let (grid, reference) = grids(wide, deep, tall);
        let mut run = ready(host, grid, host.limits().groups);
        for seed in SEEDS {
            let start = on_seam(reference, seam, seed);
            let label = format!("{name} seam {wide}x{deep}x{tall} seed {seed:#x}");
            let end = agree(&mut run, reference, 0, &start, &label);
            assert_ne!(end, start, "{label}: nothing moved");
        }
    }
}

#[test]
fn the_kernel_equals_the_reference_across_the_x_seam() {
    let Some(host) = device("sand3 x seam") else {
        return;
    };
    across(&host, Seam::X, "x");
}

#[test]
fn the_kernel_equals_the_reference_across_the_y_seam() {
    let Some(host) = device("sand3 y seam") else {
        return;
    };
    across(&host, Seam::Y, "y");
}

#[test]
fn the_kernel_equals_the_reference_across_the_z_seam() {
    let Some(host) = device("sand3 z seam") else {
        return;
    };
    across(&host, Seam::Z, "z");
}

#[test]
fn the_kernel_equals_the_reference_on_words_with_every_bit_at_random() {
    let Some(host) = device("sand3 wild") else {
        return;
    };
    for (wide, deep, tall) in [(16, 16, 16), (64, 32, 16)] {
        let (grid, reference) = grids(wide, deep, tall);
        let mut run = ready(&host, grid, host.limits().groups);
        for seed in SEEDS {
            let mut state = seed;
            let start: Vec<u32> = (0..reference.cells())
                .map(|_| next(&mut state) as u32)
                .collect();
            let label = format!("wild {wide}x{deep}x{tall} seed {seed:#x}");
            agree(&mut run, reference, 2, &start, &label);
        }
    }
}

/// The loads and stores of each binding in `words`, counted in the SPIR-V
/// as written: a load or store through an access chain into the variable
/// decorated with that binding.
fn accesses(words: &[u32]) -> std::collections::BTreeMap<u32, (usize, usize)> {
    use std::collections::{BTreeMap, HashMap};
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0, "a zero-length instruction at word {at}");
        instructions.push((words[at] & 0xFFFF, &words[at + 1..at + count]));
        at += count;
    }
    let mut bound = HashMap::new();
    for (code, operands) in &instructions {
        if *code == 71 && operands[1] == 33 {
            bound.insert(operands[0], operands[2]);
        }
    }
    let mut chains = HashMap::new();
    for (code, operands) in &instructions {
        if *code == 65
            && let Some(&binding) = bound.get(&operands[2])
        {
            chains.insert(operands[1], binding);
        }
    }
    let mut counts = BTreeMap::new();
    for (code, operands) in &instructions {
        let pointer = match code {
            61 => operands[2],
            62 => operands[0],
            _ => continue,
        };
        let Some(&binding) = chains.get(&pointer) else {
            continue;
        };
        let entry: &mut (usize, usize) = counts.entry(binding).or_default();
        if *code == 61 {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
    counts
}

#[test]
fn an_oct_is_eight_loads_and_eight_stores_of_state_and_the_frame_one_load() {
    for (wide, deep, tall) in WORLDS {
        let (grid, _) = grids(wide, deep, tall);
        for width in WIDTHS {
            for most in [u32::MAX, 1] {
                let built = sand3::settle(width, grid, most).unwrap();
                let counts = accesses(&built.words);
                let wanted: std::collections::BTreeMap<u32, (usize, usize)> =
                    [(PARAMS, (1, 0)), (STATE, (8, 8))].into();
                assert_eq!(
                    counts, wanted,
                    "{wide}x{deep}x{tall} width {width} most {most} per {}",
                    built.spread.per
                );
            }
        }
    }
    let flat = accesses(&grit::sand::settle(16, Grid::of(64, 64)).unwrap());
    assert_eq!(flat.get(&STATE), Some(&(4, 4)));
}
