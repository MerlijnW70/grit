#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{
    AWAKE, BRUSH, DENSE, GRAND, HOT, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_INDIRECT, INDIRECT,
    PARAMS, STATE, TALLY, TALLY_WORDS,
};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Resident};

/// What a fresh allocation may hold: Vulkan does not zero memory.
const JUNK: u32 = 0xA5A5_A5A5;

/// How the listed engine's buffers are left before `seed`.
#[derive(Clone, Copy, Debug)]
enum Before {
    /// Every binding holds this word.
    Junk(u32),
    /// Every binding is zero and the frame words are set, except the coarse
    /// flags past the plan's blocks, which hold this word.
    Padded(u32),
}

type Brushing = fn(u32, Grid) -> [u32; 4];

#[derive(Clone, Copy)]
struct Setup {
    label: &'static str,
    grid: Grid,
    coarse: bool,
    frames: u32,
    before: Before,
    brushing: Brushing,
    /// Frames run on another world before the world under test is written
    /// and seeded again; zero seeds once.
    again: u32,
}

/// A walled room with sand and water scattered through it.
fn scattered(grid: Grid, mut seed: u64) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for (at, cell) in cells.iter_mut().enumerate() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let row = at as u32 / grid.wide;
        let column = at as u32 % grid.wide;
        let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
        *cell = if edge {
            WALL
        } else {
            match seed % 100 {
                0..3 => SAND,
                3..5 => WATER,
                _ => AIR,
            }
        };
    }
    cells
}

/// The brush of `tests/listed.rs` that moves three cells a frame, sand and
/// water in turns, and then stops.
fn moving(frame: u32, grid: Grid) -> [u32; 4] {
    let span = grid.wide - 80;
    let x = 40 + (3 * frame) % span;
    let material = match frame {
        0..150 if (frame / 30).is_multiple_of(2) => SAND,
        0..150 => WATER,
        _ => AIR,
    };
    [x, 30, 25, material]
}

/// A small brush in the middle of a grid too small for `moving`, on for
/// eight frames and off for eight.
fn pulse(frame: u32, grid: Grid) -> [u32; 4] {
    let material = if (frame / 8).is_multiple_of(2) {
        SAND
    } else {
        AIR
    };
    [grid.wide / 2, grid.deep / 3, 5, material]
}

fn plan_for(width: u32, grid: Grid) -> Plan {
    Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap()
}

fn cleared(held: &mut Resident<'_>, sizes: &[usize], word: u32) {
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![word; words]).unwrap();
    }
}

fn left(held: &mut Resident<'_>, sizes: &[usize], plan: &Plan, before: Before) {
    match before {
        Before::Junk(word) => cleared(held, sizes, word),
        Before::Padded(word) => {
            cleared(held, sizes, 0);
            let pad = (plan.padded() - plan.blocks) as usize;
            assert!(pad > 0, "the plan has no padding to fill");
            held.write_at(HOT as usize, plan.blocks as usize, &vec![word; pad])
                .unwrap();
            held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
                .unwrap();
        }
    }
}

fn differ(label: &str, frame: u32, grid: Grid, listed: &[u32], whole: &[u32]) {
    if let Some(at) = listed.iter().zip(whole).position(|(a, b)| a != b) {
        let (row, column) = (at as u32 / grid.wide, at as u32 % grid.wide);
        let count = listed.iter().zip(whole).filter(|(a, b)| a != b).count();
        panic!(
            "{label} frame {frame}: cell ({column},{row}) listed {:#x} whole {:#x}, {count} cells differ",
            listed[at], whole[at]
        );
    }
}

/// The list is ascending, so it holds each quad at most once, and it holds
/// only quads of the grid.
fn held_once(label: &str, frame: u32, grid: Grid, list: &[u32]) {
    if let Some(at) = list.windows(2).position(|pair| pair[0] >= pair[1]) {
        panic!(
            "{label} frame {frame}: list slot {at} holds {} then {}",
            list[at],
            list[at + 1]
        );
    }
    if let Some(&past) = list.iter().find(|&&quad| quad >= grid.quads()) {
        panic!(
            "{label} frame {frame}: quad {past} listed on a grid of {} quads",
            grid.quads()
        );
    }
}

