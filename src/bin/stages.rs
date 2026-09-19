use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BRUSH, PARAMS, STATE};
use grit::sand::{AIR, Grid, SAND, WALL};
use grit::{Groups, Host, sweep};

const SETTLING: u32 = 300;

const ROUNDS: u32 = 5;

const WARMTH: Duration = Duration::from_millis(400);

const WIDEST: usize = "scan fine totals".len();

fn named(coarse: bool, tiles: u32) -> Vec<&'static str> {
    let mut names = vec!["tick decay wipe", "brush", "settle"];
    if coarse {
        names.extend(["mark coarse", "scan coarse", "gather coarse"]);
    }
    names.push("mark fine");
    names.push("scan fine");
    if tiles > 1 {
        names.push("scan fine totals");
    }
    names.push("gather fine");
    names
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        return Ok(());
    };
    let width = host.width();
    println!("{} {width}", host.name());

    let widest = WIDEST;
    for power in [9_u32, 12] {
        let grid = Grid::of(1 << power, 1 << power);
        for coarse in [false, true] {
            let (tiles, spans) = once(&host, width, grid, coarse)?;
            let names = named(coarse, tiles);
            let arrangement = if coarse { "two tiers" } else { "one tier" };
            let chosen = if coarse == grit::live::coarse_pays(blocks(width, grid)?) {
                " (the one the clock names)"
            } else {
                ""
            };
            println!();
            println!("{} cells, {arrangement}{chosen}", grid.cells());
            if spans.len() != names.len() {
                println!("  {} stages timed, {} named", spans.len(), names.len());
                continue;
            }
            for (name, span) in names.iter().zip(&spans) {
                println!("  {name:widest$} {span:>12.3?}");
            }
            println!(
                "  {:widest$} {:>12.3?}",
                "whole frame",
                spans.iter().sum::<Duration>()
            );
        }
    }

    Ok(())
}

fn blocks(width: u32, grid: Grid) -> Result<u32, Box<dyn std::error::Error>> {
    let quads = grid.quads();
    Ok(Plan::over(width, quads.div_ceil(sweep::cells(width)))?.blocks)
}

fn once(
    host: &Host,
    width: u32,
    grid: Grid,
    coarse: bool,
) -> Result<(u32, Vec<Duration>), Box<dyn std::error::Error>> {
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;

    let (passes, stages) = grit::live::arranged(width, grid, &plan, None, coarse)?;
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();

    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, held) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *held])?;
    }

    let mut best: Vec<Duration> = Vec::new();
    for round in 0..=ROUNDS {
        resident.write(STATE as usize, &room(grid))?;
        resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
        grit::live::seed(&mut resident, grid, &plan)?;
        resident.staged(&groups)?;

        let (x, y) = (grid.wide / 2, grid.deep / 8);
        resident.write_at(PARAMS as usize, BRUSH, &[x, y, 49, SAND])?;
        resident.staged_frames(&groups, SETTLING)?;
        resident.write_at(PARAMS as usize, BRUSH, &[x, y, 49, AIR])?;

        if round == 0 {
            let warming = Instant::now();
            while warming.elapsed() < WARMTH {
                resident.staged_frames(&groups, 20)?;
            }
            continue;
        }

        resident.staged_frames(&groups, 1)?;
        let spans = resident.stage_times().to_vec();
        if best.is_empty() {
            best = spans;
        } else {
            for (slot, now) in best.iter_mut().zip(spans) {
                *slot = (*slot).min(now);
            }
        }
    }

    Ok((plan.tiles, best))
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
