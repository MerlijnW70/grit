#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::Plan;

use grit::layout::{DENSE, GRAND, PARAMS, STATE};

use grit::live::{self, Waking, corners, wake};

use grit::sand::Grid;

use grit::{Groups, Host};

use simdr::kernel::{Kernel, Shape};

use simdr::lanes::{LaneError, U32};

pub use grit::board::*;

pub use grit::rule::*;

pub const WIDE: u32 = 64;

pub const DEEP: u32 = 64;

pub const STEPS: u32 = 8;

pub const TURNING: u32 = 8303;

pub const SWING: u32 = 16384;

pub const STEP: u32 = 1 << 9;

pub const GRAIN: u32 = 2;

pub fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

pub fn spread(grid: Grid, seed: u32) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % CEILING
        })
        .collect()
}

pub const ROUNDS: u32 = 40;

pub const CEILING: u32 = 1 << 12;

pub const PACE: u32 = 3;

pub fn descended(
    board: &mut Board<'_>,
    grid: Grid,
    pace: u32,
    rounds: u32,
) -> (f64, f64, f64, f64) {
    let truth = spread(grid, 0x1234_5678);
    let target = board.run(&truth).unwrap();
    let drift = spread(grid, 0x0BAD_C0DE);
    let mut guess: Vec<u32> = truth
        .iter()
        .zip(&drift)
        .map(|(one, other)| one.wrapping_add(other >> 2))
        .collect();

    let ended = board.run(&guess).unwrap();
    let reached = ended
        .iter()
        .map(|one| f64::from(*one as i32).abs())
        .sum::<f64>()
        / ended.len() as f64;
    let first = apart(&ended, &target);
    let mut strength = 0.0;
    let mut last = first;
    for round in 0..rounds {
        let ended = board.run(&guess).unwrap();
        let weight: Vec<u32> = ended
            .iter()
            .zip(&target)
            .map(|(one, other)| one.wrapping_sub(*other))
            .collect();
        let (slope, _) = board.carry(&guess, &weight).unwrap();
        if round == 0 {
            strength = slope
                .iter()
                .map(|one| f64::from(*one as i32).abs())
                .sum::<f64>()
                / slope.len() as f64;
        }
        for (cell, step) in guess.iter_mut().zip(&slope) {
            let moved = i64::from(*cell) - i64::from((*step as i32) >> pace);
            *cell = moved.clamp(0, i64::from(CEILING) - 1) as u32;
        }
        last = apart(&board.run(&guess).unwrap(), &target);
    }
    (first, last, strength, reached)
}

pub fn leaning(slope: &[u32], guess: &[u32], truth: &[u32]) -> f64 {
    let mut along = 0.0_f64;
    let mut held = 0.0_f64;
    let mut home = 0.0_f64;
    for ((one, near), far) in slope.iter().zip(guess).zip(truth) {
        let step = f64::from(*one as i32);
        let away = f64::from(near.wrapping_sub(*far) as i32);
        along += step * away;
        held += step * step;
        home += away * away;
    }
    if held == 0.0 || home == 0.0 {
        return 0.0;
    }
    along / (held.sqrt() * home.sqrt())
}

pub fn blended(truth: &[u32], guess: &[u32], part: f64) -> Vec<u32> {
    truth
        .iter()
        .zip(guess)
        .map(|(far, near)| {
            let away = f64::from(near.wrapping_sub(*far) as i32);
            let moved = f64::from(*far as i32) + away * part;
            moved as i32 as u32
        })
        .collect()
}

pub fn squared(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(held, want)| {
            let apart = f64::from(held.wrapping_sub(*want) as i32);
            apart * apart
        })
        .sum::<f64>()
        / 2.0
}

pub fn asking(board: &mut Board<'_>, start: &[u32], target: &[u32], angle: u32) -> f64 {
    let swing = board.angle.1;
    board.tune(angle, swing).unwrap();
    let ended = board.run(start).unwrap();
    let weight: Vec<u32> = ended
        .iter()
        .zip(target)
        .map(|(one, other)| one.wrapping_sub(*other))
        .collect();
    board.carry(start, &weight).unwrap();
    board.told().unwrap().0
}

