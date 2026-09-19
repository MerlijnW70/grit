use std::time::Duration;

use grit::compact::Plan;
use grit::layout::{BINDINGS, DENSE, GRAND, PARAMS, STATE};
use grit::live::{Waking, corners, wake};
use grit::sand::{self, Grid};
use grit::{Groups, Host, Resident, live, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

pub const SHIFT: u32 = 8;

const SELDOM: u32 = 8;

type Block<const LANES: u32> = [Vector<U32, LANES>; 4];

type Halves<const LANES: u32> = (Block<LANES>, Block<LANES>);

const KEYS: [u32; 5] = [
    0x9E37_79B1,
    0x85EB_CA6B,
    0xC2B2_AE35,
    0x27D4_EB2F,
    0x1656_67B1,
];

pub struct World<'host> {
    resident: Resident<'host>,
    forward: Vec<Vec<Groups>>,
    backward: Vec<Vec<Groups>>,
    grid: Grid,
    plan: Plan,
}

impl<'host> World<'host> {
    pub fn new(
        host: &'host Host,
        grid: Grid,
        shift: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let width = host.width();
        let quads = grid.quads();
        let plan = Plan::over(width, quads.div_ceil(sweep::cells(width)))?;

        let none = Groups::Fixed(0);
        let keeping = live::keeping(width, &plan, false)?;
        let rules = vec![
            (listed(width, grid, false, shift)?, keeping.over),
            (listed(width, grid, true, shift)?, none),
        ];
        let mut stages = keeping.before.clone();
        stages.push(rules);
        stages.extend(keeping.after.clone());
        let (passes, forward) = live::staged(stages);
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

        let sizes = plan.sizes();
        let mut resident = host.resident(&sizes, &modules)?;
        for (binding, words) in sizes.iter().enumerate() {
            resident.write(binding, &vec![0_u32; *words])?;
        }
        resident.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;

        let backward = forward
            .iter()
            .map(|stage| {
                if stage.len() == 2 && stage.get(1) == Some(&none) {
                    vec![none, keeping.over]
                } else {
                    stage.clone()
                }
            })
            .collect();

        Ok(Self {
            resident,
            forward,
            backward,
            grid,
            plan,
        })
    }

    pub const fn grid(&self) -> Grid {
        self.grid
    }

    pub fn quads(&self) -> u32 {
        self.grid.quads()
    }

    pub fn plant(&mut self, cells: &[u32]) -> Result<(), Box<dyn std::error::Error>> {
        self.resident.write(STATE as usize, cells)?;
        self.resident
            .write(PARAMS as usize, &[u32::MAX, self.plan.dump()])?;
        live::seed(&mut self.resident, self.grid, &self.plan)?;
        Ok(())
    }

    pub fn step(&mut self, at: i64, going: i64) -> Result<Duration, grit::host::Error> {
        let phase = u32::try_from(at.rem_euclid(2)).unwrap_or(0);
        self.resident.write_at(PARAMS as usize, 0, &[phase])?;
        let held = if going > 0 {
            &self.forward
        } else {
            &self.backward
        };
        let staged: Vec<&[Groups]> = held.iter().map(Vec::as_slice).collect();
        self.resident.staged(&staged)
    }

    pub fn cells(&mut self) -> Result<Vec<u32>, grit::host::Error> {
        self.resident.read(STATE as usize, self.grid.cells())
    }

    pub fn awake(&mut self) -> Result<u32, grit::host::Error> {
        reading(&mut self.resident)
    }

    pub fn nudge(&mut self, at: usize, bit: u32) -> Result<u32, grit::host::Error> {
        let held = self.cells()?;
        let was = held.get(at).copied().unwrap_or(0);
        let now = was ^ (1 << bit);
        self.resident.write_at(STATE as usize, at, &[now])?;
        Ok(was)
    }
}

pub fn reading(resident: &mut grit::Resident<'_>) -> Result<u32, grit::host::Error> {
    Ok(resident
        .read(GRAND as usize, 1)?
        .first()
        .copied()
        .unwrap_or(0))
}

pub fn asked() -> u32 {
    std::env::args()
        .skip_while(|one| one != "shift")
        .nth(1)
        .and_then(|held| held.parse().ok())
        .unwrap_or(SHIFT)
        .clamp(1, 24)
}

pub fn wanted(grid: Grid) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    match std::env::args().skip_while(|one| one != "photo").nth(1) {
        Some(path) => loaded(&path, grid),
        None => Ok(picture(grid)),
    }
}

pub fn loaded(path: &str, grid: Grid) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let held = image::open(path)?.resize_to_fill(
        grid.wide,
        grid.deep,
        image::imageops::FilterType::Lanczos3,
    );
    let held = held.to_rgb8();
    let mut cells = vec![0_u32; grid.cells()];
    for (at, pixel) in held.pixels().enumerate() {
        let [red, green, blue] = pixel.0;
        if let Some(slot) = cells.get_mut(at) {
            *slot = (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        }
    }
    Ok(cells)
}

pub fn picture(grid: Grid) -> Vec<u32> {
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

fn listed(subgroup: u32, grid: Grid, back: bool, shift: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => some_of::<4>(subgroup, grid, back, shift),
        8 => some_of::<8>(subgroup, grid, back, shift),
        16 => some_of::<16>(subgroup, grid, back, shift),
        32 => some_of::<32>(subgroup, grid, back, shift),
        64 => some_of::<64>(subgroup, grid, back, shift),
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
    shift: u32,
) -> Result<Halves<LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(shift)?;
    let mask = lanes.splat_bits::<U32, LANES>((1 << shift) - 1)?;
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
    shift: u32,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(shift)?;
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
    shift: u32,
) -> Result<(Block<LANES>, Vector<U32, LANES>), LaneError> {
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let (keys, rest) = split::<LANES>(lanes, held, shift)?;
    let flat = level::<LANES>(lanes, keys)?;
    let moving = lanes.equal(flat, none)?;

    let next = if back {
        let swung = swapped::<LANES>(lanes, held, keys, true)?;
        let (keys, rest) = split::<LANES>(lanes, swung, shift)?;
        let keys = lifted::<LANES>(lanes, keys, true)?;
        merged::<LANES>(lanes, keys, rest, shift)?
    } else {
        let keys = lifted::<LANES>(lanes, keys, false)?;
        let cells = merged::<LANES>(lanes, keys, rest, shift)?;
        swapped::<LANES>(lanes, cells, keys, false)?
    };

    let mut out = Vec::with_capacity(4);
    for (before, after) in held.into_iter().zip(next) {
        out.push(lanes.select(moving, after, before)?);
    }
    let stirring = lanes.select(moving, one, none)?;
    Ok((four(&out)?, stirring))
}

fn some_of<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    back: bool,
    shift: u32,
) -> Result<Vec<u32>, LaneError> {
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
        dissolving::<LANES>(&mut lanes, held, back, shift)?
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
