//! `cargo run --release --bin pour` — sixteen million cells of sand, seen through a window.
//!
//! # Seventeen passes, one submission, and six words from the host
//!
//! ```text
//!   tick     1              the frame number, and the partition's phase
//!   decay    the hot list   every wake counter, one closer to nothing
//!   wipe     1              both tile totals and the tally, back to nothing
//!   wipe     tiles          every block's count, back to nothing
//!   brush    a box          what the pointer holds, and the quads it touched
//!   settle   the work list  one Margolus step over what the last frame woke
//!   hot ×5   tiles          which blocks hold anything, compacted into a list
//!   mark     the hot list   the counters, scanned into buckets and counts
//!   scan ×2                 the blocks against each other
//!   gather   the hot list   the buckets into one contiguous list
//!   prepare  1              how many workgroups the *next* settle runs
//!   paint    the window     the visible cells, as colours
//! ```
//!
//! **Nothing in that list is the size of the world.** `live::wired` says why the five coarse passes
//! sit where they do, which is a correctness matter rather than a preference.
//!
//! The only thing crossing to the device per frame is six words: where the pointer is, how wide and
//! what of, and which corner of the world the window is at. **The size of the settle never crosses
//! at all** — `prepare` wrote it into a buffer and Vulkan reads it at dispatch time, so the host does
//! not learn how much work there was unless it asks, which the title bar does twice a second.
//!
//! # The window is the point of this pass
//!
//! The world is 4096×4096. Simulating it costs a couple of hundred microseconds; *painting* all of it
//! would cost sixty-seven megabytes of readback a frame, about thirty-eight milliseconds — two
//! hundred times the simulation. So `paint` runs one invocation a **pixel** rather than a cell, and
//! WASD moves the corner. Looking is now priced by the window and the world is free to be enormous.
//!
//! # The numbers in the title bar, and where they come from
//!
//! Everything else in this crate compacts — a prefix sum, to know *which* quads and in what order.
//! The telemetry is the other half of the lane model: a **reduction**, which does not care which,
//! only how many, and hands one answer back for the whole device. It is folded into the settle
//! rather than being a pass, because the settle already has the four cells in registers; the whole
//! cost is one `reduce_sum` and one atomic a workgroup, six times over.
//!
//! Press **F** and the window steers itself to the centre of the motion — which is that same
//! reduction, weighted: the sum of each moving quad's coordinate divided by the number of cells that
//! moved. Nothing else in the loop knows where anything is.
//!
//! # What to watch
//!
//! The number beside the frame time is how many quads the settle actually stepped, out of four
//! million. Pour sand, and it climbs into the hundreds; stop, and it falls to *nought* once the pile
//! stops moving — while the grid it is not looking at stays four million quads the whole time.
//!
//! Water is the other half of that. It runs sideways until it is level, so a pool keeps a few dozen
//! quads awake long after it looks still: a hundred and twenty-eight cells do not divide evenly into
//! sixty-two columns, and the remainder paces about looking for somewhere to be. `tests/sand.rs`
//! measures exactly how much of it there is and says why no rule reading one 2×2 block can do
//! better.

use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BRUSH, COARSE, GRAND, LOOKING, PARAMS, PIXELS, STATE, TALLY, TALLY_WORDS};
use grit::sand::{self, AIR, Grid, SAND, View, WALL, WATER};
use grit::sweep;
use grit::{Groups, Host};

/// How big the world is. Both powers of two, which `Grid::of` insists on.
const WIDE: u32 = 4096;
const DEEP: u32 = 4096;

/// How much of it the window shows. Powers of two for the same reason.
const VIEW_WIDE: u32 = 1024;
const VIEW_DEEP: u32 = 512;

/// How far WASD moves the corner in one frame, in cells.
const STEP: u32 = 6;

