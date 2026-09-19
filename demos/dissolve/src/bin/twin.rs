use std::time::Instant;

use dissolve::{World, asked, wanted};
use grit::Host;
use grit::sand::Grid;

const WIDE: u32 = 512;
const DEEP: u32 = 512;

const PROVING: i64 = 192;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no device");
        return Ok(());
    };
    let grid = Grid::of(WIDE, DEEP);
    let start = wanted(grid)?;
    let shift = asked();
    let mut left = World::new(&host, grid, shift)?;
    let mut right = World::new(&host, grid, shift)?;
    left.plant(&start)?;
    right.plant(&start)?;

    let middle = (DEEP as usize / 2) * WIDE as usize + WIDE as usize / 2;

    if std::env::args().any(|one| one == "check") {
        return proving(&mut left, &mut right, &start, middle, shift);
    }

    let mut window = minifb::Window::new(
        "twin",
        WIDE as usize * 2,
        DEEP as usize,
        minifb::WindowOptions {
            scale: minifb::Scale::X1,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: true,
            ..minifb::WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    let mut step = 0_i64;
    let mut going = 0_i64;
    let mut parted: Option<i64> = None;
    let mut clock = Instant::now();
    let mut was = [false; 5];
    let mut screen = vec![0_u32; WIDE as usize * 2 * DEEP as usize];

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let keys = [
            window.is_key_down(minifb::Key::Space),
            window.is_key_down(minifb::Key::Left),
            window.is_key_down(minifb::Key::Right),
            window.is_key_down(minifb::Key::R),
            window.is_key_down(minifb::Key::F),
        ];
        if keys[0] && !was[0] {
            going = i64::from(going == 0);
        }
        if keys[1] && !was[1] {
            going = -1;
        }
        if keys[2] && !was[2] {
            going = 1;
        }
        if keys[3] && !was[3] {
            left.plant(&start)?;
            right.plant(&start)?;
            step = 0;
            going = 0;
            parted = None;
        }
        if keys[4] && !was[4] && parted.is_none() {
            right.nudge(middle, shift)?;
            parted = Some(step);
        }
        was = keys;

        if going != 0 {
            let at = if going > 0 { step } else { step - 1 };
            left.step(at, going)?;
            right.step(at, going)?;
            step += going;
        }

        let here = left.cells()?;
        let there = right.cells()?;
        for row in 0..DEEP as usize {
            let from = row * WIDE as usize;
            let into = row * WIDE as usize * 2;
            for column in 0..WIDE as usize {
                if let (Some(slot), Some(cell)) =
                    (screen.get_mut(into + column), here.get(from + column))
                {
                    *slot = *cell;
                }
                if let (Some(slot), Some(cell)) = (
                    screen.get_mut(into + WIDE as usize + column),
                    there.get(from + column),
                ) {
                    *slot = *cell;
                }
            }
        }
        window.update_with_buffer(&screen, WIDE as usize * 2, DEEP as usize)?;

        if clock.elapsed().as_secs_f64() >= 0.25 {
            let apart = here.iter().zip(&there).filter(|(a, b)| a != b).count();
            let way = match going {
                0 => "held",
                n if n > 0 => "onward",
                _ => "back",
            };
            let told = match parted {
                None => "the same world twice".to_owned(),
                Some(when) => format!("one bit apart since step {when}"),
            };
            window.set_title(&format!(
                "twin  step {step}  {way}  {told}  differing {apart} of {}",
                grid.cells()
            ));
            clock = Instant::now();
        }
    }

    Ok(())
}

fn proving(
    left: &mut World<'_>,
    right: &mut World<'_>,
    start: &[u32],
    middle: usize,
    shift: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let grid = left.grid();
    let watched = [1_i64, 2, 4, 8, 16, 32, 64, 128, 192];

    for at in 0..PROVING {
        left.step(at, 1)?;
        right.step(at, 1)?;
    }

    let was = right.nudge(middle, shift)?;
    let now = was ^ (1 << shift);
    println!(
        "{} cells, {PROVING} steps out, then one bit turned over in the right hand world",
        grid.cells()
    );
    println!("cell {middle} went from {was:#010x} to {now:#010x}");
    println!("differing right after the turn: {}", apart(left, right)?);

    println!("\n{:>6} {:>14}", "step", "differing");
    for at in PROVING..(PROVING * 2) {
        left.step(at, 1)?;
        right.step(at, 1)?;
        let since = at - PROVING + 1;
        if watched.contains(&since) {
            println!("{since:>6} {:>14}", apart(left, right)?);
        }
    }

    for at in (0..(PROVING * 2)).rev() {
        left.step(at, -1)?;
        right.step(at, -1)?;
    }

    let here = left.cells()?;
    let there = right.cells()?;
    let kept = start.iter().zip(&here).filter(|(a, b)| a != b).count();
    let lost = start.iter().zip(&there).filter(|(a, b)| a != b).count();
    if let Some(into) = std::env::args().skip_while(|one| one != "dump").nth(1) {
        for (name, held) in [
            ("twin-start", start),
            ("twin-kept", &here),
            ("twin-lost", &there),
        ] {
            let mut bytes = Vec::with_capacity(held.len() * 3);
            for cell in held {
                bytes.push(u8::try_from((cell >> 16) & 0xFF).unwrap_or(0));
                bytes.push(u8::try_from((cell >> 8) & 0xFF).unwrap_or(0));
                bytes.push(u8::try_from(cell & 0xFF).unwrap_or(0));
            }
            std::fs::write(format!("{into}/{name}.raw"), bytes)?;
        }
    }
    println!("\nboth worlds walked all the way back to where they began");
    println!("the untouched world differs from the picture by {kept} cells");
    println!("the world one bit apart differs from the picture by {lost} cells");
    Ok(())
}

fn apart(left: &mut World<'_>, right: &mut World<'_>) -> Result<usize, Box<dyn std::error::Error>> {
    let here = left.cells()?;
    let there = right.cells()?;
    Ok(here.iter().zip(&there).filter(|(a, b)| a != b).count())
}
