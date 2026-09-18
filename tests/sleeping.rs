#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::{Over, Plan, Tier};
use grit::layout::{AWAKE, BASES, BINDINGS, COUNTS, DENSE, GRAND, HOT, PARAMS, STATE, TOTALS};
use grit::sand::{self, Grid};
use grit::{Groups, Host, compact, live, sweep};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

const WIDE: u32 = 1024;
const DEEP: u32 = 1024;

const STEPS: u32 = 100;

const PATCH: u32 = 24;

fn device(label: &str) -> Option<Host> {
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

fn scattered(grid: Grid, seed: u32) -> Vec<u32> {
    let mut state = seed;
    let mut cells = vec![0_u32; grid.cells()];
    let middle = (grid.wide / 2, grid.deep / 2);
    for row in middle.1..middle.1 + PATCH {
        for column in middle.0..middle.0 + PATCH {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            cells[(row * grid.wide + column) as usize] = state | 1;
        }
    }
    cells
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    Turn,
    Lift,
}

const KEYS: [u32; 4] = [0x9E37_79B9, 0x85EB_CA6B, 0xC2B2_AE35, 0x27D4_EB2F];

fn mixed<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    key: u32,
    first: Vector<U32, LANES>,
    second: Vector<U32, LANES>,
    third: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(key)?;
    let five = lanes.splat_bits::<U32, LANES>(5)?;
    let three = lanes.splat_bits::<U32, LANES>(3)?;

    let held = lanes.mul(first, by)?;
    let up = lanes.shift_left(second, five)?;
    let held = lanes.xor(held, up)?;
    let down = lanes.shift_right_logical(third, three)?;
    lanes.add(held, down)
}

fn lifted<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
    back: bool,
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [mut a, mut b, mut c, mut d] = held;
    if back {
        let step = mixed::<LANES>(lanes, KEYS[3], a, b, c)?;
        d = lanes.sub(d, step)?;
        let step = mixed::<LANES>(lanes, KEYS[2], d, a, b)?;
        c = lanes.sub(c, step)?;
        let step = mixed::<LANES>(lanes, KEYS[1], c, d, a)?;
        b = lanes.sub(b, step)?;
        let step = mixed::<LANES>(lanes, KEYS[0], b, c, d)?;
        a = lanes.sub(a, step)?;
    } else {
        let step = mixed::<LANES>(lanes, KEYS[0], b, c, d)?;
        a = lanes.add(a, step)?;
        let step = mixed::<LANES>(lanes, KEYS[1], c, d, a)?;
        b = lanes.add(b, step)?;
        let step = mixed::<LANES>(lanes, KEYS[2], d, a, b)?;
        c = lanes.add(c, step)?;
        let step = mixed::<LANES>(lanes, KEYS[3], a, b, c)?;
        d = lanes.add(d, step)?;
    }
    Ok([a, b, c, d])
}

struct Corners<const LANES: u32> {
    places: [Vector<U32, LANES>; 4],
    along: Vector<U32, LANES>,
    down: Vector<U32, LANES>,
}

fn corners<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    quad: Id,
    frame: Id,
) -> Result<Corners<LANES>, LaneError> {
    let mut lanes = kernel.lanes()?;
    let which = lanes.splat_id::<U32, LANES>(quad)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;

    let phase = sand::low_bits::<LANES>(&mut lanes, now, 1)?;
    let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
    let down = lanes.shift_right_logical(which, by)?;
    let along = sand::low_bits::<LANES>(&mut lanes, which, grid.across().trailing_zeros())?;

    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let doubled_x = lanes.shift_left(along, one)?;
    let doubled_y = lanes.shift_left(down, one)?;
    let column = lanes.add(doubled_x, phase)?;
    let row = lanes.add(doubled_y, phase)?;
    let right = lanes.add(column, one)?;
    let under = lanes.add(row, one)?;

    let places = [
        sand::at::<LANES>(&mut lanes, grid, column, row)?,
        sand::at::<LANES>(&mut lanes, grid, right, row)?,
        sand::at::<LANES>(&mut lanes, grid, column, under)?,
        sand::at::<LANES>(&mut lanes, grid, right, under)?,
    ];
    Ok(Corners {
        places,
        along,
        down,
    })
}

