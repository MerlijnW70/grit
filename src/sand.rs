use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

pub use crate::layout::{BINDINGS, BRUSH, BRUSH_WORDS, LOOKING, PIXELS};
use crate::layout::{PARAMS, STATE};

pub const AIR: u32 = 0;
pub const WATER: u32 = 1;
pub const SAND: u32 = 2;
pub const WALL: u32 = 3;

pub const WEIGHTS: [u32; 4] = [AIR, WATER, SAND, WALL];

pub const RUNNING_LEFT: u32 = WATER + 4;
pub const RUNNING_RIGHT: u32 = WATER + 8;

const MATERIAL_BITS: u32 = 2;
const CARRIED_BITS: u32 = 4;
pub const HELD: u32 = 1 << CARRIED_BITS;

const TIRED_BITS: u32 = 3;

pub const TIRED: u32 = HELD << 1;

pub const PATIENCE: u32 = (1 << TIRED_BITS) - 1;

const LEFTWARD: u32 = 1;
const RIGHTWARD: u32 = 2;

pub fn material<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    word: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    low_bits::<LANES>(lanes, word, MATERIAL_BITS)
}

pub fn running<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    word: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(MATERIAL_BITS)?;
    let above = lanes.shift_right_logical(word, by)?;
    low_bits::<LANES>(lanes, above, MATERIAL_BITS)
}

pub fn hold<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cell: Vector<U32, LANES>,
    under: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let water = lanes.splat_bits::<U32, LANES>(WATER)?;
    let bit = lanes.splat_bits::<U32, LANES>(HELD)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let kind = material::<LANES>(lanes, cell)?;
    let beneath = material::<LANES>(lanes, under)?;
    let wet = lanes.equal(kind, water)?;
    let standing = lanes.equal(beneath, water)?;
    let wet = lanes.select(wet, one, none)?;
    let standing = lanes.select(standing, one, none)?;

    let both = lanes.mul(wet, standing)?;
    let carried = low_bits::<LANES>(lanes, cell, CARRIED_BITS)?;
    let set = lanes.mul(both, bit)?;
    let kept = lanes.add(carried, set)?;

    let step = lanes.splat_bits::<U32, LANES>(TIRED)?;
    let worn = tired::<LANES>(lanes, cell)?;
    let worn = lanes.mul(worn, step)?;
    lanes.add(kept, worn)
}

pub fn held<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cell: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(CARRIED_BITS)?;
    let above = lanes.shift_right_logical(cell, by)?;
    low_bits::<LANES>(lanes, above, 1)
}

pub fn tired<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cell: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(CARRIED_BITS + 1)?;
    let above = lanes.shift_right_logical(cell, by)?;
    low_bits::<LANES>(lanes, above, TIRED_BITS)
}

#[must_use]
pub const fn material_of(word: u32) -> u32 {
    word % (1 << MATERIAL_BITS)
}

#[must_use]
pub const fn running_of(word: u32) -> u32 {
    (word >> MATERIAL_BITS) % (1 << MATERIAL_BITS)
}

#[must_use]
pub const fn held_of(word: u32) -> u32 {
    (word >> CARRIED_BITS) % 2
}

#[must_use]
pub const fn tired_of(word: u32) -> u32 {
    (word >> (CARRIED_BITS + 1)) % (1 << TIRED_BITS)
}

