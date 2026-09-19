use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::layout::{BINDINGS, DENSE, GRAND, PARAMS, STATE};
use grit::live::{Waking, corners, wake};
use grit::sand::{self, Grid};
use grit::{Groups, Host, live, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

const WIDE: u32 = 1024;
const DEEP: u32 = 1024;

const SHIFT: u32 = 16;
const LOW: u32 = (1 << SHIFT) - 1;

const SELDOM: u32 = 8;

const PROVING: i64 = 256;

type Block<const LANES: u32> = [Vector<U32, LANES>; 4];

type Halves<const LANES: u32> = (Block<LANES>, Block<LANES>);

const KEYS: [u32; 5] = [
    0x9E37_79B1,
    0x85EB_CA6B,
    0xC2B2_AE35,
    0x27D4_EB2F,
    0x1656_67B1,
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no device");
        return Ok(());
    };
    let width = host.width();
    let grid = Grid::of(WIDE, DEEP);
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;

    let none = Groups::Fixed(0);
    let keeping = live::keeping(width, &plan, false)?;
    let rules = vec![
        (listed(width, grid, false)?, keeping.over),
        (listed(width, grid, true)?, none),
    ];
    let mut stages = keeping.before.clone();
    stages.push(rules);
    stages.extend(keeping.after.clone());
    let (passes, ahead) = live::staged(stages);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words])?;
    }

    let start = picture(grid);
    resident.write(STATE as usize, &start)?;
    resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    live::seed(&mut resident, grid, &plan)?;

    let back: Vec<Vec<Groups>> = ahead
        .iter()
        .map(|stage| {
            if stage.len() == 2 && stage.get(1) == Some(&none) {
                vec![none, keeping.over]
            } else {
                stage.clone()
            }
        })
        .collect();
    let forward: Vec<&[Groups]> = ahead.iter().map(Vec::as_slice).collect();
    let backward: Vec<&[Groups]> = back.iter().map(Vec::as_slice).collect();

    if std::env::args().any(|one| one == "check") {
        let watched = [2_i64, 4, 16, 64, 256];
        let mut going_out = Vec::new();
        for at in 0..PROVING {
            let phase = u32::try_from(at.rem_euclid(2)).unwrap_or(0);
            resident.write_at(PARAMS as usize, 0, &[phase])?;
            resident.staged(&forward)?;
            if watched.contains(&(at + 1)) {
                going_out.push((at + 1, reading(&mut resident)?));
            }
        }
        let stirred = resident.read(STATE as usize, grid.cells())?;
        let mut coming_home = vec![(PROVING, reading(&mut resident)?)];
        for at in (0..PROVING).rev() {
            let phase = u32::try_from(at.rem_euclid(2)).unwrap_or(0);
            resident.write_at(PARAMS as usize, 0, &[phase])?;
            resident.staged(&backward)?;
            if watched.contains(&at) {
                coming_home.push((at, reading(&mut resident)?));
            }
        }
        let home = resident.read(STATE as usize, grid.cells())?;
        if let Some(into) = std::env::args().skip_while(|one| one != "dump").nth(1) {
            for (name, held) in [("start", &start), ("stirred", &stirred), ("home", &home)] {
                let mut bytes = Vec::with_capacity(held.len() * 3);
                for cell in held {
                    bytes.push(u8::try_from((cell >> 16) & 0xFF).unwrap_or(0));
                    bytes.push(u8::try_from((cell >> 8) & 0xFF).unwrap_or(0));
                    bytes.push(u8::try_from(cell & 0xFF).unwrap_or(0));
                }
                std::fs::write(format!("{into}/{name}.raw"), bytes)?;
            }
        }
        let moved = start.iter().zip(&stirred).filter(|(a, b)| a != b).count();
        let apart = start.iter().zip(&home).filter(|(a, b)| a != b).count();

        println!("{} cells, {quads} quads, {PROVING} steps", grid.cells());
        println!("stirred {moved} cells, home again differing {apart}");
        println!("the list each frame leaves behind, in quads of {quads}");
        println!(
            "{:>6} {:>12} {:>14}",
            "step", "on the way", "on the way back"
        );
        for (at, out) in &going_out {
            match coming_home.iter().find(|(when, _)| when == at) {
                Some((_, back)) => println!("{at:>6} {out:>12} {back:>14}"),
                None => println!("{at:>6} {out:>12} {:>14}", "-"),
            }
        }
        println!("the first step is left out: seeding wakes the whole grid once");
        return Ok(());
    }

    let mut window = minifb::Window::new(
        "dissolve",
        WIDE as usize,
        DEEP as usize,
        minifb::WindowOptions {
            scale: minifb::Scale::X1,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            resize: true,
            ..minifb::WindowOptions::default()
        },
    )?;
    window.set_target_fps(60);

    let mut step = 0_i64;
    let mut going = 0_i64;
    let mut spent = Duration::ZERO;
    let mut frames = 0_u32;
    let mut awake = 0_u32;
    let mut clock = Instant::now();
    let mut was = [false; 4];

    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let keys = [
            window.is_key_down(minifb::Key::Space),
            window.is_key_down(minifb::Key::Left),
            window.is_key_down(minifb::Key::Right),
            window.is_key_down(minifb::Key::R),
        ];
        if keys[0] && !was[0] {
            going = i64::from(going == 0);
        }
        if keys[1] && !was[1] {
            going = -1;
        }
        if keys[2] && !was[2] {
            going = 1;
        }
        if keys[3] && !was[3] {
            resident.write(STATE as usize, &start)?;
            live::seed(&mut resident, grid, &plan)?;
            step = 0;
            going = 0;
        }
        was = keys;

        if going != 0 {
            let at = if going > 0 { step } else { step - 1 };
            let phase = u32::try_from(at.rem_euclid(2)).unwrap_or(0);
            resident.write_at(PARAMS as usize, 0, &[phase])?;
            let staged = if going > 0 { &forward } else { &backward };
            spent += resident.staged(staged)?;
            frames += 1;
            step += going;
        }

        let picture = resident.read(STATE as usize, grid.cells())?;
        window.update_with_buffer(&picture, WIDE as usize, DEEP as usize)?;

        if clock.elapsed().as_secs_f64() >= 0.25 {
            awake = resident
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(awake);
            let each = spent.as_secs_f64() * 1e6 / f64::from(frames.max(1));
            let share = f64::from(awake) * 100.0 / f64::from(quads);
            let way = match going {
                0 => "held",
                n if n > 0 => "onward",
                _ => "back",
            };
            window.set_title(&format!(
                "dissolve  step {step}  {way}  awake {awake} of {quads} ({share:.1}%)  {each:.0}us"
            ));
            spent = Duration::ZERO;
            frames = 0;
            clock = Instant::now();
        }
    }

    Ok(())
}