/// Fills the listed engine's buffers as `setup.before` says, writes the
/// world, seeds, and compares the listed engine with the whole-grid step
/// after every frame. The whole grid starts from zeroed buffers and the
/// frame words every caller wrote by hand before `seed` set them.
fn against_the_whole_grid(setup: Setup) {
    let Some(host) = device(setup.label) else {
        return;
    };
    let width = host.width();
    let grid = setup.grid;
    let plan = plan_for(width, grid);
    let cells = grid.cells();

    let (wired, wired_stages) =
        grit::live::arranged(width, grid, &plan, None, setup.coarse).unwrap();
    let wired_groups: Vec<&[Groups]> = wired_stages.iter().map(Vec::as_slice).collect();

    let whole = [
        grit::live::tick(width).unwrap(),
        sand::brush(width, grid, false).unwrap(),
        sand::settle(width, grid).unwrap(),
    ];
    let whole_groups = [
        Groups::Fixed(1),
        Groups::Fixed(sand::BOX * sand::BOX / width),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];
    let whole_stages: Vec<&[Groups]> = whole_groups.iter().map(core::slice::from_ref).collect();

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = wired.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).unwrap();

    let start = scattered(grid, 0x2545_F491_4F6C_DD1D);
    cleared(&mut full, &sizes, 0);
    full.write(STATE as usize, &start).unwrap();
    full.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();

    left(&mut live, &sizes, &plan, setup.before);
    if setup.again > 0 {
        let earlier = scattered(grid, 0x9E37_79B9_7F4A_7C15);
        live.write(STATE as usize, &earlier).unwrap();
        grit::live::seed(&mut live, grid, &plan).unwrap();
        for frame in 0..setup.again {
            let brush = (setup.brushing)(frame + 7, grid);
            live.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
            live.staged(&wired_groups).unwrap();
        }
    }
    live.write(STATE as usize, &start).unwrap();
    grit::live::seed(&mut live, grid, &plan).unwrap();

    let mut listed_total = 0_u64;
    for frame in 0..setup.frames {
        let brush = (setup.brushing)(frame, grid);
        for held in [&mut live, &mut full] {
            held.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
        }
        let (_, there, listed) = live
            .staged_reading_both(&wired_groups, (STATE as usize, cells), (GRAND as usize, 1))
            .unwrap();
        let (_, here) = full
            .staged_reading(&whole_stages, STATE as usize, cells)
            .unwrap();
        differ(setup.label, frame, grid, &there, &here);
        let count = listed.first().copied().unwrap_or(0);
        assert!(
            count <= grid.quads(),
            "{} frame {frame}: {count} quads listed on a grid of {}",
            setup.label,
            grid.quads()
        );
        held_once(
            setup.label,
            frame,
            grid,
            &live.read(DENSE as usize, count as usize).unwrap(),
        );
        listed_total += u64::from(count);
    }

    println!(
        "{} {} sg={width} grid={}x{} blocks={} padded={} coarse={} before={:?} again={} frames={}: AGREED, mean listed {} of {}",
        setup.label,
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks,
        plan.padded(),
        setup.coarse,
        setup.before,
        setup.again,
        setup.frames,
        listed_total / u64::from(setup.frames.max(1)),
        grid.quads()
    );
}

const ROOM: Grid = Grid::of(512, 256);

fn room(label: &'static str, before: Before, coarse: bool) -> Setup {
    Setup {
        label,
        grid: ROOM,
        coarse,
        frames: 200,
        before,
        brushing: moving,
        again: 0,
    }
}

#[test]
fn junk_in_every_binding_before_seed_gives_the_whole_grid_world() {
    against_the_whole_grid(room("junk fine", Before::Junk(JUNK), false));
}

#[test]
fn junk_in_every_binding_before_seed_agrees_on_the_coarse_tier_too() {
    against_the_whole_grid(room("junk coarse", Before::Junk(JUNK), true));
}

#[test]
fn flags_in_the_coarse_padding_before_seed_list_no_phantom_block() {
    against_the_whole_grid(room("padding coarse", Before::Padded(2), true));
}

#[test]
fn flags_in_the_coarse_padding_leave_the_fine_tier_alone() {
    against_the_whole_grid(room("padding fine", Before::Padded(2), false));
}

#[test]
fn the_smallest_flag_in_the_padding_lists_no_phantom_block() {
    against_the_whole_grid(room("padding one", Before::Padded(1), true));
}

#[test]
fn the_largest_flag_in_the_padding_lists_no_phantom_block() {
    against_the_whole_grid(room("padding max", Before::Padded(u32::MAX), true));
}

#[test]
fn seeding_a_used_engine_again_gives_the_whole_grid_world() {
    for coarse in [false, true] {
        against_the_whole_grid(Setup {
            again: 51,
            ..room("again", Before::Padded(2), coarse)
        });
    }
}

/// A square grid of exactly `per_block` blocks, so that the coarse flags
/// fill whole tiles and only the cold slot lies past the blocks.
#[test]
fn junk_agrees_when_the_blocks_fill_their_tiles_exactly() {
    let Some(host) = device("unpadded") else {
        return;
    };
    let width = host.width();
    drop(host);
    let per_block = sweep::cells(width);
    let side = 2 * per_block;
    let grid = Grid::of(side, side);
    let plan = plan_for(width, grid);
    assert_eq!(plan.blocks, plan.padded());
    for coarse in [false, true] {
        against_the_whole_grid(Setup {
            label: "unpadded",
            grid,
            coarse,
            frames: 60,
            before: Before::Junk(JUNK),
            brushing: moving,
            again: 0,
        });
    }
}