#[must_use]
pub const fn hold_of(word: u32, under: u32) -> u32 {
    let carried = word % HELD;
    let worn = tired_of(word) * TIRED;
    if material_of(word) == WATER && material_of(under) == WATER {
        carried + HELD + worn
    } else {
        carried + worn
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub wide: u32,
    pub deep: u32,
}

impl Grid {
    #[must_use]
    pub const fn of(wide: u32, deep: u32) -> Self {
        Self {
            wide: wide.next_power_of_two(),
            deep: deep.next_power_of_two(),
        }
    }

    #[must_use]
    pub const fn cells(self) -> usize {
        self.wide as usize * self.deep as usize
    }

    #[must_use]
    pub const fn across(self) -> u32 {
        self.wide / 2
    }

    #[must_use]
    pub const fn down(self) -> u32 {
        self.deep / 2
    }

    #[must_use]
    pub const fn quads(self) -> u32 {
        self.across() * self.down()
    }
}

pub fn settle(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => settled::<4>(subgroup, grid),
        8 => settled::<8>(subgroup, grid),
        16 => settled::<16>(subgroup, grid),
        32 => settled::<32>(subgroup, grid),
        64 => settled::<64>(subgroup, grid),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn settled<const LANES: u32>(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, crate::compact::BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let block = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    let (top_left, top_right, bottom_left, bottom_right) = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;

        let phase = low_bits::<LANES>(&mut lanes, now, 1)?;

        let by_across = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
        let down_a_row = lanes.shift_right_logical(which, by_across)?;
        let along = low_bits::<LANES>(&mut lanes, which, grid.across().trailing_zeros())?;

        let two = lanes.splat_bits::<U32, LANES>(1)?;
        let column = lanes.shift_left(along, two)?;
        let row = lanes.shift_left(down_a_row, two)?;
        let column = lanes.add(column, phase)?;
        let row = lanes.add(row, phase)?;

        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let right = lanes.add(column, one)?;
        let under = lanes.add(row, one)?;

        (
            at::<LANES>(&mut lanes, grid, column, row)?,
            at::<LANES>(&mut lanes, grid, right, row)?,
            at::<LANES>(&mut lanes, grid, column, under)?,
            at::<LANES>(&mut lanes, grid, right, under)?,
        )
    };

    let places = [top_left, top_right, bottom_left, bottom_right];
    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let settled = {
        let mut lanes = kernel.lanes()?;
        let a = lanes.splat_id::<U32, LANES>(held[0])?;
        let b = lanes.splat_id::<U32, LANES>(held[1])?;
        let c = lanes.splat_id::<U32, LANES>(held[2])?;
        let d = lanes.splat_id::<U32, LANES>(held[3])?;

        let a = hold::<LANES>(&mut lanes, a, c)?;
        let b = hold::<LANES>(&mut lanes, b, d)?;

        let [a, b, c, d] = start::<LANES>(&mut lanes, [a, b, c, d])?;

        let (a, b) = course::<LANES>(&mut lanes, a, b)?;
        let (c, d) = course::<LANES>(&mut lanes, c, d)?;

        let (a, c) = fall::<LANES>(&mut lanes, a, c)?;
        let (b, d) = fall::<LANES>(&mut lanes, b, d)?;

        let (a, d) = fall::<LANES>(&mut lanes, a, d)?;
        let (b, c) = fall::<LANES>(&mut lanes, b, c)?;

        [a, b, c, d]
    };

    for (place, value) in places.iter().zip(&settled) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }

    kernel.finish()
}

pub fn fall<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    upper: Vector<U32, LANES>,
    lower: Vector<U32, LANES>,
) -> Result<(Vector<U32, LANES>, Vector<U32, LANES>), LaneError> {
    let wall = lanes.splat_bits::<U32, LANES>(WALL)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let above = material::<LANES>(lanes, upper)?;
    let below = material::<LANES>(lanes, lower)?;

    let heavier = lanes.greater_than(above, below)?;
    let movable = lanes.greater_than(wall, above)?;
    let falling = lanes.select(heavier, one, none)?;
    let loose = lanes.select(movable, one, none)?;
    let both = lanes.mul(falling, loose)?;
    let moving = lanes.greater_than(both, none)?;

    let upper_next = lanes.select(moving, lower, upper)?;
    let lower_next = lanes.select(moving, above, lower)?;
    Ok((upper_next, lower_next))
}

