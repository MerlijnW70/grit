use std::time::{Duration, Instant};

use grit::{Groups, Host};
use minifb::{Key, Scale, ScaleMode, Window, WindowOptions};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORKGROUP: u32 = 64;

const WIDE: usize = 960;
const DEEP: usize = 540;
const CELLS: usize = WIDE * DEEP;

const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        return Ok(());
    };
    let width = host.width();
    println!("{} {width}", host.name());

    let built = Instant::now();
    let passes = [tick(width)?, step(width)?, draw(width)?];
    let words: usize = passes.iter().map(Vec::len).sum();
    println!("{words} {} {:?}", passes.len(), built.elapsed());

    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut resident = host.resident(&[1, CELLS, CELLS], &modules)?;

    resident.write(0, &[0])?;
    resident.write(1, &vec![0_u32; CELLS])?;

    let over = u32::try_from(CELLS.div_ceil(WORKGROUP as usize)).unwrap_or(u32::MAX);
    let groups = [Groups::Fixed(1), Groups::Fixed(over), Groups::Fixed(over)];

    let mut window = Window::new(
        "grit",
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
    println!("{:?}", warm.elapsed());

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
                "{:.0} {:.2} {CELLS} {}",
                f64::from(frames) / seconds,
                on_device.as_secs_f64() * 1000.0 / f64::from(frames.max(1)),
                host.name()
            ));
            clock = Instant::now();
            frames = 0;
            on_device = Duration::ZERO;
        }
    }

    println!("{:?}", wall.elapsed());
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
