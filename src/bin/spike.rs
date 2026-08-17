//! `cargo run --release --bin spike` — the one question everything else depends on.
//!
//! **Can several `simdr` modules run over buffers that never leave the device, at frame rates?**
//!
//! Not "is it fast enough" — `simdr`'s own `runner/examples/bench.rs` and `sweep.rs` already
//! measured the arithmetic. The risk is structural: `runner` has no primitive that keeps state
//! resident across *several* kernels, `decisions/DR-0008` explains why it has no reason to, and a
//! sparse-grid simulation is nothing but that primitive repeated. If the loop below does not exist,
//! the whole design changes — so it is built before anything that would depend on it.
//!
//! # What it deliberately is not
//!
//! Three passes that do almost nothing, over a field of numbers that means almost nothing. The
//! content is a placeholder; the *shape* is the finding:
//!
//! ```text
//!   binding 0   a single word — the frame number, incremented on the device
//!   binding 1   the state, one word per cell
//!   binding 2   the picture, one word per pixel
//! ```
//!
//! **Nothing is written from the host after the first frame.** Not even the frame counter: pass one
//! increments it in place, so the only traffic on the bus is the picture coming back. That is the
//! property the simulation needs, and it is easier to check now than to retrofit.

use std::time::{Duration, Instant};

use grit::{Groups, Host};
use minifb::{Key, Scale, ScaleMode, Window, WindowOptions};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// Invocations per workgroup.
const WORKGROUP: u32 = 64;

/// The field, which is also the picture.
const WIDE: usize = 960;
const DEEP: usize = 540;
const CELLS: usize = WIDE * DEEP;

/// The mixer's multipliers — `simdr` has no exclusive-or, so this stirs with multiply and shift.
const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}", host.name());

    let built = Instant::now();
    let passes = [tick(width)?, step(width)?, draw(width)?];
    let words: usize = passes.iter().map(Vec::len).sum();
    println!(
        "{words} words of SPIR-V across {} passes in {:?}",
        passes.len(),
        built.elapsed()
    );

    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut resident = host.resident(&[1, CELLS, CELLS], &modules)?;

    // The only thing the host ever writes. After this the bus carries pictures and nothing else.
    resident.write(0, &[0])?;
    resident.write(1, &vec![0_u32; CELLS])?;

    // One workgroup for the tick, the whole field for the other two.
    let over = u32::try_from(CELLS.div_ceil(WORKGROUP as usize)).unwrap_or(u32::MAX);
    let groups = [Groups::Fixed(1), Groups::Fixed(over), Groups::Fixed(over)];

    let mut window = Window::new(
        "grit — three passes, nothing uploaded",
        WIDE,
        DEEP,
        WindowOptions {
            scale: Scale::X1,
            scale_mode: ScaleMode::AspectRatioStretch,
            resize: true,
            ..WindowOptions::default()
        },
    )?;
    window.set_target_fps(0);

    let warm = Instant::now();
    let _ = resident.frame_reading(&groups, 2, CELLS)?;
    println!("first frame, driver compile included: {:?}", warm.elapsed());

    let mut frames = 0_u32;
    let mut on_device = Duration::ZERO;
    let mut clock = Instant::now();
    let wall = Instant::now();

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let (spent, picture) = resident.frame_reading(&groups, 2, CELLS)?;
        on_device += spent;
        window.update_with_buffer(&picture, WIDE, DEEP)?;

        frames += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            let seconds = clock.elapsed().as_secs_f64();
            window.set_title(&format!(
                "grit — {:.0} fps · {:.2} ms in the submission · {CELLS} cells · {}",
                f64::from(frames) / seconds,
                on_device.as_secs_f64() * 1000.0 / f64::from(frames.max(1)),
                host.name(),
            ));
            clock = Instant::now();
            frames = 0;
            on_device = Duration::ZERO;
        }
    }

    println!("flew for {:?} without uploading a word", wall.elapsed());
    Ok(())
}

/// Pass one: the frame number, incremented on the device.
///
/// **The whole point of the spike in four instructions.** A host that wrote this each frame would
/// pay a submit and a fence for one word — about a hundred microseconds by `decisions/DR-0008`'s
/// measurement, for four bytes. The device already has the number; it can add to it.
///
/// Every invocation of the one workgroup writes the same value to the same slot, which is the case
/// `Kernel::store_at` documents as safe: identical writes to one address need no ordering.
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
///
/// One element per invocation, and the coordinate is the invocation — no buffer of positions is
/// uploaded, which is the same trick the deleted `demo` rested on.
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

        // The lane's own place, moved along by the frame, mixed. Multiply, shift and add, because
        // `src/module/op.rs` declares no exclusive-or.
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
///
/// A separate module rather than a tail on pass two, deliberately: what is being proved is that a
/// *chain* of kernels runs over the same buffers with a barrier between, and a chain of one proves
/// nothing.
fn draw(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, WORKGROUP, 3))?;
    let state = kernel.load::<WORKGROUP>(1)?;

    let pixel = {
        let mut lanes = kernel.lanes()?;
        let by24 = lanes.splat_bits::<U32, WORKGROUP>(24)?;
        let by8 = lanes.splat_bits::<U32, WORKGROUP>(8)?;
        let mask = lanes.splat_bits::<U32, WORKGROUP>(0x0001_0101)?;

        // One byte of the state, in all three channels: grey, and enough to see that it moves.
        let shade = lanes.shift_right_logical(state, by24)?;
        let spread = lanes.mul(shade, mask)?;
        lanes.shift_left(spread, by8)?
    };

    kernel.store::<WORKGROUP>(2, pixel)?;
    kernel.finish()
}
