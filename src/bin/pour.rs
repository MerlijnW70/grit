use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BRUSH, COARSE, GRAND, LOOKING, PARAMS, PIXELS, STATE, TALLY, TALLY_WORDS};
use grit::sand::{AIR, Grid, SAND, View, WALL, WATER};
use grit::sweep;
use grit::{Groups, Host};

const WIDE: u32 = 4096;
const DEEP: u32 = 4096;

const VIEW_WIDE: u32 = 1024;
const VIEW_DEEP: u32 = 512;

const STEP: u32 = 6;

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

    let (passes, stages) = grit::live::wired(width, grid, &plan, Some(view))?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
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

    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;

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

    let mut looking = (grid.wide / 2, grid.deep / 2);
    resident.write_at(PARAMS as usize, LOOKING, &[looking.0, looking.1])?;

    print!("waking the driver... ");
    let warm = Instant::now();
    drop(resident.staged_reading(&groups, PIXELS as usize, view.pixels())?);
    println!("{:?}", warm.elapsed());

    let mut frames = 0_u32;
    let mut cost = Duration::ZERO;
    let mut clock = Instant::now();
    let mut following = false;
    let mut was_pressed = false;
    let mut target = looking;

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let pressed = window.is_key_down(minifb::Key::F);
        if pressed && !was_pressed {
            following = !following;
        }
        was_pressed = pressed;

        let steered = flown(&window, grid, looking);
        looking = if steered == looking && following {
            drifted(grid, looking, target)
        } else {
            target = steered;
            steered
        };
        resident.write_at(PARAMS as usize, LOOKING, &[looking.0, looking.1])?;

        let sanding = window.get_mouse_down(minifb::MouseButton::Left);
        let wetting = window.get_mouse_down(minifb::MouseButton::Right);
        let (x, y) = window
            .get_mouse_pos(minifb::MouseMode::Discard)
            .unwrap_or((0.0, 0.0));
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

        let (spent, picture) = resident.staged_reading(&groups, PIXELS as usize, view.pixels())?;
        cost += spent;
        window.update_with_buffer(&picture, view.wide as usize, view.deep as usize)?;

        frames += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            let listed = resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0);
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

fn middle(tally: &[u32]) -> Option<(u32, u32)> {
    let moved = tally.first().copied().unwrap_or(0);
    if moved == 0 {
        return None;
    }
    let across = tally.get(3).copied().unwrap_or(0) / moved;
    let down = tally.get(4).copied().unwrap_or(0) / moved;
    Some((across << (COARSE + 1), down << (COARSE + 1)))
}

fn drifted(grid: Grid, (x, y): (u32, u32), want: (u32, u32)) -> (u32, u32) {
    let view = View::of(VIEW_WIDE, VIEW_DEEP);
    let step = |now: u32, aim: u32, half: u32, span: u32| {
        let corner = (aim + span - half) & (span - 1);
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

fn room(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
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