pub fn hunted(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> u32 {
    let mut here = from;
    let mut span = 1_i64;
    let low;
    let high;
    let leaning = asking(board, start, target, here) > 0.0;
    loop {
        let next = if leaning {
            i64::from(here) - span
        } else {
            i64::from(here) + span
        };
        let held = next.clamp(1, 4096) as u32;
        let now = asking(board, start, target, held) > 0.0;
        if now != leaning || span > 512 {
            (low, high) = if leaning {
                (i64::from(held), i64::from(here))
            } else {
                (i64::from(here), i64::from(held))
            };
            break;
        }
        here = held;
        span *= 2;
    }
    let (mut low, mut high) = (low, high);
    while high - low > 1 {
        let middle = (low + high) / 2;
        if asking(board, start, target, middle.clamp(1, 4096) as u32) > 0.0 {
            high = middle;
        } else {
            low = middle;
        }
    }
    low as u32
}

pub fn narrowed(board: &mut Board<'_>, start: &[u32], target: &[u32], from: u32) -> (u32, u32) {
    let mut runs = 0_u32;
    let loss = |board: &mut Board<'_>, at: u32, runs: &mut u32| {
        board.tune(at, board.angle.1).unwrap();
        *runs += 1;
        squared(&board.run(start).unwrap(), target)
    };

    let mut low = 1_i64;
    let mut high = i64::from(from) * 2;
    while high - low > 2 {
        let third = (high - low) / 3;
        let left = low + third;
        let right = high - third;
        let here = loss(board, left as u32, &mut runs);
        let there = loss(board, right as u32, &mut runs);
        if here < there {
            high = right;
        } else {
            low = left;
        }
    }
    let mut best = low;
    let mut held = loss(board, low as u32, &mut runs);
    for at in (low + 1)..=high {
        let now = loss(board, at as u32, &mut runs);
        if now < held {
            held = now;
            best = at;
        }
    }
    (best as u32, runs)
}

pub fn strayed_where(one: &[u32], other: &[u32], live: &[bool]) -> f64 {
    let mut held = 0.0_f64;
    let mut seen = 0_usize;
    for ((a, b), on) in one.iter().zip(other).zip(live) {
        if *on {
            held += f64::from((*a as i32) - (*b as i32)).abs();
            seen += 1;
        }
    }
    held / seen.max(1) as f64
}

pub fn worst(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .fold(0.0, f64::max)
}

pub fn apart(one: &[u32], other: &[u32]) -> f64 {
    one.iter()
        .zip(other)
        .map(|(a, b)| f64::from(a.wrapping_sub(*b) as i32).abs())
        .sum::<f64>()
        / one.len() as f64
}

pub const WIDER: u32 = 256;

pub const PATCH: u32 = 12;

pub fn over_list(subgroup: u32, grid: Grid, pass: Pass) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => over_listing::<4>(subgroup, grid, pass),
        8 => over_listing::<8>(subgroup, grid, pass),
        16 => over_listing::<16>(subgroup, grid, pass),
        32 => over_listing::<32>(subgroup, grid, pass),
        64 => over_listing::<64>(subgroup, grid, pass),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