pub fn course<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    left: Vector<U32, LANES>,
    right: Vector<U32, LANES>,
) -> Result<(Vector<U32, LANES>, Vector<U32, LANES>), LaneError> {
    let air = lanes.splat_bits::<U32, LANES>(AIR)?;
    let leftward = lanes.splat_bits::<U32, LANES>(LEFTWARD)?;
    let rightward = lanes.splat_bits::<U32, LANES>(RIGHTWARD)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let here = material::<LANES>(lanes, left)?;
    let there = material::<LANES>(lanes, right)?;
    let going_here = running::<LANES>(lanes, left)?;
    let going_there = running::<LANES>(lanes, right)?;

    let is = |lanes: &mut Lanes<'_>, value: Vector<U32, LANES>, want: Vector<U32, LANES>| {
        let same = lanes.equal(value, want)?;
        lanes.select(same, one, none)
    };
    let is_not = |lanes: &mut Lanes<'_>, value: Vector<U32, LANES>, want: Vector<U32, LANES>| {
        let same = lanes.equal(value, want)?;
        lanes.select(same, none, one)
    };

    let wants_right = is(lanes, going_here, rightward)?;
    let wants_left = is(lanes, going_there, leftward)?;
    let right_free = is(lanes, there, air)?;
    let left_free = is(lanes, here, air)?;
    let right_shut = is_not(lanes, there, air)?;
    let left_shut = is_not(lanes, here, air)?;

    let goes_right = lanes.mul(wants_right, right_free)?;
    let goes_left = lanes.mul(wants_left, left_free)?;
    let either = lanes.add(goes_right, goes_left)?;
    let swapping = lanes.greater_than(either, none)?;

    let bit = lanes.splat_bits::<U32, LANES>(HELD)?;
    let step = lanes.splat_bits::<U32, LANES>(TIRED)?;
    let halt = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>, kind: Vector<U32, LANES>| {
        let memory = held::<LANES>(lanes, cell)?;
        let up = lanes.mul(memory, bit)?;
        let stilled = lanes.add(kind, up)?;
        let worn = tired::<LANES>(lanes, cell)?;
        let worn = lanes.mul(worn, step)?;
        lanes.add(stilled, worn)
    };
    let stops_here = lanes.mul(wants_right, right_shut)?;
    let stops_there = lanes.mul(wants_left, left_shut)?;
    let halted_here = lanes.greater_than(stops_here, none)?;
    let halted_there = lanes.greater_than(stops_there, none)?;
    let stilled_here = halt(lanes, left, here)?;
    let stilled_there = halt(lanes, right, there)?;
    let left_kept = lanes.select(halted_here, stilled_here, left)?;
    let right_kept = lanes.select(halted_there, stilled_there, right)?;

    let left_next = lanes.select(swapping, right_kept, left_kept)?;
    let right_next = lanes.select(swapping, left_kept, right_kept)?;
    Ok((left_next, right_next))
}

