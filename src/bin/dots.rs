use std::time::Duration;

use grit::ops::{Cpu, Device, Dots, Ops};
use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const INVOCATIONS: usize = 1 << 20;

const UNROLL: u32 = 512;

const RUNS: usize = 9;

const WARM: usize = 3;

const STRIDE: u32 = 0x0101_0101;

const STEP: u32 = 0x9E37_79B9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sign {
    Signed,
    Unsigned,
}

fn dotted<O: Ops>(ops: &mut O, sign: Sign, left: O::Word, right: O::Word) -> O::Word {
    match sign {
        Sign::Signed => ops.dot_signed(left, right),
        Sign::Unsigned => ops.dot_unsigned(left, right),
    }
}

fn chain<O: Ops>(ops: &mut O, sign: Sign, left: O::Word, right: O::Word) -> O::Word {
    let stride = ops.constant(STRIDE);
    let step = ops.constant(STEP);
    let mut walking = left;
    let mut total = ops.constant(0);
    for _ in 0..UNROLL {
        let dot = dotted(ops, sign, walking, right);
        total = ops.add(total, dot);
        let scaled = ops.mul(walking, stride);
        walking = ops.add(scaled, step);
    }
    total
}

fn kernel<const LANES: u32>(subgroup: u32, sign: Sign, dots: Dots) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, LANES, 3))?;
    let left = kernel.load::<LANES>(0)?;
    let right = kernel.load::<LANES>(1)?;
    let total = {
        let mut lanes = kernel.lanes()?;
        let mut device = Device::<LANES>::dotting(&mut lanes, dots)?;
        let total = chain(&mut device, sign, left, right);
        device.finish(total)?
    };
    kernel.store::<LANES>(2, total)?;
    kernel.finish()
}

fn built(subgroup: u32, sign: Sign, dots: Dots) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => kernel::<4>(subgroup, sign, dots),
        8 => kernel::<8>(subgroup, sign, dots),
        16 => kernel::<16>(subgroup, sign, dots),
        32 => kernel::<32>(subgroup, sign, dots),
        64 => kernel::<64>(subgroup, sign, dots),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn median(mut spent: Vec<Duration>) -> Duration {
    spent.sort_unstable();
    spent.get(spent.len() / 2).copied().unwrap_or_default()
}

fn inputs() -> (Vec<u32>, Vec<u32>) {
    let mut state = 0x5EED_u64;
    let mut roll = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    };
    (0..INVOCATIONS)
        .map(|_| {
            let held = roll();
            ((held >> 32) as u32, held as u32)
        })
        .unzip()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        return Err("no Vulkan device".into());
    };
    let width = host.width();
    let groups = u32::try_from(INVOCATIONS)? / width;
    if groups > host.limits().groups {
        return Err(format!(
            "{groups} groups exceed the device's {}",
            host.limits().groups
        )
        .into());
    }
    let mut paths = vec![Dots::Exact];
    if host.dots() == Dots::Native {
        paths.push(Dots::Native);
    }
    println!(
        "{} width {width} offers {:?}: {} dots per dispatch, {RUNS} runs after {WARM}",
        host.name(),
        host.dots(),
        INVOCATIONS as u64 * u64::from(UNROLL)
    );
    let (left, right) = inputs();
    let dispatch = [Groups::Fixed(groups)];
    for sign in [Sign::Signed, Sign::Unsigned] {
        let want: Vec<u32> = left
            .iter()
            .zip(&right)
            .map(|(&one, &other)| chain(&mut Cpu, sign, one, other))
            .collect();
        let mut held = Vec::new();
        for &dots in &paths {
            let module = built(width, sign, dots)?;
            let mut resident = host.resident(&[INVOCATIONS; 3], &[&module])?;
            resident.write(0, &left)?;
            resident.write(1, &right)?;
            let (_, got) = resident.frame_reading(&dispatch, 2, INVOCATIONS)?;
            if got != want {
                return Err(format!("{sign:?} {dots:?}: the words differ from the CPU").into());
            }
            for _ in 0..WARM {
                resident.frame(&dispatch)?;
            }
            held.push((dots, resident, Vec::with_capacity(RUNS)));
        }
        for _ in 0..RUNS {
            for (_, resident, spent) in &mut held {
                resident.frame(&dispatch)?;
                spent.push(resident.on_device().ok_or("no device time")?);
            }
        }
        for (dots, _, spent) in held {
            let fastest = spent.iter().min().copied().unwrap_or_default();
            let middle = median(spent);
            println!(
                "{sign:?} {dots:?}: median {:.3} ms, min {:.3} ms, {:.1} G dots/s, words equal the CPU",
                middle.as_secs_f64() * 1e3,
                fastest.as_secs_f64() * 1e3,
                (INVOCATIONS as f64 * f64::from(UNROLL)) / middle.as_secs_f64() / 1e9
            );
        }
    }
    Ok(())
}
