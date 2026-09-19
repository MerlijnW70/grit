use std::time::{Duration, Instant};

use dissolve::{World, asked, wanted};
use grit::Host;
use grit::sand::Grid;

const WIDE: u32 = 1024;
const DEEP: u32 = 1024;

const PROVING: i64 = 256;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no device");
        return Ok(());
    };
    let grid = Grid::of(WIDE, DEEP);
    let shift = asked();
    let mut world = World::new(&host, grid, shift)?;
    let start = wanted(grid)?;
    world.plant(&start)?;

    if std::env::args().any(|one| one == "check") {
        return proving(&mut world, &start, shift);
    }

    let quads = world.quads();
    let mut window = minifb::Window::new(
        "dissolve",
        WIDE as usize,
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
    let mut spent = Duration::ZERO;
    let mut frames = 0_u32;
    let mut awake = 0_u32;
    let mut clock = Instant::now();
    let mut was = [false; 4];

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let keys = [
            window.is_key_down(minifb::Key::Space),
            window.is_key_down(minifb::Key::Left),
            window.is_key_down(minifb::Key::Right),
            window.is_key_down(minifb::Key::R),
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
            world.plant(&start)?;
            step = 0;
            going = 0;
        }
        was = keys;

        if going != 0 {
            let at = if going > 0 { step } else { step - 1 };
            spent += world.step(at, going)?;
            frames += 1;
            step += going;
        }

        let shown = world.cells()?;
        window.update_with_buffer(&shown, WIDE as usize, DEEP as usize)?;

        if clock.elapsed().as_secs_f64() >= 0.25 {
            awake = world.awake().unwrap_or(awake);
            let each = spent.as_secs_f64() * 1e6 / f64::from(frames.max(1));
            let share = f64::from(awake) * 100.0 / f64::from(quads);
            let way = match going {
                0 => "held",
                n if n > 0 => "onward",
                _ => "back",
            };
            window.set_title(&format!(
                "dissolve  step {step}  {way}  shift {shift}  awake {awake} of {quads} ({share:.1}%)  {each:.0}us"
            ));
            spent = Duration::ZERO;
            frames = 0;
            clock = Instant::now();
        }
    }

    Ok(())
}

fn proving(
    world: &mut World<'_>,
    start: &[u32],
    shift: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let grid = world.grid();
    let quads = world.quads();
    let watched = [2_i64, 4, 16, 64, 256];

    let mut going_out = Vec::new();
    for at in 0..PROVING {
        world.step(at, 1)?;
        if watched.contains(&(at + 1)) {
            going_out.push((at + 1, world.awake()?));
        }
    }
    let stirred = world.cells()?;

    let mut coming_home = vec![(PROVING, world.awake()?)];
    for at in (0..PROVING).rev() {
        world.step(at, -1)?;
        if watched.contains(&at) {
            coming_home.push((at, world.awake()?));
        }
    }
    let home = world.cells()?;

    if let Some(into) = std::env::args().skip_while(|one| one != "dump").nth(1) {
        for (name, held) in [("start", start), ("stirred", &stirred), ("home", &home)] {
            dumped(&format!("{into}/{name}.raw"), held)?;
        }
    }

    let moved = start.iter().zip(&stirred).filter(|(a, b)| a != b).count();
    let apart = start.iter().zip(&home).filter(|(a, b)| a != b).count();

    println!(
        "{} cells, {quads} quads, {PROVING} steps, shift {shift}",
        grid.cells()
    );
    println!("stirred {moved} cells, home again differing {apart}");
    println!("the list each frame leaves behind, in quads of {quads}");
    println!(
        "{:>6} {:>12} {:>14}",
        "step", "on the way", "on the way back"
    );
    for (at, out) in &going_out {
        match coming_home.iter().find(|(when, _)| when == at) {
            Some((_, back)) => println!("{at:>6} {out:>12} {back:>14}"),
            None => println!("{at:>6} {out:>12} {:>14}", "-"),
        }
    }
    println!("the first step is left out: seeding wakes the whole grid once");
    Ok(())
}

fn dumped(path: &str, held: &[u32]) -> Result<(), std::io::Error> {
    let mut bytes = Vec::with_capacity(held.len() * 3);
    for cell in held {
        bytes.push(u8::try_from((cell >> 16) & 0xFF).unwrap_or(0));
        bytes.push(u8::try_from((cell >> 8) & 0xFF).unwrap_or(0));
        bytes.push(u8::try_from(cell & 0xFF).unwrap_or(0));
    }
    std::fs::write(path, bytes)
}