pub fn start<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    block: [Vector<U32, LANES>; 4],
) -> Result<[Vector<U32, LANES>; 4], LaneError> {
    let [a, b, c, d] = block;
    let air = lanes.splat_bits::<U32, LANES>(AIR)?;
    let water = lanes.splat_bits::<U32, LANES>(WATER)?;
    let to_left = lanes.splat_bits::<U32, LANES>(RUNNING_LEFT)?;
    let to_right = lanes.splat_bits::<U32, LANES>(RUNNING_RIGHT)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let flag = |lanes: &mut Lanes<'_>, value: Vector<U32, LANES>, want: Vector<U32, LANES>| {
        let same = lanes.equal(value, want)?;
        lanes.select(same, one, none)
    };
    let is = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>, want: Vector<U32, LANES>| {
        let kind = material::<LANES>(lanes, cell)?;
        let same = lanes.equal(kind, want)?;
        lanes.select(same, one, none)
    };
    let idle = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>| {
        let wet = is(lanes, cell, water)?;
        let way = running::<LANES>(lanes, cell)?;
        let stopped = flag(lanes, way, none)?;
        lanes.mul(wet, stopped)
    };
    let standing = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>| {
        let bit = held::<LANES>(lanes, cell)?;
        lanes.greater_than(bit, none).and_then(|is| {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            lanes.select(is, one, none)
        })
    };

    let all = |lanes: &mut Lanes<'_>,
               first: Vector<U32, LANES>,
               second: Vector<U32, LANES>,
               third: Vector<U32, LANES>| {
        let both = lanes.mul(first, second)?;
        lanes.mul(both, third)
    };

    let a_idle = idle(lanes, a)?;
    let b_idle = idle(lanes, b)?;
    let c_idle = idle(lanes, c)?;
    let d_idle = idle(lanes, d)?;
    let a_held = standing(lanes, a)?;
    let b_held = standing(lanes, b)?;
    let c_held = standing(lanes, c)?;
    let d_held = standing(lanes, d)?;
    let a_free = is(lanes, a, air)?;
    let b_free = is(lanes, b, air)?;
    let c_free = is(lanes, c, air)?;
    let d_free = is(lanes, d, air)?;

    let willing = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>| {
        let worn = tired::<LANES>(lanes, cell)?;
        let done = lanes.splat_bits::<U32, LANES>(PATIENCE)?;
        let spare = lanes.greater_than(done, worn)?;
        lanes.select(spare, one, none)
    };
    let a_spare = willing(lanes, a)?;
    let b_spare = willing(lanes, b)?;
    let c_spare = willing(lanes, c)?;
    let d_spare = willing(lanes, d)?;

    let a_goes = all(lanes, a_idle, a_held, b_free)?;
    let b_goes = all(lanes, b_idle, b_held, a_free)?;
    let c_goes = all(lanes, c_idle, c_held, d_free)?;
    let d_goes = all(lanes, d_idle, d_held, c_free)?;
    let a_goes = lanes.mul(a_goes, a_spare)?;
    let b_goes = lanes.mul(b_goes, b_spare)?;
    let c_goes = lanes.mul(c_goes, c_spare)?;
    let d_goes = lanes.mul(d_goes, d_spare)?;

    let goes = |lanes: &mut Lanes<'_>,
                cell: Vector<U32, LANES>,
                when: Vector<U32, LANES>,
                way: Vector<U32, LANES>| {
        let fires = lanes.greater_than(when, none)?;
        let bit = held::<LANES>(lanes, cell)?;
        let up = lanes.splat_bits::<U32, LANES>(HELD)?;
        let kept = lanes.mul(bit, up)?;
        let running = lanes.add(way, kept)?;

        let step = lanes.splat_bits::<U32, LANES>(TIRED)?;
        let worn = tired::<LANES>(lanes, cell)?;
        let worn = lanes.add(worn, one)?;
        let worn = lanes.mul(worn, step)?;
        let running = lanes.add(running, worn)?;
        lanes.select(fires, running, cell)
    };
    Ok([
        goes(lanes, a, a_goes, to_right)?,
        goes(lanes, b, b_goes, to_left)?,
        goes(lanes, c, c_goes, to_right)?,
        goes(lanes, d, d_goes, to_left)?,
    ])
}

pub fn at<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    grid: Grid,
    column: Vector<U32, LANES>,
    row: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let column = low_bits::<LANES>(lanes, column, grid.wide.trailing_zeros())?;
    let row = low_bits::<LANES>(lanes, row, grid.deep.trailing_zeros())?;

    let by = lanes.splat_bits::<U32, LANES>(grid.wide.trailing_zeros())?;
    let along = lanes.shift_left(row, by)?;
    lanes.add(along, column)
}

pub fn low_bits<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    value: Vector<U32, LANES>,
    bits: u32,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(bits)?;
    let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;

    let above = lanes.shift_right_logical(value, by)?;
    let back = lanes.shift_left(above, by)?;
    let away = lanes.mul(back, minus)?;
    lanes.add(value, away)
}

