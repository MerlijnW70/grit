//! `cargo run --release --bin scale` — the same loop, at sizes up to sixteen million cells.
//!
//! # What is being asked
//!
//! Not "is it fast". `runner/examples/bench.rs` in `simdr` measured the arithmetic long ago, and
//! `loop_cost` here measured the frame. The question is whether the *shape* survives:
//!
//! * two levels of scan reach `per_block²` blocks and no further — 65 536 on a 32-lane part, which
//!   is 16.7 million quads, and 1024 on a four-lane one. Past that `Plan::over` refuses by name.
//! * the wake set is a fraction of the grid, and the fraction should not care how big the grid is.
//! * a frame at sixteen million must still fit inside a sixtieth of a second.
//!
//! # The timing is twenty frames to a submission, and that is not a detail
//!
//! `Resident::frame` times a submit and a fence around the work, and on this machine that round
//! trip is about a hundred microseconds whatever is inside it. Measured that way every size looked
//! like "a hundred and something", and a change that halved the work was invisible — worse,
//! the first version of the coarse gate *appeared to make things slower* and nearly went in the bin
//! on that evidence.
//!
//! `Resident::frames` records twenty frames into one command buffer. The round trip is paid once
//! and the work twenty times, so what comes back is the work. Four arrangements, measured:
//!
//! ```text
//!         cells     ungated       gated      listed      staged
//!       262 144       66 us       79 us       57 us       44 us
//!     1 048 576       92 us       79 us       57 us       44 us
//!     4 194 304      161 us       79 us       59 us       44 us
//!    16 777 216      323 us      144 us       60 us       46 us
//! ```
//!
//! *Ungated* is a workgroup a block, every block. *Gated* is the same dispatch with a vote that
//! lets a cold block exit early — which flattens the middle and leaves the largest size paying for
//! 3 × 16 384 workgroups that do one load and then nothing. *Listed* is the coarse hierarchy: those
//! three passes dispatched over a compacted list of the blocks that hold something, so a cold block
//! is not launched at all.
//!
//! **The last column is not about work at all.** By then the frame was sixteen passes with a barrier
//! between each, and a barrier is a full pipeline flush. Priced by taking five passes out at a size
//! where none of them had anything to do — 60.2 µs to 41.2 µs — which is about 3.8 µs a barrier, and
//! sixteen of those is very nearly the whole frame. *Staged* is the same sixteen passes in eleven
//! stages, which is `crate::live::wired` saying which of them do not read what their neighbours
//! write.
//!
//! **What the table is for** is the flatness rather than any one number: sixty-four times the world
//! for two microseconds more, with nothing in the frame proportional to the grid.
//!
//! # The pour is the same at every size
//!
//! One spout, in the same place, for the same number of frames. So the *absolute* amount of moving
//! sand is roughly constant and the grid around it grows — which is exactly the case a work list is
//! for, and exactly the case a pass over the whole grid is worst at. The last column is the ratio
//! between them, and it is the number this repository exists to print.

use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

/// How many frames each size runs.
const FRAMES: u32 = 300;

/// How many of them go into one submission.
const BATCH: u32 = 20;

/// How long the pour lasts.
const POURING: u32 = 200;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}\n", host.name());
    println!(
        "  {:>10}  {:>9}  {:>7}  {:>9}  {:>9}  {:>8}  {:>7}",
        "cells", "quads", "blocks", "frame", "listed", "memory", "of grid"
    );

    for power in [9_u32, 10, 11, 12] {
        let grid = Grid::of(1 << power, 1 << power);
        match once(&host, width, grid) {
            Ok(Some(())) => {}
            Ok(None) => {}
            Err(why) => println!("  {:>10}  refused: {why}", grid.cells()),
        }
    }

    println!(
        "\n  `listed` is how many quads the settle stepped, averaged over the run. `of grid` is that\n  \
         against every quad there is — what a pass over the whole world would have cost."
    );
    Ok(())
}

/// One size, run and reported.
fn once(host: &Host, width: u32, grid: Grid) -> Result<Option<()>, Box<dyn std::error::Error>> {
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            println!("  {:>10}  {quads:>9}  refused: {too_big}", grid.cells());
            return Ok(None);
        }
    };

    // No picture: this measures the loop, and what looking costs is `loop_cost`'s question.
    let (passes, stages) = grit::live::wired(width, grid, &plan, None)?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let words: usize = sizes.iter().sum();

    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held])?;
    }
    resident.write(STATE as usize, &room(grid))?;
    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    grit::live::seed(&mut resident, grid, &plan)?;

    // A discarded frame: the first submission of a pipeline pays for its compile.
    resident.staged(&groups)?;

    let mut spent = Duration::ZERO;
    let mut listed_total = 0_u64;
    for frame in (0..FRAMES).step_by(BATCH as usize) {
        let material = if frame < POURING { SAND } else { AIR };
        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])?;
        // **Twenty frames a submission, and the timing divided by twenty.** One frame's wall clock
        // is a hundred microseconds of submit and fence whatever is inside it, so it cannot tell a
        // pass that got cheaper from one that did not. See `Resident::frames`.
        spent += resident.staged_frames(&groups, BATCH)?;

        // Twice over the run, because asking is a submit and a fence.
        if frame % (BATCH * 2) == 0 {
            let listed = resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0);
            listed_total += u64::from(listed);
        }
    }

    let asked = u64::from(FRAMES.div_ceil(BATCH * 2));
    let listed = listed_total / asked.max(1);
    let share = f64::from(quads) / (listed.max(1) as f64);
    println!(
        "  {:>10}  {quads:>9}  {:>7}  {:>9.3?}  {listed:>9}  {:>6} MB  {share:>6.0}×",
        grid.cells(),
        plan.blocks,
        spent / FRAMES.div_ceil(BATCH),
        words * 4 / (1 << 20),
    );
    Ok(Some(()))
}

/// A box with a floor and two sides, whatever size it is.
fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge { WALL } else { AIR };
            }
        }
    }
    cells
}
