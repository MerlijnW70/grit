#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BINDINGS, BRUSH, DENSE, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Host, Resident};

const WIDE: u32 = 512;
const DEEP: u32 = 256;

const FRAMES: u32 = 6000;

const POURING: u32 = 90;

const SETTLING: u32 = 400;

const WETTING: u32 = 180;

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

fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            let shelf = (170..178).contains(&row) && (140..240).contains(&column);
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge || shelf { WALL } else { AIR };
            }
        }
    }
    cells
}

fn brushing(frame: u32) -> [u32; 4] {
    let material = match frame {
        0..POURING => SAND,
        POURING..WETTING => WATER,
        _ => AIR,
    };
    [190, 30, 36, material]
}

fn against_the_whole_grid(coarse: bool, grid: Grid, frames: u32, until_quiet: bool) {
    let Some(host) = device("live") else {
        return;
    };
    let width = host.width();
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            eprintln!("SKIPPED live at width {width}: {too_big}");
            return;
        }
    };
    let cells = grid.cells();
    println!(
        "{} — subgroup {width}, {}×{} cells in {quads} quads, {} blocks, coarse {coarse}",
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks
    );

    let (wired, wired_stages) =
        grit::live::arranged(width, grid, &plan, None, coarse).expect("wired");
    let wired_groups: Vec<&[Groups]> = wired_stages.iter().map(Vec::as_slice).collect();
    let over_box = sand::BOX * sand::BOX / width;

    let whole = [
        grit::live::tick(width).expect("tick"),
        sand::brush(width, grid, false).expect("brush"),
        sand::settle(width, grid).expect("settle"),
    ];
    let whole_groups = [
        Groups::Fixed(1),
        Groups::Fixed(over_box),
        Groups::Fixed(grid.quads().div_ceil(width)),
    ];

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = wired.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).expect("wired");
    let modules: Vec<&[u32]> = whole.iter().map(Vec::as_slice).collect();
    let mut full = host.resident(&sizes, &modules).expect("whole");

    let start = room(grid);
    for held in [&mut live, &mut full] {
        clear_all(held, &sizes);
        held.write(STATE as usize, &start).expect("state");
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .expect("params");
    }

    grit::live::seed(&mut live, grid, &plan).expect("seeded");

    let mut listed_total = 0_u64;
    let mut smallest = u32::MAX;
    let mut busiest = 0_u32;

    for frame in 0..frames {
        for held in [&mut live, &mut full] {
            held.write_at(PARAMS as usize, BRUSH, &brushing(frame))
                .expect("brush");
        }
        live.staged(&wired_groups).expect("wired");
        full.frame(&whole_groups).expect("whole");

        let there = live.read(STATE as usize, cells).expect("wired state");
        let here = full.read(STATE as usize, cells).expect("whole state");

        if let Some(at) = there.iter().zip(&here).position(|(a, b)| a != b) {
            let (row, column) = (at as u32 / grid.wide, at as u32 % grid.wide);

            let phase = frame & 1;
            let quad_x = (column.wrapping_sub(phase)) / 2;
            let quad_y = (row.wrapping_sub(phase)) / 2;
            let quad = quad_y * grid.across() + quad_x;
            let flags = live.read(AWAKE as usize, quads as usize).expect("awake");
            let listed_now = live
                .read(GRAND as usize, 1)
                .expect("grand")
                .first()
                .copied()
                .unwrap_or(0);
            let list = live
                .read(DENSE as usize, (listed_now as usize).min(quads as usize))
                .unwrap_or_default();
            eprintln!(
                "  quad {quad} at ({quad_x}, {quad_y}), phase {phase}: flag now {}, in the list {}, \
                 list holds {listed_now}",
                flags.get(quad as usize).copied().unwrap_or(9),
                list.contains(&quad)
            );
            panic!(
                "frame {frame}: cell ({column}, {row}) is {} through the list and {} over the whole \
                 grid — a quad that should have woken did not",
                there[at], here[at]
            );
        }

        let listed = live
            .read(GRAND as usize, 1)
            .expect("grand")
            .first()
            .copied()
            .unwrap_or(0);
        listed_total += u64::from(listed);
        smallest = smallest.min(listed);
        busiest = busiest.max(listed);
    }

    let mean = listed_total / u64::from(frames);
    let saving = f64::from(quads) / (mean.max(1) as f64);
    println!(
        "  {frames} frames agreed cell for cell. The list averaged {mean} quads of {quads} \
         — {saving:.1}× fewer — peaked at {busiest} and fell to {smallest} once the pile settled."
    );

    assert!(
        mean < u64::from(quads),
        "the list averaged every quad there is, so nothing was ever asleep"
    );
    assert!(
        busiest > 0,
        "the list was never anything but empty, so the pour woke nothing"
    );
    if until_quiet {
        assert_eq!(
            smallest, 0,
            "the list never emptied, so something stayed awake after the sand stopped"
        );
    } else {
        eprintln!("  the list fell to {smallest}; emptying is not what this size is run for");
    }
}

#[test]
fn the_list_is_shorter_and_never_different() {
    against_the_whole_grid(false, Grid::of(WIDE, DEEP), FRAMES, true);
}

#[test]
fn the_coarse_list_is_shorter_and_never_different_either() {
    against_the_whole_grid(true, Grid::of(WIDE, DEEP), FRAMES, true);
}

#[test]
fn the_coarse_list_agrees_at_a_size_worth_using_it_at() {
    against_the_whole_grid(true, Grid::of(WIDE * 4, DEEP * 4), SETTLING, false);
}

fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).expect("cleared");
    }
}

#[test]
fn a_host_says_how_much_memory_it_has() {
    let Some(host) = device("a_host_says_how_much_memory_it_has") else {
        return;
    };
    let (heap, ceiling) = (host.heap(), host.ceiling());
    eprintln!(
        "  {} — heap {heap} bytes, ceiling {ceiling} bytes",
        host.name()
    );

    assert!(heap > 0, "a device with no device-local heap at all");
    assert!(
        heap >= ceiling,
        "the largest allocation {ceiling} is bigger than the heap {heap} it comes out of"
    );
    assert!(
        heap >= 64 * 1024 * 1024,
        "a heap of {heap} bytes is not a device this runs on"
    );
}
