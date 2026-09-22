use std::time::Instant;

use grit::Host;
use grit::board::seated;
use grit::rule::{Kind, POINT};
use grit::sand::Grid;

const SIDE: u32 = 256;

const DEPTH: u32 = 24;

const BLOB: u32 = 9;

const MASS: u32 = 1 << 14;

const GROUND: u32 = 1 << 11;

const HAZE: u32 = 1 << 12;

const KNOB: u32 = 256;

fn blotted(grid: Grid, seed: u32) -> Vec<u32> {
    let mut state = seed | 1;
    let mut cells: Vec<u32> = (0..grid.cells())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            GROUND + state % HAZE
        })
        .collect();
    for (down, across) in [(40, 60), (150, 90), (80, 170), (190, 180)] {
        for row in down..down + BLOB {
            for column in across..across + BLOB {
                if let Some(one) = cells.get_mut((row * grid.wide + column) as usize) {
                    *one = MASS;
                }
            }
        }
    }
    cells
}

fn shaded(state: &[u32]) -> Vec<u32> {
    state
        .iter()
        .map(|one| {
            let lit = (one >> 6).min(255);
            (lit << 16) | (lit << 8) | lit
        })
        .collect()
}

fn lit(slope: &[u32]) -> Vec<u32> {
    let peak = slope
        .iter()
        .map(|one| (*one as i32).unsigned_abs())
        .max()
        .unwrap_or(1)
        .max(1);
    slope
        .iter()
        .map(|one| {
            let held = *one as i32;
            let much = (held.unsigned_abs().saturating_mul(255) / peak).min(255);
            if held > 0 {
                (much << 16) | (much << 3)
            } else if held < 0 {
                (much << 3) | much
            } else {
                0
            }
        })
        .collect()
}

fn reach(grid: Grid, slope: &[u32], at: usize) -> (usize, u32) {
    let (row, column) = (
        (at as u32 / grid.wide) as i64,
        (at as u32 % grid.wide) as i64,
    );
    let mut heard = 0_usize;
    let mut far = 0_i64;
    for (other, one) in slope.iter().enumerate() {
        if *one == 0 {
            continue;
        }
        heard += 1;
        let down = (other as u32 / grid.wide) as i64;
        let across = (other as u32 % grid.wide) as i64;
        far = far.max((down - row).abs().max((across - column).abs()));
    }
    (heard, far as u32)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("SKIPPED back");
        return Ok(());
    };
    let grid = Grid::of(SIDE, SIDE);
    let asking: Vec<String> = std::env::args().collect();
    let still = asking.iter().any(|one| one == "--still");
    // The window takes the depth from the arrow keys, which is no use to
    // anything driving it from outside. Named here it is the same run twice.
    let mut depth = asking
        .iter()
        .position(|one| one == "--depth")
        .and_then(|at| asking.get(at + 1))
        .and_then(|one| one.parse::<u32>().ok())
        .filter(|one| (1..=256).contains(one))
        .unwrap_or(DEPTH);
    let mut board = seated(&host, grid, Kind::Living, (KNOB, 0), depth)?;
    let mut state = blotted(grid, 0x1234_5678);
    let middle = (grid.deep / 2 * grid.wide + grid.wide / 2) as usize;

    if still {
        println!(
            "{:>7} {:>9} {:>7} {:>12}",
            "depth", "heard", "reach", "took"
        );
        for each in [1_u32, 2, 4, 8, 16, 32, 64, 128, 256] {
            board.depth = each;
            let ticking = Instant::now();
            let slope = board.slope(&state, middle)?;
            let (heard, far) = reach(grid, &slope, middle);
            println!("{each:>7} {heard:>9} {far:>7} {:>12?}", ticking.elapsed());
        }
        println!(
            "the buffers hold {} words at every one of those depths",
            grid.cells() * 5 + 8
        );

        board.depth = 16;
        let slope = board.slope(&state, middle)?;
        let settled = board.run(&state)?;
        let was = settled.get(middle).copied().unwrap_or(0);
        let nudge = 1_u32 << 13;
        let named = |one: usize| (one as u32 / grid.wide, one as u32 % grid.wide);
        let loudest = slope
            .iter()
            .enumerate()
            .max_by_key(|(_, one)| (**one as i32).unsigned_abs())
            .map(|(at, _)| at);
        let quiet = slope
            .iter()
            .enumerate()
            .filter(|(at, one)| **one == 0 && *at > middle)
            .map(|(at, _)| at)
            .next();
        println!(
            "{:>10} {:>9} {:>10} {:>12} {:>10}",
            "cell", "distance", "slope", "it promised", "it moved"
        );
        for (what, held) in [("inside", loudest), ("outside", quiet)] {
            let Some(at) = held else {
                continue;
            };
            let mut nudged = state.clone();
            if let Some(one) = nudged.get_mut(at) {
                *one = one.wrapping_add(nudge);
            }
            let after = board.run(&nudged)?;
            let now = after.get(middle).copied().unwrap_or(0);
            let told = slope.get(at).map_or(0, |one| *one as i32);
            let promised = f64::from(told) * f64::from(nudge) / f64::from(1_u32 << POINT);
            let moved = f64::from(now as i32) - f64::from(was as i32);
            let (down, across) = named(at);
            let (row, column) = named(middle);
            let far = (i64::from(down) - i64::from(row))
                .abs()
                .max((i64::from(across) - i64::from(column)).abs());
            println!("{what:>10} {far:>9} {told:>10} {promised:>12.1} {moved:>10.0}");
        }
        println!("a nudge of {nudge} at depth 16, where the cell reads {was} untouched");
        return Ok(());
    }

    let mut window = minifb::Window::new(
        "grit - click a cell to see what made it",
        SIDE as usize,
        SIDE as usize,
        minifb::WindowOptions {
            scale: minifb::Scale::X2,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: true,
            ..minifb::WindowOptions::default()
        },
    )?;
    window.set_target_fps(30);

    let mut showing: Option<Vec<u32>> = None;
    let mut asked: Option<u32> = None;
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        if window.is_key_pressed(minifb::Key::Space, minifb::KeyRepeat::Yes) {
            board.depth = 1;
            state = board.run(&state)?;
            board.depth = depth;
            showing = None;
            asked = None;
        }
        if window.is_key_pressed(minifb::Key::Up, minifb::KeyRepeat::No) {
            depth = (depth * 2).min(256);
            board.depth = depth;
            showing = None;
            asked = None;
        }
        if window.is_key_pressed(minifb::Key::Down, minifb::KeyRepeat::No) {
            depth = (depth / 2).max(1);
            board.depth = depth;
            showing = None;
            asked = None;
        }
        if window.get_mouse_down(minifb::MouseButton::Left)
            && let Some((x, y)) = window.get_mouse_pos(minifb::MouseMode::Discard)
        {
            let at = (y as u32).min(grid.deep - 1) * grid.wide + (x as u32).min(grid.wide - 1);
            if asked != Some(at) {
                asked = Some(at);
                let ticking = Instant::now();
                let slope = board.slope(&state, at as usize)?;
                let (heard, far) = reach(grid, &slope, at as usize);
                println!(
                    "depth {depth}: {heard} cells reach it, {far} away, in {:?}",
                    ticking.elapsed()
                );
                showing = Some(lit(&slope));
            }
        }
        let frame = match &showing {
            Some(held) => held.clone(),
            None => shaded(&state),
        };
        window.update_with_buffer(&frame, SIDE as usize, SIDE as usize)?;
    }
    Ok(())
}