fn reading(resident: &mut grit::Resident<'_>) -> Result<u32, grit::host::Error> {
    Ok(resident
        .read(GRAND as usize, 1)?
        .first()
        .copied()
        .unwrap_or(0))
}

fn picture(grid: Grid) -> Vec<u32> {
    let mut cells = vec![0_u32; grid.cells()];
    let side = f64::from(grid.wide.min(grid.deep));
    let middle = (f64::from(grid.wide) / 2.0, f64::from(grid.deep) / 2.0);
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let sky = 0x0010_1428 + ((u32::from(u16::try_from(row).unwrap_or(0)) * 96) / grid.deep);
            let mut colour = 0x0010_1428 | (sky & 0xFF);
            let (x, y) = (f64::from(column) - middle.0, f64::from(row) - middle.1);
            let far = x.hypot(y);
            if far < side * 0.33 {
                colour = 0x00E6_C8A0;
            }
            for eye in [-0.13, 0.13] {
                if (x - side * eye).hypot(y + side * 0.07) < side * 0.05 {
                    colour = 0x0020_1E1E;
                }
            }
            let smile = far > side * 0.17 && far < side * 0.21 && y > side * 0.06;
            if smile {
                colour = 0x0078_2828;
            }
            if let Some(slot) = cells.get_mut((row * grid.wide + column) as usize) {
                *slot = colour;
            }
        }
    }
    cells
}