fn spun<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    held: [Vector<U32, LANES>; 4],
    back: bool,
    rule: Rule,
) -> Result<([Vector<U32, LANES>; 4], Vector<U32, LANES>), LaneError> {
    if rule == Rule::Lift {
        let next = lifted::<LANES>(lanes, held, back)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut stirring = none;
        for (before, after) in held.into_iter().zip(next) {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            stirring = lanes.add(stirring, differs)?;
        }
        let moving = lanes.greater_than(stirring, none)?;
        let stirring = lanes.select(moving, one, none)?;
        return Ok((next, stirring));
    }
    let [a, b, c, d] = held;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let same = |lanes: &mut Lanes<'_>, first, second| {
        let is = lanes.equal(first, second)?;
        lanes.select(is, one, none)
    };
    let flat = same(lanes, a, b)?;
    let next = same(lanes, b, c)?;
    let last = same(lanes, c, d)?;
    let flat = lanes.mul(flat, next)?;
    let flat = lanes.mul(flat, last)?;
    let moving = lanes.equal(flat, none)?;

    let sum = lanes.add(a, b)?;
    let sum = lanes.add(sum, c)?;
    let sum = lanes.add(sum, d)?;
    let odd = sand::low_bits::<LANES>(lanes, sum, 1)?;
    let halfway = lanes.equal(odd, one)?;

    let quarter = if back { [b, d, a, c] } else { [c, a, d, b] };
    let half = [d, c, b, a];

    let mut out = Vec::with_capacity(4);
    for ((before, turned), flipped) in held.into_iter().zip(quarter).zip(half) {
        let taken = lanes.select(halfway, flipped, turned)?;
        out.push(lanes.select(moving, taken, before)?);
    }
    let kept = [
        *out.first().ok_or(LaneError::NoMapping {
            lanes: LANES,
            width: LANES,
        })?,
        *out.get(1).ok_or(LaneError::NoMapping {
            lanes: LANES,
            width: LANES,
        })?,
        *out.get(2).ok_or(LaneError::NoMapping {
            lanes: LANES,
            width: LANES,
        })?,
        *out.get(3).ok_or(LaneError::NoMapping {
            lanes: LANES,
            width: LANES,
        })?,
    ];
    let stirring = lanes.select(moving, one, none)?;
    Ok((kept, stirring))
}

fn every(subgroup: u32, grid: Grid, back: bool, rule: Rule) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => all_of::<4>(subgroup, grid, back, rule),
        8 => all_of::<8>(subgroup, grid, back, rule),
        16 => all_of::<16>(subgroup, grid, back, rule),
        32 => all_of::<32>(subgroup, grid, back, rule),
        64 => all_of::<64>(subgroup, grid, back, rule),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn all_of<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    back: bool,
    rule: Rule,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let quad = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let corners = corners::<LANES>(&mut kernel, grid, quad, frame)?;
    let places = corners.places;

    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];
    let next = {
        let mut lanes = kernel.lanes()?;
        let held = [
            lanes.splat_id::<U32, LANES>(held[0])?,
            lanes.splat_id::<U32, LANES>(held[1])?,
            lanes.splat_id::<U32, LANES>(held[2])?,
            lanes.splat_id::<U32, LANES>(held[3])?,
        ];
        spun::<LANES>(&mut lanes, held, back, rule)?.0
    };
    for (place, value) in places.iter().zip(&next) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    kernel.finish()
}

fn listed(subgroup: u32, grid: Grid, back: bool, rule: Rule) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => some_of::<4>(subgroup, grid, back, rule),
        8 => some_of::<8>(subgroup, grid, back, rule),
        16 => some_of::<16>(subgroup, grid, back, rule),
        32 => some_of::<32>(subgroup, grid, back, rule),
        64 => some_of::<64>(subgroup, grid, back, rule),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn some_of<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    back: bool,
    rule: Rule,
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
        spun::<LANES>(&mut lanes, held, back, rule)?
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

    for below in 0..3_u32 {
        for across in 0..3_u32 {
            let slot_at = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let back = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
                let step_x = lanes.splat_bits::<U32, LANES>(across)?;
                let step_y = lanes.splat_bits::<U32, LANES>(below)?;
                let near_x = lanes.add(corners.along, step_x)?;
                let near_x = lanes.add(near_x, back)?;
                let near_y = lanes.add(corners.down, step_y)?;
                let near_y = lanes.add(near_y, back)?;
                let x =
                    sand::low_bits::<LANES>(&mut lanes, near_x, grid.across().trailing_zeros())?;
                let y = sand::low_bits::<LANES>(&mut lanes, near_y, grid.down().trailing_zeros())?;
                let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
                let stride = lanes.shift_left(y, by)?;
                let flag = lanes.add(stride, x)?;

                let base = lanes.splat_id::<U32, LANES>(dump_base)?;
                let mine = lanes.splat_id::<U32, LANES>(group)?;
                let seat = lanes.splat_id::<U32, LANES>(lane)?;
                let wide = lanes.splat_bits::<U32, LANES>(subgroup)?;
                let spread = lanes.mul(mine, wide)?;
                let elsewhere = lanes.add(base, spread)?;
                let elsewhere = lanes.add(elsewhere, seat)?;

                let both = lanes.mul(stirring, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, flag, elsewhere)?
            };
            let mark = kernel.module().constant_u32(2)?;
            kernel.store_at(AWAKE, slot_at.id(), mark)?;

            let coarse = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let per_block = sweep::cells(subgroup);
                let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
                let blocks = grid.quads().div_ceil(per_block);
                let cold =
                    lanes.splat_bits::<U32, LANES>(blocks.div_ceil(per_block) * per_block)?;
                let which = lanes.shift_right_logical(slot_at, by)?;
                let both = lanes.mul(stirring, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, which, cold)?
            };
            kernel.store_at(HOT, coarse.id(), mark)?;
        }
    }
    kernel.finish()
}

