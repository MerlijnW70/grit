use std::time::Instant;

use grit::marking::{
    BEST, BINDINGS, HELD, SCORE, SEED, TRIAL, VALUES, WANTED, chosen, marked, turned_on,
};
use grit::netlist::{Chip, Exam, Gate, Netlist, drawn};
use grit::{Groups, Host};

const SIDE: u32 = 256;

const CROWD: u32 = SIDE * SIDE;

const GATES: u32 = 12;

const KINDS: u32 = 16;

const EACH: u32 = 1;

fn shape() -> Result<Netlist, Box<dyn std::error::Error>> {
    Ok(Netlist::over(5, 3, GATES, KINDS)?)
}

fn adding(shape: Netlist) -> Result<Exam, Box<dyn std::error::Error>> {
    let answers: Vec<u32> = (0..shape.words())
        .map(|row| {
            let left = row & 3;
            let right = (row >> 2) & 3;
            let carry = (row >> 4) & 1;
            left + right + carry
        })
        .collect();
    Ok(Exam::of(shape, answers)?)
}

fn columns(exam: &Exam) -> Vec<u32> {
    let shape = exam.shape();
    (0..shape.outputs)
        .map(|output| {
            (0..shape.words()).fold(0_u32, |held, row| {
                held | (((exam.asked(row) >> output) & 1) << row)
            })
        })
        .collect()
}

fn crowd(shape: Netlist, seed: u32) -> Vec<u32> {
    let mut state = seed | 1;
    (0..CROWD)
        .flat_map(|_| {
            (0..shape.gates)
                .map(|at| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    drawn(&shape, at, state).pack()
                })
                .collect::<Vec<u32>>()
        })
        .collect()
}

fn room(shape: Netlist) -> [usize; BINDINGS as usize] {
    [
        (CROWD * shape.gates) as usize,
        (CROWD * shape.gates) as usize,
        (CROWD * shape.sources()) as usize,
        shape.outputs as usize,
        CROWD as usize,
        CROWD as usize,
        CROWD as usize,
    ]
}

fn named(shape: Netlist) -> Vec<u32> {
    (shape.gates - shape.outputs..shape.gates).collect()
}

fn shaded(best: &[u32], full: u32) -> Vec<u32> {
    best.iter()
        .map(|one| {
            if *one >= full {
                0x00ff_ffff
            } else {
                let far = full.saturating_sub(*one);
                let lit = 255_u32.saturating_sub((far * 255 / full.max(1)).min(255));
                let dim = lit / 3;
                (dim << 16) | (lit << 8) | dim
            }
        })
        .collect()
}

fn spelled(chip: &Chip, exam: &Exam) -> String {
    let shape = chip.shape();
    let table = chip.table();
    (0..shape.words())
        .map(|row| {
            let got = table.get(row as usize).copied().unwrap_or(0);
            if got == exam.asked(row) { '.' } else { 'x' }
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("SKIPPED climb");
        return Ok(());
    };
    let width = host.width();
    let shape = shape()?;
    let exam = adding(shape)?;
    let full = exam.marks();
    let still = std::env::args().any(|one| one == "--still");

    let turn = turned_on(width, shape)?;
    let mark = marked(width, shape)?;
    let pick = chosen(width, shape, SIDE, CROWD)?;
    let modules: Vec<&[u32]> = vec![&turn, &mark, &pick];
    let sizes = room(shape);
    let mut resident = host.resident(&sizes, &modules)?;

    resident.write(HELD as usize, &crowd(shape, 0x1234_5678))?;
    for binding in [TRIAL, VALUES, SCORE, BEST] {
        let held = sizes.get(binding as usize).copied().unwrap_or(0);
        resident.write(binding as usize, &vec![0_u32; held])?;
    }
    resident.write(WANTED as usize, &columns(&exam))?;
    let seeds: Vec<u32> = (0..CROWD).map(|who| (who * 2_654_435_761) | 1).collect();
    resident.write(SEED as usize, &seeds)?;

    let over = Groups::Fixed(CROWD.div_ceil(width));
    let turning = [over];
    let marking = [over];
    let choosing = [over];
    let stages: Vec<&[Groups]> = vec![&turning, &marking, &choosing];

    println!("{CROWD} climbers of {GATES} gates, {full} marks for a two bit adder with a carry in",);

    let mut rounds = 0_u32;
    let telling = |best: &[u32]| {
        let top = best.iter().copied().max().unwrap_or(0);
        let solved = best.iter().filter(|one| **one >= full).count();
        let sum: u64 = best.iter().map(|one| u64::from(*one)).sum();
        (top, solved, sum as f64 / f64::from(CROWD))
    };

    if still {
        println!(
            "{:>8} {:>6} {:>9} {:>9} {:>11}",
            "rounds", "best", "average", "solved", "took"
        );
        let started = Instant::now();
        for _ in 0..40_u32 {
            resident.staged_frames(&stages, 100)?;
            rounds += 100;
            let best = resident.read(BEST as usize, CROWD as usize)?;
            let (top, solved, mean) = telling(&best);
            println!(
                "{rounds:>8} {top:>6} {mean:>9.1} {solved:>9} {:>11?}",
                started.elapsed()
            );
            if solved > 0 {
                break;
            }
        }
        let best = resident.read(BEST as usize, CROWD as usize)?;
        let held = resident.read(HELD as usize, sizes.first().copied().unwrap_or(0))?;
        if let Some(who) = best
            .iter()
            .enumerate()
            .max_by_key(|(_, one)| **one)
            .map(|(who, _)| who)
        {
            let from = who * shape.gates as usize;
            let to = from + shape.gates as usize;
            let gates = held.get(from..to).unwrap_or(&[]).to_vec();
            let chip = Chip::of(shape, gates.clone(), named(shape))?;
            println!(
                "climber {who} holds {} of {full} marks",
                exam.scored(&chip.table())
            );
            println!("its rows against the table: {}", spelled(&chip, &exam));
            for (at, packed) in gates.iter().enumerate() {
                let gate = Gate::unpack(*packed);
                println!(
                    "  gate {at:>2} kind {:>2} reads {} and {}",
                    gate.kind, gate.left, gate.right
                );
            }
        }
        return Ok(());
    }

    let mut window = minifb::Window::new(
        "grit",
        SIDE as usize,
        SIDE as usize,
        minifb::WindowOptions {
            scale: minifb::Scale::X2,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: true,
            ..minifb::WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    let started = Instant::now();
    let mut said = false;
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        resident.staged_frames(&stages, EACH)?;
        rounds += EACH;
        let best = resident.read(BEST as usize, CROWD as usize)?;
        let (top, solved, mean) = telling(&best);
        if solved > 0 && !said {
            println!(
                "solved after {rounds} rounds in {:?}, {solved} of {CROWD} hold a perfect {full}",
                started.elapsed()
            );
            said = true;
        }
        window.set_title(&format!(
            "grit - round {rounds}, best {top} of {full}, average {mean:.1}, solved {solved}"
        ));
        window.update_with_buffer(&shaded(&best, full), SIDE as usize, SIDE as usize)?;
    }
    Ok(())
}