/// A grid of exactly one block: every coarse flag but the first is padding.
#[test]
fn junk_agrees_on_a_grid_of_one_block() {
    let Some(host) = device("one block") else {
        return;
    };
    let per_block = sweep::cells(host.width());
    drop(host);
    let grid = Grid::of(64, per_block * 4 / 64);
    assert_eq!(grid.quads(), per_block);
    for coarse in [false, true] {
        against_the_whole_grid(Setup {
            label: "one block",
            grid,
            coarse,
            frames: 200,
            before: Before::Junk(JUNK),
            brushing: pulse,
            again: 0,
        });
    }
}

/// A grid smaller than a block: the plan's one block holds quads the grid
/// does not have, and their wake marks are read by every fine mark.
#[test]
fn junk_agrees_on_a_grid_smaller_than_a_block() {
    for coarse in [false, true] {
        against_the_whole_grid(Setup {
            label: "smaller than a block",
            grid: Grid::of(16, 16),
            coarse,
            frames: 200,
            before: Before::Junk(JUNK),
            brushing: pulse,
            again: 0,
        });
    }
}

fn read(held: &mut Resident<'_>, binding: u32, words: usize) -> Vec<u32> {
    held.read(binding as usize, words).unwrap()
}

/// Reads back every word `seed` promises, after junk in every binding.
fn seed_sets_up(label: &str, grid: Grid) {
    let Some(host) = device(label) else {
        return;
    };
    let width = host.width();
    let plan = plan_for(width, grid);
    let (wired, _) = grit::live::arranged(width, grid, &plan, None, true).unwrap();
    let modules: Vec<&[u32]> = wired.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    cleared(&mut held, &sizes, JUNK);
    let world = scattered(grid, 7);
    held.write(STATE as usize, &world).unwrap();
    grit::live::seed(&mut held, grid, &plan).unwrap();

    let quads = grid.quads() as usize;
    let blocks = plan.blocks as usize;
    let dump = plan.dump() as usize;

    let params = read(&mut held, PARAMS, sizes[PARAMS as usize]);
    assert_eq!(params[..2], [u32::MAX, plan.dump()], "{label} params");
    assert!(
        params[2..].iter().all(|&word| word == JUNK),
        "{label} brush"
    );

    let hot = read(&mut held, HOT, sizes[HOT as usize]);
    assert!(hot[..blocks].iter().all(|&flag| flag == 2), "{label} hot");
    assert!(
        hot[blocks..].iter().all(|&flag| flag == 0),
        "{label} hot pad"
    );

    let awake = read(&mut held, AWAKE, sizes[AWAKE as usize]);
    assert!(
        awake[..quads].iter().all(|&mark| mark == 2),
        "{label} awake"
    );
    assert!(
        awake[quads..].iter().all(|&mark| mark == 0),
        "{label} scratch"
    );

    let dense = read(&mut held, DENSE, dump + 1);
    assert!(
        dense[..quads].iter().zip(0_u32..).all(|(&a, b)| a == b),
        "{label} dense"
    );
    assert_eq!(dense[dump], 0, "{label} claimed");

    let tally = read(&mut held, TALLY, TALLY_WORDS);
    assert_eq!(tally, vec![0; TALLY_WORDS], "{label} tally");

    let counts = read(&mut held, HOT_COUNTS, sizes[HOT_COUNTS as usize]);
    assert!(counts.iter().all(|&count| count == 0), "{label} hot counts");

    let listed = read(&mut held, HOT_DENSE, blocks);
    assert!(
        listed.iter().zip(0_u32..).all(|(&a, b)| a == b),
        "{label} hot dense"
    );
    assert_eq!(read(&mut held, GRAND, 1), [grid.quads()], "{label} grand");
    assert_eq!(
        read(&mut held, HOT_GRAND, 1),
        [plan.blocks],
        "{label} hot grand"
    );
    assert_eq!(
        read(&mut held, INDIRECT, 3),
        [grid.quads().div_ceil(width), 1, 1],
        "{label} indirect"
    );
    assert_eq!(
        read(&mut held, HOT_INDIRECT, 3),
        [plan.blocks, 1, 1],
        "{label} hot indirect"
    );
    assert_eq!(read(&mut held, STATE, world.len()), world, "{label} world");
    println!(
        "{label} {} sg={width} grid={}x{} blocks={blocks} padded={}: seed set every word",
        host.name(),
        grid.wide,
        grid.deep,
        plan.padded()
    );
}

#[test]
fn seed_writes_every_word_the_first_frame_reads() {
    seed_sets_up("seed room", ROOM);
}

#[test]
fn seed_writes_every_word_on_a_grid_smaller_than_a_block() {
    seed_sets_up("seed small", Grid::of(16, 16));
}