/// How wide the pointer paints, in cells.
const REACH: u32 = 7;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let view = View::of(VIEW_WIDE, VIEW_DEEP);
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;
    println!(
        "{} — subgroup {width}, {}×{} cells in {quads} quads, seen {}×{}",
        host.name(),
        grid.wide,
        grid.deep,
        view.wide,
        view.deep
    );

    // The loop, in the order `live::wired` insists on — and the picture on the end, because how
    // much of the world is being looked at is this program's business and not the loop's.
    let (mut passes, mut groups) = grit::live::wired(width, grid, &plan)?;
    passes.push(sand::paint(width, grid, view)?);
    groups.push(Groups::Fixed(
        u32::try_from(view.pixels())
            .unwrap_or(u32::MAX)
            .div_ceil(width),
    ));
    let words: usize = passes.iter().map(Vec::len).sum();

    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.showing(view);
    let held: usize = sizes.iter().sum();
    println!(
        "{words} words of SPIR-V across {} passes, {} MB of buffers",
        passes.len(),
        held * 4 / (1 << 20)
    );
    let mut resident = host.resident(&sizes, &modules)?;

    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words])?;
    }
    print!("building the room... ");
    let laying = Instant::now();
    resident.write(STATE as usize, &room(grid))?;
    println!("{:?}", laying.elapsed());

    // The frame counter one below nought, and the dump base — which at nought would be slot zero of
    // whatever a discarded write was avoiding.
    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;

    // **Seeded, because the first frame has been told nothing.** Every quad awake, every block hot,
    // and both lists the identity — which is what "look at everything" means before anything has
    // said otherwise.
    grit::live::seed(&mut resident, grid, &plan)?;

    let mut window = minifb::Window::new(
        "grit — left button pours sand, right pours water, WASD flies, F follows",
        view.wide as usize,
        view.deep as usize,
        minifb::WindowOptions {
            scale: minifb::Scale::X1,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: true,
            ..minifb::WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    // Somewhere with a shelf under it, so the first pour has something to land on.
    let mut looking = (grid.wide / 2, grid.deep / 2);
    resident.write_at(PARAMS as usize, LOOKING, &[looking.0, looking.1])?;

    print!("waking the driver... ");
    let warm = Instant::now();
    drop(resident.frame_reading(&groups, PIXELS as usize, view.pixels())?);
    println!("{:?}", warm.elapsed());

    let mut frames = 0_u32;
    let mut cost = Duration::ZERO;
    let mut clock = Instant::now();
    let mut following = false;
    let mut was_pressed = false;
    let mut target = looking;

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        // An edge, not a level: a key held down for a fifth of a second would otherwise toggle
        // twelve times and land wherever it happened to stop.
        let pressed = window.is_key_down(minifb::Key::F);
        if pressed && !was_pressed {
            following = !following;
        }
        was_pressed = pressed;

        let steered = flown(&window, grid, looking);
        looking = if steered == looking && following {
            drifted(grid, looking, target)
        } else {
            // A hand on the keys takes it back, and takes the target with it.
            target = steered;
            steered
        };
        resident.write_at(PARAMS as usize, LOOKING, &[looking.0, looking.1])?;

        let sanding = window.get_mouse_down(minifb::MouseButton::Left);
        let wetting = window.get_mouse_down(minifb::MouseButton::Right);
        let (x, y) = window
            .get_mouse_pos(minifb::MouseMode::Discard)
            .unwrap_or((0.0, 0.0));
        // The pointer is in *window* coordinates and the brush works in world ones, so the corner
        // has to be added back on — the one translation a viewport costs the host.
        let column = (looking.0 + x as u32) & (grid.wide - 1);
        let row = (looking.1 + y as u32) & (grid.deep - 1);
        let material = if wetting {
            WATER
        } else if sanding {
            SAND
        } else {
            AIR
        };
        resident.write_at(
            PARAMS as usize,
            BRUSH,
            &[column, row, REACH * REACH, material],
        )?;

        let (spent, picture) = resident.frame_reading(&groups, PIXELS as usize, view.pixels())?;
        cost += spent;
        window.update_with_buffer(&picture, view.wide as usize, view.deep as usize)?;

        frames += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            // **Only twice a second**, because asking is a submit and a fence for one word — and the
            // dispatch that used it did not have to ask at all.
            let listed = resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0);
            // **Six words that cost the frame nothing.** They were summed inside the settle, on
            // data already in registers, by one subgroup reduction and one atomic a workgroup —
            // `live::tally` says why that is one atomic and not thirty-two.
            let tally = resident.read(TALLY as usize, TALLY_WORDS)?;
            let moved = tally.first().copied().unwrap_or(0);
            let wet = tally.get(1).copied().unwrap_or(0);
            let dry = tally.get(2).copied().unwrap_or(0);
            if let Some(centre) = middle(&tally) {
                target = centre;
            }

            let seconds = clock.elapsed().as_secs_f64();
            window.set_title(&format!(
                "grit — {:.0} fps · {:.2} ms · {listed} of {quads} quads · {moved} cells moving · \
                 {dry} sand {wet} water · ({}, {}) of {}×{}{}",
                f64::from(frames) / seconds,
                cost.as_secs_f64() * 1000.0 / f64::from(frames.max(1)),
                looking.0,
                looking.1,
                grid.wide,
                grid.deep,
                if following { " · following" } else { "" },
            ));
            clock = Instant::now();
            frames = 0;
            cost = Duration::ZERO;
        }
    }

    println!("poured.");
    Ok(())
}

