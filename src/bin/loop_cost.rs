//! `cargo run --release --bin loop_cost` — what a resident frame costs, without a window in it.
//!
//! The spike next door opens a window, and a window is a blit and a compositor and a vsync. This
//! runs the same three passes over the same buffers with none of that, so the number is the
//! submission: three dispatches, two barriers, one copy out, one fence.
//!
//! It also asks the question the other way round. The same frame **without** the copy out is what a
//! simulation costs when nothing is looked at — and the difference between the two is the price of
//! looking, which is the only traffic this design allows.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// Invocations per workgroup.
const WORKGROUP: u32 = 64;

/// The mixer's multipliers.
const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

/// How many frames each measurement averages over.
const FRAMES: u32 = 240;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}\n", host.name());

    println!(
        "  {:>12}  {:>12}  {:>12}  {:>10}  {:>10}",
        "cells", "frame", "no readback", "looking", "fps"
    );

    for power in [18_u32, 20, 22, 24] {
        let cells = 1_usize << power;
        let passes = [tick(width)?, step(width)?, draw(width)?];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

        let mut resident = host.resident(&[1, cells, cells], &modules)?;
        resident.write(0, &[0])?;
        resident.write(1, &vec![0_u32; cells])?;

        let over = u32::try_from(cells.div_ceil(WORKGROUP as usize)).unwrap_or(u32::MAX);
        let groups = [Groups::Fixed(1), Groups::Fixed(over), Groups::Fixed(over)];

        // A discarded frame first: the first submission of a pipeline pays for its compile.
        let _ = resident.frame_reading(&groups, 2, cells)?;

        let mut looking = Duration::ZERO;
        for _ in 0..FRAMES {
            let (spent, _) = resident.frame_reading(&groups, 2, cells)?;
            looking += spent;
        }

        let mut blind = Duration::ZERO;
        for _ in 0..FRAMES {
            blind += resident.frame(&groups)?;
        }

        let looking = looking / FRAMES;
        let blind = blind / FRAMES;
        println!(
            "  {cells:>12}  {looking:>12.3?}  {blind:>12.3?}  {:>10.3?}  {:>10.0}",
            looking.saturating_sub(blind),
            1.0 / looking.as_secs_f64()
        );
    }

    println!(
        "\n  `frame` is three dispatches, two barriers and one fence. `no readback` is the same\n  \
         thing with nothing copied out — what the simulation costs when nobody is watching.\n  \
         `looking` is the difference, and it is the only traffic this design allows."
    );
    Ok(())
}

/// Pass one: the frame number, incremented on the device.
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

/// Pass two: every cell, from its own coordinate and the frame number.
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

/// Pass three: the picture, from the state.
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