fn stepped(host: &Host, grid: Grid, cells: &[u32], sparse: bool, rule: Rule) -> (Vec<u32>, u32) {
    let width = host.width();
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();

    let one = Groups::Fixed(1);
    let blocks = Groups::Fixed(plan.blocks);
    let tiles = Groups::Fixed(plan.tiles);
    let over = Groups::Fixed(quads.div_ceil(width));

    let turning = if sparse {
        listed(width, grid, false, rule).unwrap()
    } else {
        every(width, grid, false, rule).unwrap()
    };
    let passes = if sparse {
        vec![
            live::tick(width).unwrap(),
            live::decay(width, Over::Every).unwrap(),
            live::wipe(width).unwrap(),
            live::wipe_counts(width, COUNTS).unwrap(),
            turning,
            live::mark(width, Tier::FINE, Over::Every).unwrap(),
            compact::scan(width, COUNTS, BASES, TOTALS).unwrap(),
            compact::scan(width, TOTALS, TOTALS, GRAND).unwrap(),
            compact::gather(width, Tier::FINE, Over::Every, plan.dump()).unwrap(),
            compact::prepare(width, width, Tier::FINE).unwrap(),
        ]
    } else {
        vec![live::tick(width).unwrap(), turning]
    };
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words]).unwrap();
    }
    resident.write(STATE as usize, cells).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut resident, grid, &plan).unwrap();

    let stages: Vec<Vec<Groups>> = if sparse {
        vec![
            vec![one, blocks, one, tiles],
            vec![Plan::listed()],
            vec![blocks],
            vec![tiles],
            vec![one],
            vec![blocks, Groups::Counting(1)],
        ]
    } else {
        vec![vec![one], vec![over]]
    };
    let staged: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();

    for _ in 0..STEPS {
        resident.staged(&staged).unwrap();
    }
    let awake = if sparse {
        resident
            .read(GRAND as usize, 1)
            .unwrap()
            .first()
            .copied()
            .unwrap_or(0)
    } else {
        quads
    };
    (resident.read(STATE as usize, grid.cells()).unwrap(), awake)
}

#[test]
fn the_quads_left_out_had_nothing_to_do() {
    let Some(host) = device("the_quads_left_out_had_nothing_to_do") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let cells = scattered(grid, 0x0510_0BED);

    let (dense, _) = stepped(&host, grid, &cells, false, Rule::Turn);
    let (sparse, awake) = stepped(&host, grid, &cells, true, Rule::Turn);

    let apart = dense
        .iter()
        .zip(&sparse)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!("{} {} {awake} {apart}", grid.quads(), STEPS);

    assert_ne!(dense, cells);
    assert_eq!(apart, 0);
    assert!(awake < grid.quads());
}

