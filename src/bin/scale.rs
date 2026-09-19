use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BRUSH, GRAND, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const FRAMES: u32 = 300;

const BATCH: u32 = 20;

const POURING: u32 = 200;

const ROUNDS: u32 = 5;

const WARMTH: Duration = Duration::from_millis(400);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        return Ok(());
    };
    let width = host.width();
    println!("{} {width}", host.name());

    for power in [9_u32, 10, 11, 12] {
        let grid = Grid::of(1 << power, 1 << power);
        match once(&host, width, grid) {
            Ok(Some(())) => {}
            Ok(None) => {}
            Err(why) => println!("{:>10} {why}", grid.cells()),
        }
    }

    Ok(())
}

fn once(host: &Host, width: u32, grid: Grid) -> Result<Option<()>, Box<dyn std::error::Error>> {
    let quads = grid.quads();
    let plan = match Plan::over(width, quads.div_ceil(sweep::cells(width))) {
        Ok(plan) => plan,
        Err(too_big) => {
            println!("{:>10} {quads:>9} {too_big}", grid.cells());
            return Ok(None);
        }
    };

    let (passes, stages) = grit::live::wired(width, grid, &plan, None)?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let words: usize = sizes.iter().sum();

    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held])?;
    }
    let warming = std::time::Instant::now();
    while warming.elapsed() < WARMTH {
        resident.staged_frames(&groups, BATCH)?;
    }

    let mut spent = Duration::MAX;
    let mut card = None;
    let mut listed_total = 0_u64;
    for round in 0..ROUNDS {
        resident.write(STATE as usize, &room(grid))?;
        resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
        grit::live::seed(&mut resident, grid, &plan)?;
        resident.staged(&groups)?;

        let mut walled = Duration::ZERO;
        let mut counted = 0_u64;
        for frame in (0..FRAMES).step_by(BATCH as usize) {
            let material = if frame < POURING { SAND } else { AIR };
            let (x, y) = (grid.wide / 2, grid.deep / 8);
            resident.write_at(PARAMS as usize, BRUSH, &[x, y, 49, material])?;
            walled += resident.staged_frames(&groups, BATCH)?;

            if frame % (BATCH * 2) == 0 {
                let listed = resident
                    .read(GRAND as usize, 1)?
                    .first()
                    .copied()
                    .unwrap_or(0);
                counted += u64::from(listed);
            }
        }

        let held = resident.on_device();
        let better = match (held, card) {
            (Some(now), Some(best)) => now < best,
            (Some(_), None) => true,
            _ => walled < spent,
        };
        if better || round == 0 {
            card = held.or(card);
            listed_total = counted;
        }
        if let (Some(now), Some(best)) = (held, card) {
            card = Some(now.min(best));
        }
        spent = spent.min(walled);
    }
    let card = card;

    let asked = u64::from(FRAMES.div_ceil(BATCH * 2));
    let listed = listed_total / asked.max(1);
    let share = f64::from(quads) / (listed.max(1) as f64);
    println!(
        "{:>10} {quads:>9} {:>7} {:>10.3?} {:>10.3?} {listed:>8} {:>5} {share:>6.0}",
        grid.cells(),
        plan.blocks,
        spent / FRAMES.div_ceil(BATCH),
        card.unwrap_or_default(),
        words * 4 / (1 << 20)
    );
    Ok(Some(()))
}

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