/// Where the motion is, in cells, or nothing if nothing is moving.
///
/// The device summed `quad column x cells moved` and the same for rows; dividing by the total gives
/// the weighted centre. Both sums are in coarse quads — see [`COARSE`] — so getting back to cells is
/// a shift by that plus one more for the two cells a quad is wide.
fn middle(tally: &[u32]) -> Option<(u32, u32)> {
    let moved = tally.first().copied().unwrap_or(0);
    if moved == 0 {
        return None;
    }
    let across = tally.get(3).copied().unwrap_or(0) / moved;
    let down = tally.get(4).copied().unwrap_or(0) / moved;
    Some((across << (COARSE + 1), down << (COARSE + 1)))
}

/// One frame's move of the window towards `want`, centred on it rather than cornered at it.
///
/// A fraction of the way each frame rather than a jump, because the centre of the motion hops about
/// by a few quads between readings and a window that tracked it exactly would shake.
fn drifted(grid: Grid, (x, y): (u32, u32), want: (u32, u32)) -> (u32, u32) {
    let view = View::of(VIEW_WIDE, VIEW_DEEP);
    let step = |now: u32, aim: u32, half: u32, span: u32| {
        // The corner that would centre the window on the target, wrapped into the world.
        let corner = (aim + span - half) & (span - 1);
        // Whichever way round is shorter: the world joins up, so a difference of nearly `span` is
        // really a small step the other way.
        let ahead = (corner + span - now) & (span - 1);
        let behind = span - ahead;
        let take = |gap: u32| (gap / 8).clamp(1, 24).min(gap);
        if ahead == 0 {
            now
        } else if ahead <= behind {
            (now + take(ahead)) & (span - 1)
        } else {
            (now + span - take(behind)) & (span - 1)
        }
    };
    (
        step(x, want.0, view.wide / 2, grid.wide),
        step(y, want.1, view.deep / 2, grid.deep),
    )
}

/// Where the window is after this frame's keys — wrapped, because the world is a torus.
fn flown(window: &minifb::Window, grid: Grid, (x, y): (u32, u32)) -> (u32, u32) {
    let quick =
        window.is_key_down(minifb::Key::LeftShift) || window.is_key_down(minifb::Key::RightShift);
    let step = if quick { STEP * 8 } else { STEP };

    let mut moved = (x, y);
    if window.is_key_down(minifb::Key::A) {
        moved.0 = moved.0.wrapping_sub(step);
    }
    if window.is_key_down(minifb::Key::D) {
        moved.0 = moved.0.wrapping_add(step);
    }
    if window.is_key_down(minifb::Key::W) {
        moved.1 = moved.1.wrapping_sub(step);
    }
    if window.is_key_down(minifb::Key::S) {
        moved.1 = moved.1.wrapping_add(step);
    }
    (moved.0 & (grid.wide - 1), moved.1 & (grid.deep - 1))
}

/// A world with a floor, sides, and shelves wherever you fly to.
///
/// At four thousand cells square there is no point placing furniture by hand: the shelves come from
/// a rule, so panning always finds something for the sand to pile on.
fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        // A bar every 96 rows, staggered so no two line up, and gapped so sand drains through.
        let shelf = row % 96 < 5;
        let slide = (row / 96) * 331;
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            let along = (column + slide) % 512;
            let bar = shelf && (40..430).contains(&along);
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = if edge || bar { WALL } else { AIR };
            }
        }
    }
    cells
}