fn both_ways(host: &Host, grid: Grid, cells: &[u32], rule: Rule) -> (Vec<u32>, Vec<u32>, u32) {
    let width = host.width();
    let quads = grid.quads();
    let plan = Plan::over(width, quads.div_ceil(sweep::cells(width))).unwrap();

    let one = Groups::Fixed(1);
    let blocks = Groups::Fixed(plan.blocks);
    let tiles = Groups::Fixed(plan.tiles);
    let none = Groups::Fixed(0);

    let passes = vec![
        live::decay(width, Over::Every).unwrap(),
        live::wipe(width).unwrap(),
        live::wipe_counts(width, COUNTS).unwrap(),
        listed(width, grid, false, rule).unwrap(),
        listed(width, grid, true, rule).unwrap(),
        live::mark(width, Tier::FINE, Over::Every).unwrap(),
        compact::scan(width, COUNTS, BASES, TOTALS).unwrap(),
        compact::scan(width, TOTALS, TOTALS, GRAND).unwrap(),
        compact::gather(width, Tier::FINE, Over::Every, plan.dump()).unwrap(),
        compact::prepare(width, width, Tier::FINE).unwrap(),
    ];
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();

    let sizes = plan.sizes();
    let mut resident = host.resident(&sizes, &modules).unwrap();
    for (binding, words) in sizes.iter().enumerate() {
        resident.write(binding, &vec![0_u32; *words]).unwrap();
    }
    resident.write(STATE as usize, cells).unwrap();
    resident
        .write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut resident, grid, &plan).unwrap();

    let ahead: Vec<Vec<Groups>> = vec![
        vec![blocks, one, tiles],
        vec![Plan::listed(), none],
        vec![blocks],
        vec![tiles],
        vec![one],
        vec![blocks, Groups::Counting(1)],
    ];
    let back: Vec<Vec<Groups>> = vec![
        vec![blocks, one, tiles],
        vec![none, Plan::listed()],
        vec![blocks],
        vec![tiles],
        vec![one],
        vec![blocks, Groups::Counting(1)],
    ];
    let forward: Vec<&[Groups]> = ahead.iter().map(Vec::as_slice).collect();
    let backward: Vec<&[Groups]> = back.iter().map(Vec::as_slice).collect();

    for frame in 0..STEPS {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&forward).unwrap();
    }
    let stirred = resident.read(STATE as usize, grid.cells()).unwrap();
    let awake = resident
        .read(GRAND as usize, 1)
        .unwrap()
        .first()
        .copied()
        .unwrap_or(0);

    for frame in (0..STEPS).rev() {
        resident.write_at(PARAMS as usize, 0, &[frame]).unwrap();
        resident.staged(&backward).unwrap();
    }
    let home = resident.read(STATE as usize, grid.cells()).unwrap();
    (stirred, home, awake)
}

#[test]
fn a_sparse_run_undoes_itself_over_the_list_it_kept() {
    let Some(host) = device("a_sparse_run_undoes_itself_over_the_list_it_kept") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let cells = scattered(grid, 0x0510_0BED);

    let (stirred, home, awake) = both_ways(&host, grid, &cells, Rule::Turn);
    let apart = home
        .iter()
        .zip(&cells)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!("{} {} {awake} {apart}", grid.quads(), STEPS);

    assert_ne!(stirred, cells);
    assert_eq!(apart, 0);
}

#[test]
fn a_rule_that_computes_still_undoes_itself_over_the_list() {
    let Some(host) = device("a_rule_that_computes_still_undoes_itself") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let cells = scattered(grid, 0x0510_0BED);

    let (stirred, home, awake) = both_ways(&host, grid, &cells, Rule::Lift);
    let apart = home
        .iter()
        .zip(&cells)
        .filter(|(one, other)| one != other)
        .count();
    let touched = stirred.iter().filter(|word| **word != 0).count();
    let began = cells.iter().filter(|word| **word != 0).count();
    eprintln!("{} {STEPS} {awake} {began} {touched} {apart}", grid.quads());

    assert_ne!(stirred, cells);
    assert!(touched > began * 4);
    assert!(awake < grid.quads());
    assert_eq!(apart, 0);
}

#[test]
fn the_computing_rule_skips_nothing_the_dense_run_did() {
    let Some(host) = device("the_computing_rule_skips_nothing_the_dense_run_did") else {
        return;
    };
    let grid = Grid::of(WIDE, DEEP);
    let cells = scattered(grid, 0x0BED_0510);

    let (dense, _) = stepped(&host, grid, &cells, false, Rule::Lift);
    let (sparse, awake) = stepped(&host, grid, &cells, true, Rule::Lift);

    let apart = dense
        .iter()
        .zip(&sparse)
        .filter(|(one, other)| one != other)
        .count();
    eprintln!("{} {STEPS} {awake} {apart}", grid.quads());

    assert_ne!(dense, cells);
    assert_eq!(apart, 0);
}
