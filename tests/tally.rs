//! The one number that comes back by reduction, held to a count done by hand.
//!
//! # Why a telemetry number is worth a test at all
//!
//! Because it is the easiest thing in the crate to get quietly wrong and never notice. A frame
//! counter that is off by one still counts up. A grain count that is short by the last workgroup
//! still rises when you pour and falls when you stop, and it will look right on a title bar for as
//! long as nobody adds it up themselves.
//!
//! The failures it is actually exposed to are specific, and each of them survives a glance:
//!
//! * **The reduction counted once a lane instead of once a workgroup.** An atomic add is not
//!   idempotent, and every lane of a subgroup holds the same total after `reduce_sum` — so the
//!   obvious code multiplies every number by the subgroup width. On a thirty-two-wide part that is
//!   a plausible-looking number thirty-two times too big.
//! * **The accumulator was never cleared.** Then it is the sum over the whole run, which also goes
//!   up when you pour. The same mistake cost this crate a work list once already; `live::wipe` says
//!   so at length.
//! * **The slots past the end of the list were counted.** They step a spare quad off the end of the
//!   grid, and their contents are not the world's.
//!
//! So the check is a count done the slow way: read the work list the settle is about to use, run the
//! frame, and add up the cells of exactly those quads on the host. Not a bound, not a trend — the
//! same number.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::{self, Plan};
use grit::layout::{
    AWAKE, BINDINGS, BRUSH, COARSE, DENSE, GRAND, HOT, INDIRECT, PARAMS, STATE, TALLY, TALLY_WORDS,
};
use grit::sand::{self, AIR, Grid, SAND, WALL, WATER, material_of};
use grit::sweep;
use grit::{Groups, Host};

/// The world under test.
const WIDE: u32 = 512;
const DEEP: u32 = 256;

/// How many frames are checked.
const FRAMES: u32 = 120;

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

/// A box with a floor and two sides.
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

/// The four cells of `quad` at this frame's partition origin — the same arithmetic `live::corners`
/// does on the device, written out.
fn corners(grid: Grid, quad: u32, phase: u32) -> [usize; 4] {
    let column = ((quad % grid.across()) * 2 + phase) & (grid.wide - 1);
    let row = ((quad / grid.across()) * 2 + phase) & (grid.deep - 1);
    let right = (column + 1) & (grid.wide - 1);
    let under = (row + 1) & (grid.deep - 1);
    [
        (row * grid.wide + column) as usize,
        (row * grid.wide + right) as usize,
        (under * grid.wide + column) as usize,
        (under * grid.wide + right) as usize,
    ]
}

