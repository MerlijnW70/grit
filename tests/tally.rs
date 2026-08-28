#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;
use grit::layout::{BRUSH, COARSE, DENSE, GRAND, PARAMS, STATE, TALLY, TALLY_WORDS};
use grit::sand::{AIR, Grid, SAND, WALL, WATER, material_of};
use grit::sweep;
use grit::{Groups, Host};

const WIDE: u32 = 512;
const DEEP: u32 = 256;

const FRAMES: u32 = 120;

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

    let (passes, stages) = grit::live::wired(width, grid, &plan, None).expect("wired");
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).expect("resident");
    for (binding, &words) in sizes.iter().enumerate() {
        live.write(binding, &vec![0_u32; words]).expect("cleared");
    }
    live.write(STATE as usize, &room(grid)).expect("state");
    live.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .expect("params");
    grit::live::seed(&mut live, grid, &plan).expect("seeded");

    let mut busiest = 0_u32;
    let mut ever_moved = 0_u64;

    for frame in 0..FRAMES {
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

        let material = match frame {
            0..40 => SAND,
            40..80 => WATER,
            _ => AIR,
        };
        live.write_at(PARAMS as usize, BRUSH, &[190, 30, 36, material])
            .expect("brush");
        live.staged(&groups).expect("dispatched");

        let state = live.read(STATE as usize, cells).expect("state");
        let tally = live.read(TALLY as usize, TALLY_WORDS).expect("tally");

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

    assert!(
        busiest > 0,
        "nothing ever moved, so the agreement is between two zeroes"
    );
}