fn listed(subgroup: u32, grid: Grid, back: bool) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => some_of::<4>(subgroup, grid, back),
        8 => some_of::<8>(subgroup, grid, back),
        16 => some_of::<16>(subgroup, grid, back),
        32 => some_of::<32>(subgroup, grid, back),
        64 => some_of::<64>(subgroup, grid, back),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn stirred<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    key: u32,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(key)?;
    let other = lanes.splat_bits::<U32, LANES>(KEYS[1])?;
    let held = lanes.mul(first, by)?;
    let up = lanes.mul(second, other)?;
    lanes.xor(held, up)
}

fn shifted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 3],
) -> Result<Vector<U32, LANES>, LaneError> {
    let [one, two, three] = held;
    let a = lanes.splat_bits::<U32, LANES>(KEYS[2])?;
    let b = lanes.splat_bits::<U32, LANES>(KEYS[3])?;
    let c = lanes.splat_bits::<U32, LANES>(KEYS[4])?;
    let first = lanes.mul(one, a)?;
    let second = lanes.mul(two, b)?;
    let third = lanes.mul(three, c)?;
    let held = lanes.xor(first, second)?;
    lanes.xor(held, third)
}

fn lifted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    keys: [Vector<U32, LANES>; 4],
    back: bool,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [base, first, second, third] = keys;
    let mut one = lanes.sub(first, base)?;
    let mut two = lanes.sub(second, base)?;
    let mut three = lanes.sub(third, base)?;
    let seed;
    if back {
        let step = shifted::<LANES>(lanes, [one, two, three])?;
        seed = lanes.sub(base, step)?;
        let step = stirred::<LANES>(lanes, KEYS[0], one, two)?;
        three = lanes.sub(three, step)?;
        let step = stirred::<LANES>(lanes, KEYS[0], three, one)?;
        two = lanes.sub(two, step)?;
        let step = stirred::<LANES>(lanes, KEYS[0], two, three)?;
        one = lanes.sub(one, step)?;
    } else {
        let step = stirred::<LANES>(lanes, KEYS[0], two, three)?;
        one = lanes.add(one, step)?;
        let step = stirred::<LANES>(lanes, KEYS[0], three, one)?;
        two = lanes.add(two, step)?;
        let step = stirred::<LANES>(lanes, KEYS[0], one, two)?;
        three = lanes.add(three, step)?;
        let step = shifted::<LANES>(lanes, [one, two, three])?;
        seed = lanes.add(base, step)?;
    }
    Ok([
        seed,
        lanes.add(seed, one)?,
        lanes.add(seed, two)?,
        lanes.add(seed, three)?,
    ])
}

fn split<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
) -> Result<Halves<LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(SHIFT)?;
    let mask = lanes.splat_bits::<U32, LANES>(LOW)?;
    let mut keys = Vec::with_capacity(4);
    let mut rest = Vec::with_capacity(4);
    for one in held {
        keys.push(lanes.shift_right_logical(one, by)?);
        rest.push(lanes.and(one, mask)?);
    }
    Ok((four(&keys)?, four(&rest)?))
}

fn merged<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    keys: [Vector<U32, LANES>; 4],
    rest: [Vector<U32, LANES>; 4],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(SHIFT)?;
    let mut out = Vec::with_capacity(4);
    for (key, kept) in keys.into_iter().zip(rest) {
        let up = lanes.shift_left(key, by)?;
        out.push(lanes.or(up, kept)?);
    }
    four(&out)
}

