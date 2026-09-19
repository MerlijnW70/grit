use std::time::{Duration, Instant};

use grit::host::Error;
use grit::{Groups, Host, Resident};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORKGROUP: u32 = 64;

const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

const FRAMES: u32 = 48;

const ROUNDS: u32 = 5;

const WARMTH: Duration = Duration::from_millis(400);

const VIEW: usize = 1 << 19;

fn blind(held: &mut Resident<'_>, groups: &[Groups]) -> Result<Duration, Error> {
    let mut spent = Duration::ZERO;
    for _ in 0..FRAMES {
        spent += held.frame(groups)?;
    }
    Ok(spent / FRAMES)
}

fn ahead(held: &mut Resident<'_>, stages: &[&[Groups]]) -> Result<Duration, Error> {
    let began = Instant::now();
    let mut flying = held.submit(stages)?;
    for _ in 0..FRAMES {
        let next = held.submit(stages)?;
        held.wait(flying)?;
        flying = next;
    }
    held.wait(flying)?;
    Ok(began.elapsed() / (FRAMES + 1))
}

fn reading(held: &mut Resident<'_>, groups: &[Groups], count: usize) -> Result<Duration, Error> {
    let mut spent = Duration::ZERO;
    for _ in 0..FRAMES {
        let (took, _) = held.frame_reading(groups, 2, count)?;
        spent += took;
    }
    Ok(spent / FRAMES)
}

fn reading_ahead(
    held: &mut Resident<'_>,
    stages: &[&[Groups]],
    count: usize,
) -> Result<Duration, Error> {
    let began = Instant::now();
    let mut waiting = held.submit_reading(stages, 2, count)?;
    for _ in 0..FRAMES {
        let next = held.submit_reading(stages, 2, count)?;
        let _ = held.wait_reading(waiting)?;
        waiting = next;
    }
    let _ = held.wait_reading(waiting)?;
    Ok(began.elapsed() / (FRAMES + 1))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        return Ok(());
    };
    let width = host.width();
    println!("{} {width}", host.name());

    for power in [18_u32, 20, 22, 24] {
        let cells = 1_usize << power;
        let passes = [tick(width)?, step(width)?, draw(width)?];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

        let mut resident = host.resident(&[1, cells, cells], &modules)?;
        resident.write(0, &[0])?;
        resident.write(1, &vec![0_u32; cells])?;

        let over = u32::try_from(cells.div_ceil(WORKGROUP as usize)).unwrap_or(u32::MAX);
        let groups = [Groups::Fixed(1), Groups::Fixed(over), Groups::Fixed(over)];
        let stages: Vec<&[Groups]> = groups.iter().map(core::slice::from_ref).collect();
        let view = VIEW.min(cells);

        let _ = resident.frame_reading(&groups, 2, cells)?;
        let warming = Instant::now();
        while warming.elapsed() < WARMTH {
            resident.frames(&groups, FRAMES)?;
        }

        let mut best = [Duration::MAX; 5];
        let mut carded = Duration::MAX;
        for _ in 0..ROUNDS {
            let first = blind(&mut resident, &groups)?;
            carded = carded.min(resident.on_device().unwrap_or(Duration::MAX));
            let round = [
                first,
                ahead(&mut resident, &stages)?,
                reading(&mut resident, &groups, view)?,
                reading_ahead(&mut resident, &stages, view)?,
                reading(&mut resident, &groups, cells)?,
            ];
            for (slot, spent) in best.iter_mut().zip(round) {
                *slot = (*slot).min(spent);
            }
        }

        let [blind, ahead, shown, shown_ahead, whole] = best;
        println!(
            "{cells:>12} {blind:>12.3?} {ahead:>12.3?} {carded:>12.3?} {shown:>12.3?} {shown_ahead:>12.3?} {whole:>12.3?} {:>8.0} {:>8.0} {:>7.1}",
            1.0 / shown.as_secs_f64(),
            1.0 / shown_ahead.as_secs_f64(),
            (cells * size_of::<u32>()) as f64 / whole.as_secs_f64() / 1e9
        );
    }

    Ok(())
}

fn tick(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, WORKGROUP, 3))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();

    let now = kernel.load_at(0, slot)?;
    let next = kernel.module().i_add(element, now, one)?;
    kernel.store_at(0, slot, next)?;
    kernel.finish()
}

fn step(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, WORKGROUP, 3))?;
    let slot = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(0, slot)?;

    let stirred = {
        let mut lanes = kernel.lanes()?;
        let at = lanes.position::<WORKGROUP>()?;
        let now = lanes.splat_id::<U32, WORKGROUP>(frame)?;

        let golden = lanes.splat_bits::<U32, WORKGROUP>(GOLDEN)?;
        let stir = lanes.splat_bits::<U32, WORKGROUP>(STIR)?;
        let by16 = lanes.splat_bits::<U32, WORKGROUP>(16)?;

        let drifting = lanes.mul(now, golden)?;
        let seeded = lanes.add(at, drifting)?;
        let h = lanes.mul(seeded, stir)?;
        let folded = lanes.shift_right_logical(h, by16)?;
        lanes.add(h, folded)?
    };

    kernel.store::<WORKGROUP>(1, stirred)?;
    kernel.finish()
}

fn draw(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, WORKGROUP, 3))?;
    let state = kernel.load::<WORKGROUP>(1)?;

    let pixel = {
        let mut lanes = kernel.lanes()?;
        let by24 = lanes.splat_bits::<U32, WORKGROUP>(24)?;
        let by8 = lanes.splat_bits::<U32, WORKGROUP>(8)?;
        let mask = lanes.splat_bits::<U32, WORKGROUP>(0x0001_0101)?;

        let shade = lanes.shift_right_logical(state, by24)?;
        let spread = lanes.mul(shade, mask)?;
        lanes.shift_left(spread, by8)?
    };

    kernel.store::<WORKGROUP>(2, pixel)?;
    kernel.finish()
}
