#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

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
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
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
            eprintln!("SKIPPED {width} {too_big}");
            return;
        }
    };
    let cells = grid.cells();
    println!(
        "{} {width} {} {} {quads}",
        host.name(),
        grid.wide,
        grid.deep
    );

    let (passes, stages) = grit::live::wired(width, grid, &plan, None).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut live = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        live.write(binding, &vec![0_u32; words]).unwrap();
    }
    live.write(STATE as usize, &room(grid)).unwrap();
    live.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    grit::live::seed(&mut live, grid, &plan).unwrap();

    let mut busiest = 0_u32;
    let mut ever_moved = 0_u64;

    for frame in 0..FRAMES {
        let listed = live
            .read(GRAND as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0)
            .min(quads);
        let list = live.read(DENSE as usize, listed as usize).unwrap();

        let material = match frame {
            0..40 => SAND,
            40..80 => WATER,
            _ => AIR,
        };
        live.write_at(PARAMS as usize, BRUSH, &[190, 30, 36, material])
            .unwrap();
        live.staged(&groups).unwrap();

        let state = live.read(STATE as usize, cells).unwrap();
        let tally = live.read(TALLY as usize, TALLY_WORDS).unwrap();

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

        assert_eq!(tally[1], wet);
        assert_eq!(tally[2], dry);

        assert!(tally[0] <= listed * 4);
        assert!(tally[5] <= listed);

        let far = (grid.across() >> COARSE) * tally[0];
        assert!(tally[3] <= far && tally[4] <= (grid.down() >> COARSE) * tally[0]);

        busiest = busiest.max(tally[0]);
        ever_moved += u64::from(tally[0]);
    }

    println!("{FRAMES} {busiest} {ever_moved}");

    assert!(busiest > 0);
}