#[must_use]
pub fn settle_reference(grid: Grid, frame: u32, cells: &[u32]) -> Vec<u32> {
    let mut out = cells.to_vec();
    let phase = frame & 1;

    for block in 0..grid.quads() as usize {
        let column = ((block as u32 % grid.across()) * 2 + phase) & (grid.wide - 1);
        let row = ((block as u32 / grid.across()) * 2 + phase) & (grid.deep - 1);
        let right = (column + 1) & (grid.wide - 1);
        let under = (row + 1) & (grid.deep - 1);

        let places = [
            (row * grid.wide + column) as usize,
            (row * grid.wide + right) as usize,
            (under * grid.wide + column) as usize,
            (under * grid.wide + right) as usize,
        ];
        let mut cells = [AIR; 4];
        for (slot, place) in cells.iter_mut().zip(places) {
            *slot = out.get(place).copied().unwrap_or(WALL);
        }

        let put = |cells: &mut [u32; 4], at: usize, value: u32| {
            if let Some(slot) = cells.get_mut(at) {
                *slot = value;
            }
        };
        let read = |cells: &[u32; 4], at: usize| cells.get(at).copied().unwrap_or(WALL);

        let along = |cells: &mut [u32; 4], left: usize, right: usize| {
            let (here, there) = (read(cells, left), read(cells, right));
            let goes_right = running_of(here) == RIGHTWARD;
            let goes_left = running_of(there) == LEFTWARD;
            let right_free = material_of(there) == AIR;
            let left_free = material_of(here) == AIR;

            let halt =
                |word: u32| material_of(word) + held_of(word) * HELD + tired_of(word) * TIRED;
            let kept_left = if goes_right && !right_free {
                halt(here)
            } else {
                here
            };
            let kept_right = if goes_left && !left_free {
                halt(there)
            } else {
                there
            };
            if (goes_right && right_free) || (goes_left && left_free) {
                put(cells, left, kept_right);
                put(cells, right, kept_left);
            } else {
                put(cells, left, kept_left);
                put(cells, right, kept_right);
            }
        };
        let (top, under) = (read(&cells, 0), read(&cells, 2));
        put(&mut cells, 0, hold_of(top, under));
        let (top, under) = (read(&cells, 1), read(&cells, 3));
        put(&mut cells, 1, hold_of(top, under));

        let (a, b) = (read(&cells, 0), read(&cells, 1));
        let (c, d) = (read(&cells, 2), read(&cells, 3));
        let idle = |word: u32| material_of(word) == WATER && running_of(word) == 0;
        let ready = |word: u32| idle(word) && held_of(word) != 0 && tired_of(word) < PATIENCE;
        let started = |word: u32, way: u32| way + HELD + (tired_of(word) + 1) * TIRED;
        if ready(a) && material_of(b) == AIR {
            put(&mut cells, 0, started(a, RUNNING_RIGHT));
        }
        if ready(b) && material_of(a) == AIR {
            put(&mut cells, 1, started(b, RUNNING_LEFT));
        }
        if ready(c) && material_of(d) == AIR {
            put(&mut cells, 2, started(c, RUNNING_RIGHT));
        }
        if ready(d) && material_of(c) == AIR {
            put(&mut cells, 3, started(d, RUNNING_LEFT));
        }

        along(&mut cells, 0, 1);
        along(&mut cells, 2, 3);

        let moved = |cells: &mut [u32; 4], upper: usize, lower: usize| {
            let (up, low) = (read(cells, upper), read(cells, lower));
            let (above, below) = (material_of(up), material_of(low));
            if above > below && above != WALL {
                put(cells, upper, low);
                put(cells, lower, above);
            }
        };
        moved(&mut cells, 0, 2);
        moved(&mut cells, 1, 3);
        moved(&mut cells, 0, 3);
        moved(&mut cells, 1, 2);

        for (place, value) in places.into_iter().zip(cells) {
            if let Some(slot) = out.get_mut(place) {
                *slot = value;
            }
        }
    }

    out
}

pub const BOX: u32 = 32;

/// How many seats the brush dispatches, whatever the grid measures.
pub const SEATS: usize = (BOX * BOX) as usize;

