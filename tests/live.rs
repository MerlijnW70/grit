//! The wired loop against the whole grid, frame for frame.
//!
//! # The one check that matters, and why it is not a reference
//!
//! Everything else in this crate compares a device against a host that computes the same thing a
//! different way. That cannot work here: a host reference for the wake set would derive it from the
//! same rule the kernel does, agree perfectly, and be wrong in the same place.
//!
//! So the comparison is against **a different algorithm**. The same pour is run twice: once through
//! the work list, and once by stepping every quad in the grid whether it needs it or not. The second
//! is obviously correct and obviously slow, and it knows nothing about waking. If the two grids
//! agree cell for cell on every frame, the wake set missed nothing — because anything it missed is a
//! grain that stopped in one run and kept falling in the other.
//!
//! That is the whole argument for compaction stated as a test: *the list is allowed to be shorter,
//! and not allowed to be different.*

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{AWAKE, BINDINGS, BRUSH, DENSE, GRAND, PARAMS, PIXELS, STATE};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER};
use grit::sweep;
use grit::{Groups, Host, Resident};

/// The world under test.
const WIDE: u32 = 512;
const DEEP: u32 = 256;

/// How many frames the two runs are compared over.
///
/// **Enough that the list reaches nought**, which is what the last assertion here demands and what a
/// scene with water in it takes a while to do: a runner moves one cell a frame, and levelling a pool
/// five hundred columns wide is thousands of them. Measured on this scene — the list fell to nine at
/// four thousand frames and to nought between four thousand and four and a half. Six is that with
/// margin, and costs the test about four seconds.
const FRAMES: u32 = 6000;

/// How long the pour lasts, in frames.
const POURING: u32 = 90;

/// And how long the water lasts after it, in frames.
const WETTING: u32 = 180;

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

/// A box with a floor, two sides, and a shelf to pile against.
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

/// The brush the two runs share, so the only difference between them is the dispatch.
///
/// **Sand first and then water**, because they fail differently and this test used to see only one
/// of them. Sand is what proves the list can reach nothing: a heap stops, and a wake rule keyed to
/// movement stops with it. Water is what proves the list is never *wrong*: a runner crosses the
/// world sideways through cells that are not changing, which is exactly the case a movement-keyed
/// rule would lose — and, since `sand::TIRED` went in, the case where a cell's own history decides
/// whether it moves. A rule that reads history is a rule with state, and state is the thing a
/// listed run and a whole-grid run can disagree about.
fn brushing(frame: u32) -> [u32; 4] {
    let material = match frame {
        0..POURING => SAND,
        POURING..WETTING => WATER,
        _ => AIR,
    };
    [190, 30, 36, material]
}

#[test]
fn the_list_is_shorter_and_never_different() {
    let Some(host) = device("live") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
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
        "{} — subgroup {width}, {}×{} cells in {quads} quads, {} blocks",
        host.name(),
        grid.wide,
        grid.deep,
        plan.blocks
    );

    // The wired loop — both hierarchies, in the order `live::wired` insists on.
    let (wired, wired_stages) = grit::live::wired(width, grid, &plan, None).expect("wired");
    let wired_groups: Vec<&[Groups]> = wired_stages.iter().map(Vec::as_slice).collect();
    let over_box = sand::BOX * sand::BOX / width;

    // The whole grid, which knows nothing about waking.
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
        // **The dump base, and leaving it at nought is a buffer overwritten by its own discards.**
        // Every conditional write in this crate lands here when the condition is false; at nought
        // that is slot zero of whatever it was avoiding. The frame counter starts one below nought
        // so the first increment makes it nought.
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .expect("params");
    }

    // **Seeded, because the first frame has been told nothing.** Every quad awake, every block hot,
    // and both lists the identity — which is what "look at everything" means when nothing has said
    // otherwise yet.
    grit::live::seed(&mut live, grid, &plan).expect("seeded");

    let mut listed_total = 0_u64;
    let mut smallest = u32::MAX;
    let mut busiest = 0_u32;

    for frame in 0..FRAMES {
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

            // What the wake set actually said about the quad this cell is in, either phase.
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

    let mean = listed_total / u64::from(FRAMES);
    let saving = f64::from(quads) / (mean.max(1) as f64);
    println!(
        "  {FRAMES} frames agreed cell for cell. The list averaged {mean} quads of {quads} \
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
    // **Nought is the right answer, not a failure.** A settled pile has nothing to step, and a wake
    // rule that could not say so would keep the whole pile in the list forever — which is exactly
    // what waking on *content* did, at a third of this saving.
    assert_eq!(
        smallest, 0,
        "the list never emptied, so something stayed awake after the sand stopped"
    );
}

/// Zero every binding, so nothing carries over from an allocation.
fn clear_all(held: &mut Resident<'_>, sizes: &[usize; BINDINGS as usize]) {
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).expect("cleared");
    }
}

/// What the device says it has, against what it says it will allocate at once.
///
/// `Host::heap` and `Host::ceiling` answer different questions and a caller sizing a cache wants
/// the first. They are far apart on a real card — about four gigabytes against sixteen — and a
/// caller that reached for the ceiling because there was no heap to ask would size its cache to a
/// quarter of the card without knowing it.
#[test]
fn a_host_says_how_much_memory_it_has() {
    let Some(host) = device("a_host_says_how_much_memory_it_has") else {
        return;
    };
    let (heap, ceiling) = (host.heap(), host.ceiling());
    eprintln!("  {} — heap {heap} bytes, ceiling {ceiling} bytes", host.name());

    assert!(heap > 0, "a device with no device-local heap at all");
    assert!(
        heap >= ceiling,
        "the largest allocation {ceiling} is bigger than the heap {heap} it comes out of"
    );
    // A device this can run a compute pass on has more than a token's worth of memory. Loose on
    // purpose: the point is that the number is the heap's and not a placeholder.
    assert!(heap >= 64 * 1024 * 1024, "a heap of {heap} bytes is not a device this runs on");
}