#[test]
fn the_reduction_says_what_a_count_by_hand_says() {
    let Some(host) = device("tally") else {
        return;
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            eprintln!("SKIPPED tally at width {width}: {too_big}");
            return;
        }
    };
    let cells = grid.cells();
    println!(
        "{} — subgroup {width}, {}×{} cells in {quads} quads",
        host.name(),
        grid.wide,
        grid.deep
    );

    let passes = [
        counter(width).expect("counter"),
        grit::live::decay(width).expect("decay"),
        grit::live::wipe(width).expect("wipe"),
        sand::brush(width, grid, true).expect("brush"),
        grit::live::settle(width, grid).expect("settle"),
        grit::live::mark(width).expect("mark"),
        compact::scan(width, sweep::COUNTS, compact::BASES, compact::TOTALS).expect("counts"),
        compact::scan(width, compact::TOTALS, compact::TOTALS, compact::GRAND).expect("totals"),
        compact::gather(width).expect("gather"),
        compact::prepare(width, width).expect("prepare"),
    ];
    let groups = [
        Groups::Fixed(1),
        Groups::Fixed(plan.blocks),
        Groups::Fixed(1),
        Groups::Fixed(sand::BOX * sand::BOX / width),
        Plan::listed(),
        Groups::Fixed(plan.blocks),
        Groups::Fixed(plan.tiles),
        Groups::Fixed(1),
        Groups::Fixed(plan.blocks),
        Groups::Fixed(1),
    ];

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).expect("resident");
    for (binding, &words) in sizes.iter().enumerate() {
        live.write(binding, &vec![0_u32; words]).expect("cleared");
    }
    live.write(STATE as usize, &room(grid)).expect("state");
    live.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    live.write(AWAKE as usize, &vec![2_u32; quads as usize])
        .expect("awake");
    live.write(DENSE as usize, &(0..quads).collect::<Vec<u32>>())
        .expect("dense");
    live.write(GRAND as usize, &[quads]).expect("grand");
    live.write(INDIRECT as usize, &[quads.div_ceil(width), 1, 1])
        .expect("indirect");
    live.write(HOT as usize, &vec![2_u32; plan.blocks as usize])
        .expect("hot");

    let mut busiest = 0_u32;
    let mut ever_moved = 0_u64;

    for frame in 0..FRAMES {
        // **Before the frame**, because `gather` rewrites the list during it: what the settle is
        // about to read is what is there now, not what is there afterwards.
        let listed = live
            .read(GRAND as usize, 1)
            .expect("grand")
            .first()
            .copied()
            .unwrap_or(0)
            .min(quads);
        let list = live
            .read(DENSE as usize, listed as usize)
            .expect("dense list");

        // Sand for a while, then water, so both are in the world and both are counted.
        let material = match frame {
            0..40 => SAND,
            40..80 => WATER,
            _ => AIR,
        };
        live.write_at(PARAMS as usize, BRUSH, &[190, 30, 36, material])
            .expect("brush");
        live.frame(&groups).expect("dispatched");

        let state = live.read(STATE as usize, cells).expect("state");
        let tally = live.read(TALLY as usize, TALLY_WORDS).expect("tally");

        // The same count, by hand, over exactly the quads the list named.
        let phase = frame & 1;
        let (mut wet, mut dry) = (0_u32, 0_u32);
        for &quad in &list {
            for at in corners(grid, quad, phase) {
                match material_of(state[at]) {
                    WATER => wet += 1,
                    SAND => dry += 1,
                    _ => {}
                }
            }
        }

        assert_eq!(
            tally[1], wet,
            "frame {frame}: the device counted {} water cells over {listed} quads and the host {wet}",
            tally[1]
        );
        assert_eq!(
            tally[2], dry,
            "frame {frame}: the device counted {} sand cells over {listed} quads and the host {dry}",
            tally[2]
        );

        // What moved cannot exceed what was looked at, and the quads that moved cannot exceed the
        // quads that were listed. Both would fail if a lane's share were counted more than once.
        assert!(
            tally[0] <= listed * 4,
            "frame {frame}: {} cells moved out of {listed} quads, which is more than they hold",
            tally[0]
        );
        assert!(
            tally[5] <= listed,
            "frame {frame}: {} quads moved out of {listed} listed",
            tally[5]
        );

        // The centre of the motion, which is a weighted sum and so has a bound of its own.
        let far = (grid.across() >> COARSE) * tally[0];
        assert!(
            tally[3] <= far && tally[4] <= (grid.down() >> COARSE) * tally[0],
            "frame {frame}: the centre sums are {} and {}, past what {} moving cells can reach",
            tally[3],
            tally[4],
            tally[0]
        );

        busiest = busiest.max(tally[0]);
        ever_moved += u64::from(tally[0]);
    }

    println!(
        "  {FRAMES} frames counted twice and agreed. The busiest frame moved {busiest} cells; \
         {ever_moved} cell-moves altogether."
    );

    // **A tally that was always nought would satisfy every equality above.** The host would agree
    // with it, because the host would be counting the same empty list.
    assert!(
        busiest > 0,
        "nothing ever moved, so the agreement is between two zeroes"
    );
}

/// One word, incremented on the device.
fn counter(subgroup: u32) -> Result<Vec<u32>, simdr::lanes::LaneError> {
    use simdr::kernel::{Kernel, Shape};
    use simdr::lanes::U32;

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();

    let now = kernel.load_at(PARAMS, slot)?;
    let next = kernel.module().i_add(element, now, one)?;
    kernel.store_at(PARAMS, slot, next)?;
    kernel.finish()
}