pub fn brush(subgroup: u32, grid: Grid, waking: bool) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => brushed::<4>(subgroup, grid, waking),
        8 => brushed::<8>(subgroup, grid, waking),
        16 => brushed::<16>(subgroup, grid, waking),
        32 => brushed::<32>(subgroup, grid, waking),
        64 => brushed::<64>(subgroup, grid, waking),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn brushed<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    waking: bool,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let seat = kernel.module().i_add(uint, base, lane)?;

    let word = |kernel: &mut Kernel<U32>, at: usize| -> Result<Id, LaneError> {
        let slot = kernel
            .module()
            .constant_u32(u32::try_from(BRUSH + at).unwrap_or(0))?;
        kernel.load_at(PARAMS, slot)
    };
    let where_x = word(&mut kernel, 0)?;
    let where_y = word(&mut kernel, 1)?;
    let how_wide = word(&mut kernel, 2)?;
    let what_of = word(&mut kernel, 3)?;

    let (cell, column, row) = {
        let mut lanes = kernel.lanes()?;
        let within = lanes.splat_id::<U32, LANES>(seat)?;
        let by = lanes.splat_bits::<U32, LANES>(BOX.trailing_zeros())?;
        let across_box = low_bits::<LANES>(&mut lanes, within, BOX.trailing_zeros())?;
        let down_box = lanes.shift_right_logical(within, by)?;

        let half = lanes.splat_bits::<U32, LANES>(BOX / 2)?;
        let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
        let back = lanes.mul(half, minus)?;

        let bx = lanes.splat_id::<U32, LANES>(where_x)?;
        let by_row = lanes.splat_id::<U32, LANES>(where_y)?;
        let column = lanes.add(bx, across_box)?;
        let column = lanes.add(column, back)?;
        let row = lanes.add(by_row, down_box)?;
        let row = lanes.add(row, back)?;

        let column = low_bits::<LANES>(&mut lanes, column, grid.wide.trailing_zeros())?;
        let row = low_bits::<LANES>(&mut lanes, row, grid.deep.trailing_zeros())?;
        let cell = at::<LANES>(&mut lanes, grid, column, row)?;
        (cell, column, row)
    };

    let held = kernel.load_at(STATE, cell.id())?;

    let painted = {
        let mut lanes = kernel.lanes()?;
        let there = lanes.splat_id::<U32, LANES>(held)?;

        let bx = lanes.splat_id::<U32, LANES>(where_x)?;
        let by_row = lanes.splat_id::<U32, LANES>(where_y)?;
        let reach = lanes.splat_id::<U32, LANES>(how_wide)?;
        let material = lanes.splat_id::<U32, LANES>(what_of)?;

        let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
        let air = lanes.splat_bits::<U32, LANES>(AIR)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let away_x = lanes.mul(bx, minus)?;
        let away_y = lanes.mul(by_row, minus)?;
        let dx = lanes.add(column, away_x)?;
        let dy = lanes.add(row, away_y)?;
        let across_sq = lanes.mul(dx, dx)?;
        let down_sq = lanes.mul(dy, dy)?;
        let far = lanes.add(across_sq, down_sq)?;

        let inside = lanes.greater_than(reach, far)?;
        let empty = lanes.equal(there, air)?;
        let pouring = lanes.greater_than(material, none)?;
        let near = lanes.select(inside, one, none)?;
        let free = lanes.select(empty, one, none)?;
        let asked = lanes.select(pouring, one, none)?;

        let both = lanes.mul(near, free)?;
        let all = lanes.mul(both, asked)?;
        let paint = lanes.greater_than(all, none)?;
        lanes.select(paint, material, there)?
    };

    kernel.store_at(STATE, cell.id(), painted.id())?;

    if waking {
        let changed = {
            let mut lanes = kernel.lanes()?;
            let was = lanes.splat_id::<U32, LANES>(held)?;
            let same = lanes.equal(painted, was)?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            lanes.select(same, none, one)?
        };
        let (quad_x, quad_y) = {
            let mut lanes = kernel.lanes()?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            (
                lanes.shift_right_logical(column, one)?,
                lanes.shift_right_logical(row, one)?,
            )
        };
        let second = kernel.module().constant_u32(1)?;
        let dump_base = kernel.load_at(PARAMS, second)?;
        let always = {
            let mut lanes = kernel.lanes()?;
            lanes.splat_bits::<U32, LANES>(1)?
        };
        let where_at = crate::live::Waking {
            subgroup,
            grid,
            column: quad_x,
            row: quad_y,
        };
        crate::live::wake::<LANES>(&mut kernel, where_at, changed, always, dump_base)?;
    }

    kernel.finish()
}

