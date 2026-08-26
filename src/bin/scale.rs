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
//! `Resident::frame` times a submit and a fence around the work, and on this machine that costs
//! 37 µs whatever is inside it — `tests/floor.rs` prices it. Measured that way every size looked
//! like "a hundred and something", and a change that halved the work was invisible.
//!
//! `Resident::frames` records twenty frames into one command buffer. The round trip is paid once
//! and the work twenty times, so what comes back is the work.
//!
//! # Six arrangements, all measured on one afternoon
//!
//! **Every column was taken on one machine state against one `simdr`.** Each is a worktree at the
//! commit that introduced it, built and run round-robin three times, and all eighteen runs reported
//! `listed` at 768 — the same program throughout. Host clock, µs:
//!
//! ```text
//!         cells       gated      listed      staged    narrowed        kept         now
//!       262 144   36.6–36.8   57.9–61.8   43.3–44.3   33.9–34.3   27.4–27.6   17.0–17.7
//!     1 048 576   37.0–37.4   57.6–63.6   42.9–44.0   34.1–36.6   27.5–29.5   21.7–22.4
//!     4 194 304   41.4–42.7   58.0–58.7   43.6–44.8   34.4–36.3   27.9–28.2   28.1–29.1
//!    16 777 216   60.3–61.5   59.4–62.0   45.3–46.8   36.2–36.3   29.5–29.5   29.6–31.0
//! ```
//!
//! *Gated* is `47cf09a`: a workgroup a block, every block, with a vote that lets a cold one exit
//! early. *Listed* is `da7c7b8`, the coarse hierarchy. *Staged* is `a7d9fce`, the same passes in
//! eleven stages. *Narrowed* is `ea75656`, `DRAW_INDIRECT` in four barriers instead of ten. *Kept*
//! is `9eb4577`, the command buffer recorded once and submitted again. *Now* is `HEAD`.
//!
//! **Two columns this file used to carry are gone, and the reason is the point.** *Ungated* — no
//! vote at all — read 66, 92, 161 and 323, and *gated* read 79, 79, 79 and 144. Both arrived in
//! `245a786`, "Recover grit from the conversation log": they were taken on a codebase that is not in
//! this history, by a method that cannot be inspected. *Ungated* has no commit here to retake it at,
//! because the vote is in that first commit. *Gated* has one, and at it the numbers are the row
//! above — building it against the `simdr` of the day before moves nothing, so the engine is not the
//! difference.
//!
//! **On the numbers that are here, the coarse hierarchy did not pay for itself when it went in.**
//! *Listed* is 21 µs worse than *gated* at the smallest size and a wash at the largest: it adds five
//! passes to a frame that still put a wide barrier after every pass, and 21.3 µs over five passes is
//! 4.3 µs each — which is the wide barrier, priced this time by adding rather than removing. The
//! three columns after it are what made the hierarchy worth keeping.
//!
//! # What the hierarchy is worth now
//!
//! Three arrangements of `HEAD`. The middle dispatches `decay`, `mark` and `gather` over every block
//! instead of over the coarse list; the right one also drops the five passes that build that list.
//! Device clock, µs, three runs each, and `tests/live.rs` passes in all three — so they compute the
//! same simulation and differ only in price:
//!
//! ```text
//!         cells   over the list   list built, not used   neither built nor used
//!       262 144     24.40–24.49            20.81–20.85              14.38–14.45
//!     1 048 576     24.95–25.03            25.61–25.64              19.16–19.19
//!     4 194 304     24.95–25.04            43.63–43.66              37.41–49.02
//!    16 777 216     25.13–25.18          115.44–115.50            108.81–108.86
//! ```
//!
//! **Building the list costs 6.2–6.7 µs at every size**, which is the middle column against the
//! right one — a constant, because `Plan::over` bounds the tiles at `per_block` and one workgroup
//! scans them all.
//!
//! **Using it costs 10 µs of 24 at 262 144 cells and saves 84 µs of 109 at 16 777 216.** The two
//! cross at 2048 blocks, where they read 25.0 and 25.4 µs and the order changes between runs — so
//! `live::COARSE_FROM` is 2048, and `live::wired` picks the arrangement instead of always taking
//! the list. `tests/coarse.rs` measures both sides of that; `decisions/DR-0003` carries the figures.
//!
//! # The stages, the barriers and the recording
//!
//! **The last three columns of the first table are not about work at all.** By *listed* the frame
//! was sixteen passes with a barrier between each. *Staged* is those sixteen in eleven stages, which
//! is `crate::live::wired` saying which of them do not read what their neighbours write.
//!
//! *Narrowed* is `DRAW_INDIRECT` named only in the four barriers that precede a dispatch reading its
//! count from a buffer. `Resident` had it in all ten, on the written grounds that naming it "costs
//! nothing and cannot be forgotten" — the second half was true and the first was not, by a fifth of
//! the frame. `tests/floor.rs` prices a narrowed barrier at 1004 ns and a dispatch at 224 ns, on an
//! apparatus with no simulation in it at all.
//!
//! *Kept* is the command buffer recorded once and submitted again. Nothing that changes between
//! frames is in a recording — a dispatch names a pipeline and a count, and `Groups::From` reads even
//! the count out of a buffer at execution — so a frame the same shape as the last needs no second
//! recording. It needed a second command buffer to be worth anything at all: transfers were recorded
//! into the frame's, so the four brush words this file writes between batches threw the frame away
//! every time. *Recorded sixteen times, reused nought* before; two and fourteen after.
//!
//! **Every column of the first table is a host clock, and the file prints a device one beside it.**
//! `Resident` writes two timestamps around the recorded work, and *kept* against the two largest
//! sizes of *now* says those two writes cost nothing this apparatus can see.
//!
//! **What the table is for** is that nothing in the frame is proportional to the grid: four times
//! the world, from four million cells to sixteen, is a tenth of a microsecond on the card. Below
//! four million *now* is smaller than *kept* rather than equal to it, because `live::wired` stops
//! building a hierarchy that costs more than it saves — which is the two left columns of the second
//! table, acted on.
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
        "  {:>10}  {:>9}  {:>7}  {:>10}  {:>10}  {:>8}  {:>8}  {:>7}",
        "cells", "quads", "blocks", "wall", "on card", "listed", "memory", "of grid"
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
        // is 37 µs of submit and fence whatever is inside it, so it cannot tell a
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

    let card = resident.on_device();

    let asked = u64::from(FRAMES.div_ceil(BATCH * 2));
    // **`listed` is the canary on any timing taken from this file, and it is necessary rather than
    // sufficient.** A change that alters what the loop computes hands back a timing that belongs to
    // another program, and this column is how that shows: dropping `DRAW_INDIRECT` from every
    // barrier once reported a third off the frame with `listed` at 471 instead of 768, because the
    // settle was dispatching over a stale count.
    //
    // It does not catch everything. Setting five passes to `Groups::Fixed(0)` left `listed` at 768
    // and was still a different program — without those passes the hot-block list is never rebuilt
    // and three passes fall back to a workgroup a block, which changes how many workgroups run and
    // not how many quads are listed. `decisions/DR-0001`.
    let listed = listed_total / asked.max(1);
    let share = f64::from(quads) / (listed.max(1) as f64);
    println!(
        "  {:>10}  {quads:>9}  {:>7}  {:>10.3?}  {:>10.3?}  {listed:>8}  {:>5} MB  {share:>6.0}×",
        grid.cells(),
        plan.blocks,
        spent / FRAMES.div_ceil(BATCH),
        card.unwrap_or_default(),
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