fn four<const LANES: u32>(
    held: &[Vector<U32, LANES>],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let missing = || LaneError::NoMapping {
        lanes: LANES,
        width: LANES,
    };
    Ok([
        *held.first().ok_or_else(missing)?,
        *held.get(1).ok_or_else(missing)?,
        *held.get(2).ok_or_else(missing)?,
        *held.get(3).ok_or_else(missing)?,
    ])
}

fn level<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    keys: [Vector<U32, LANES>; 4],
) -> Result<Vector<U32, LANES>, LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let [a, b, c, d] = keys;
    let mut flat = one;
    for (first, second) in [(a, b), (b, c), (c, d)] {
        let is = lanes.equal(first, second)?;
        let held = lanes.select(is, one, none)?;
        flat = lanes.mul(flat, held)?;
    }
    Ok(flat)
}

fn swapped<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cells: [Vector<U32, LANES>; 4],
    keys: [Vector<U32, LANES>; 4],
    back: bool,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [first, second, third, fourth] = keys;
    let mut sum = first;
    for one in [second, third, fourth] {
        sum = lanes.add(sum, one)?;
    }
    let rung = sand::low_bits::<LANES>(lanes, sum, SELDOM)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let rarely = lanes.equal(rung, none)?;

    let [a, b, c, d] = cells;
    let quarter = if back { [b, d, a, c] } else { [c, a, d, b] };
    let half = [d, c, b, a];

    let mut out = Vec::with_capacity(4);
    for (turned, flipped) in quarter.into_iter().zip(half) {
        out.push(lanes.select(rarely, turned, flipped)?);
    }
    four(&out)
}

fn dissolving<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
    back: bool,
) -> Result<(Block<LANES>, Vector<U32, LANES>), LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let (keys, rest) = split::<LANES>(lanes, held)?;
    let flat = level::<LANES>(lanes, keys)?;
    let moving = lanes.equal(flat, none)?;

    let next = if back {
        let swung = swapped::<LANES>(lanes, held, keys, true)?;
        let (keys, rest) = split::<LANES>(lanes, swung)?;
        let keys = lifted::<LANES>(lanes, keys, true)?;
        merged::<LANES>(lanes, keys, rest)?
    } else {
        let keys = lifted::<LANES>(lanes, keys, false)?;
        let cells = merged::<LANES>(lanes, keys, rest)?;
        swapped::<LANES>(lanes, cells, keys, false)?
    };

    let mut out = Vec::with_capacity(4);
    for (before, after) in held.into_iter().zip(next) {
        out.push(lanes.select(moving, after, before)?);
    }
    let stirring = lanes.select(moving, one, none)?;
    Ok((four(&out)?, stirring))
}

fn some_of<const LANES: u32>(subgroup: u32, grid: Grid, back: bool) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
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
    let corners = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let places = corners.places;
    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let (next, stirring) = {
        let mut lanes = kernel.lanes()?;
        let held = [
            lanes.splat_id::<U32, LANES>(held[0])?,
            lanes.splat_id::<U32, LANES>(held[1])?,
            lanes.splat_id::<U32, LANES>(held[2])?,
            lanes.splat_id::<U32, LANES>(held[3])?,
        ];
        dissolving::<LANES>(&mut lanes, held, back)?
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
    for (corner, (place, value)) in places.iter().zip(&next).enumerate() {
        let where_at = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let step = lanes.splat_bits::<U32, LANES>(u32::try_from(corner).unwrap_or(0))?;
            let elsewhere = lanes.add(spare, step)?;
            let held = lanes.greater_than(inside, none)?;
            lanes.select(held, *place, elsewhere)?
        };
        kernel.store_at(STATE, where_at.id(), value.id())?;
    }

    let where_at = Waking {
        subgroup,
        grid,
        column: corners.along,
        row: corners.down,
    };
    wake::<LANES>(&mut kernel, where_at, stirring, inside, dump_base)?;
    kernel.finish()
}
