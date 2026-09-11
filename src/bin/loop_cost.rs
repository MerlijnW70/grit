use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const WORKGROUP: u32 = 64;

const GOLDEN: u32 = 0x9E37_79B1;
const STIR: u32 = 0x85EB_CA6B;

const FRAMES: u32 = 240;

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
            "{cells:>12} {looking:>12.3?} {blind:>12.3?} {:>10.3?} {:>10.0}",
            looking.saturating_sub(blind),
            1.0 / looking.as_secs_f64()
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