#[derive(Debug, Clone, Copy)]
pub struct View {
    pub wide: u32,
    pub deep: u32,
}

impl View {
    #[must_use]
    pub const fn of(wide: u32, deep: u32) -> Self {
        Self {
            wide: wide.next_power_of_two(),
            deep: deep.next_power_of_two(),
        }
    }

    #[must_use]
    pub const fn pixels(self) -> usize {
        self.wide as usize * self.deep as usize
    }
}

pub fn paint(subgroup: u32, grid: Grid, view: View) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => painted::<4>(subgroup, grid, view),
        8 => painted::<8>(subgroup, grid, view),
        16 => painted::<16>(subgroup, grid, view),
        32 => painted::<32>(subgroup, grid, view),
        64 => painted::<64>(subgroup, grid, view),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn painted<const LANES: u32>(subgroup: u32, grid: Grid, view: View) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let pixel = kernel.module().i_add(uint, base, lane)?;

    let first = kernel.module().constant_u32(looking(0))?;
    let second = kernel.module().constant_u32(looking(1))?;
    let from_x = kernel.load_at(PARAMS, first)?;
    let from_y = kernel.load_at(PARAMS, second)?;

    let cell = {
        let mut lanes = kernel.lanes()?;
        let seat = lanes.splat_id::<U32, LANES>(pixel)?;
        let across = low_bits::<LANES>(&mut lanes, seat, view.wide.trailing_zeros())?;
        let by = lanes.splat_bits::<U32, LANES>(view.wide.trailing_zeros())?;
        let down = lanes.shift_right_logical(seat, by)?;

        let corner_x = lanes.splat_id::<U32, LANES>(from_x)?;
        let corner_y = lanes.splat_id::<U32, LANES>(from_y)?;
        let column = lanes.add(corner_x, across)?;
        let row = lanes.add(corner_y, down)?;
        at::<LANES>(&mut lanes, grid, column, row)?
    };
    let held = kernel.load_at(STATE, cell.id())?;

    let colour = {
        let mut lanes = kernel.lanes()?;
        let word = lanes.splat_id::<U32, LANES>(held)?;
        let there = material::<LANES>(&mut lanes, word)?;

        let water = lanes.splat_bits::<U32, LANES>(WATER)?;
        let sand = lanes.splat_bits::<U32, LANES>(SAND)?;
        let wall = lanes.splat_bits::<U32, LANES>(WALL)?;
        let sky = lanes.splat_bits::<U32, LANES>(0x0011_1622)?;
        let wave = lanes.splat_bits::<U32, LANES>(0x0024_6FA8)?;
        let grain = lanes.splat_bits::<U32, LANES>(0x00D9_B36A)?;
        let stone = lanes.splat_bits::<U32, LANES>(0x0044_4A52)?;

        let is_water = lanes.equal(there, water)?;
        let is_sand = lanes.equal(there, sand)?;
        let is_wall = lanes.equal(there, wall)?;
        let ground = lanes.select(is_wall, stone, sky)?;
        let wet = lanes.select(is_water, wave, ground)?;
        lanes.select(is_sand, grain, wet)?
    };

    kernel.store_at(PIXELS, pixel, colour.id())?;
    kernel.finish()
}

fn looking(part: usize) -> u32 {
    u32::try_from(LOOKING + part).unwrap_or(u32::MAX)
}