pub fn over_listing<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    pass: Pass,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, grit::layout::BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let second = kernel.module().constant_u32(1)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let grand = kernel.load_at(GRAND, head)?;
    let dump_base = kernel.load_at(PARAMS, second)?;

    let inside = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(total, here)?;
        lanes.select(listed, one, none)?
    };

    let quad = kernel.load_at(DENSE, slot)?;
    let held = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let places = held.places;

    let state = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];
    let carried = [
        kernel.load_at(SLOPE, places[0].id())?,
        kernel.load_at(SLOPE, places[1].id())?,
        kernel.load_at(SLOPE, places[2].id())?,
        kernel.load_at(SLOPE, places[3].id())?,
    ];

    let (state_next, weight_next, stirring) = {
        let mut lanes = kernel.lanes()?;
        let block = [
            lanes.splat_id::<U32, LANES>(state[0])?,
            lanes.splat_id::<U32, LANES>(state[1])?,
            lanes.splat_id::<U32, LANES>(state[2])?,
            lanes.splat_id::<U32, LANES>(state[3])?,
        ];
        let along = [
            lanes.splat_id::<U32, LANES>(carried[0])?,
            lanes.splat_id::<U32, LANES>(carried[1])?,
            lanes.splat_id::<U32, LANES>(carried[2])?,
            lanes.splat_id::<U32, LANES>(carried[3])?,
        ];
        let (state_next, weight_next) = match pass {
            Pass::Ahead => {
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                (
                    onward::<LANES>(&mut lanes, Kind::Product, block, (none, none), none)?,
                    along,
                )
            }
            Pass::Back => {
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let (state, weight, _) =
                    backward::<LANES>(&mut lanes, Kind::Product, block, along, (none, none), none)?;
                (state, weight)
            }
        };

        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut count = none;
        for (before, after) in block
            .into_iter()
            .chain(along)
            .zip(state_next.into_iter().chain(weight_next))
        {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }
        let moving = lanes.greater_than(count, none)?;
        let stirring = lanes.select(moving, one, none)?;
        (state_next, weight_next, stirring)
    };

    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let two = lanes.splat_bits::<U32, LANES>(2)?;
        let along = lanes.shift_left(mine, two)?;
        lanes.add(past, along)?
    };
    for (corner, place) in places.iter().enumerate() {
        let where_at = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let step = lanes.splat_bits::<U32, LANES>(u32::try_from(corner).unwrap_or(0))?;
            let elsewhere = lanes.add(spare, step)?;
            let listed = lanes.greater_than(inside, none)?;
            lanes.select(listed, *place, elsewhere)?
        };
        if let (Some(one), Some(other)) = (state_next.get(corner), weight_next.get(corner)) {
            kernel.store_at(STATE, where_at.id(), one.id())?;
            kernel.store_at(SLOPE, where_at.id(), other.id())?;
        }
    }

    let where_at = Waking {
        subgroup,
        grid,
        column: held.along,
        row: held.down,
    };
    wake::<LANES>(&mut kernel, where_at, stirring, inside, dump_base)?;
    kernel.finish()
}

pub struct Listed<'host> {
    pub resident: grit::Resident<'host>,
    pub ahead: Vec<Vec<Groups>>,
    pub back: Vec<Vec<Groups>>,
    pub grid: Grid,
    pub plan: Plan,
}

impl Listed<'_> {
    pub fn rouse(&mut self, cells: &[u32], weight: &[u32]) {
        self.resident.write(STATE as usize, cells).unwrap();
        self.resident.write(SLOPE as usize, weight).unwrap();
        self.resident
            .write(PARAMS as usize, &[0, self.plan.dump()])
            .unwrap();
        live::seed(&mut self.resident, self.grid, &self.plan).unwrap();
    }

    pub fn run(&mut self, cells: &[u32]) -> (Vec<u32>, u32) {
        let quiet = vec![0_u32; self.grid.cells()];
        self.rouse(cells, &quiet);
        let stages: Vec<&[Groups]> = self.ahead.iter().map(Vec::as_slice).collect();
        for frame in 0..STEPS {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        let awake = self
            .resident
            .read(GRAND as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0);
        (
            self.resident
                .read(STATE as usize, self.grid.cells())
                .unwrap(),
            awake,
        )
    }

    pub fn carry(&mut self, ended: &[u32], weight: &[u32]) -> Vec<u32> {
        self.rouse(ended, weight);
        let stages: Vec<&[Groups]> = self.back.iter().map(Vec::as_slice).collect();
        for frame in (0..STEPS).rev() {
            self.resident
                .write_at(PARAMS as usize, 0, &[frame])
                .unwrap();
            self.resident.staged(&stages).unwrap();
        }
        self.resident
            .read(SLOPE as usize, self.grid.cells())
            .unwrap()
    }
}

pub fn patched(grid: Grid, seed: u32, span: u32) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![0_u32; grid.cells()];
    let middle = (grid.wide / 2, grid.deep / 2);
    for row in middle.1..middle.1 + span {
        for column in middle.0..middle.0 + span {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            cells[(row * grid.wide + column) as usize] = state % CEILING;
        }
    }
    cells
}
