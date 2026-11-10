use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

use crate::geometry::{self, Emit, Held, Plane};
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
    /// The cells along the third axis: 1 for a 2D grid.
    pub tall: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unfit {
    Side { asked: u32 },
    TooMany { wide: u32, deep: u32 },
    Empty { wide: u32, deep: u32 },
    TooTall { wide: u32, deep: u32, tall: u32 },
}

impl std::fmt::Display for Unfit {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Side { asked } => write!(
                out,
                "a side of {asked} cells rounds up past the largest power of two a u32 holds"
            ),
            Self::TooMany { wide, deep } => write!(
                out,
                "a grid of {wide} by {deep} cells has more quads than a u32 counts"
            ),
            Self::Empty { wide, deep } => write!(
                out,
                "a grid of {wide} by {deep} cells holds no whole quad of two by two"
            ),
            Self::TooTall { wide, deep, tall } => write!(
                out,
                "a grid of {wide} by {deep} by {tall} cells has more octs than a u32 counts"
            ),
        }
    }
}

impl std::error::Error for Unfit {}

impl Grid {
    /// The grid `of` gives, refused where `of` would wrap: a side that
    /// rounds past `2^31`, quads that pass a `u32`, cells that pass a `usize`,
    /// or a side under two that leaves no quad at all.
    pub const fn new(wide: u32, deep: u32) -> Result<Self, Unfit> {
        let Some(rounded_wide) = wide.checked_next_power_of_two() else {
            return Err(Unfit::Side { asked: wide });
        };
        let Some(rounded_deep) = deep.checked_next_power_of_two() else {
            return Err(Unfit::Side { asked: deep });
        };
        let grid = Self {
            wide: rounded_wide,
            deep: rounded_deep,
            tall: 1,
        };
        if grid.across() == 0 || grid.down() == 0 {
            return Err(Unfit::Empty {
                wide: rounded_wide,
                deep: rounded_deep,
            });
        }
        let quads = grid.across().checked_mul(grid.down());
        let cells = (rounded_wide as usize).checked_mul(rounded_deep as usize);
        if quads.is_none() || cells.is_none() {
            return Err(Unfit::TooMany {
                wide: rounded_wide,
                deep: rounded_deep,
            });
        }
        Ok(grid)
    }

    /// The 3D grid `wide` by `deep` by `tall`, each side rounded up to a
    /// power of two, refused where [`Grid::new`] refuses the first two sides,
    /// where `tall` rounds past `2^31`, or where its octs pass a `u32` or its
    /// cells a `usize`. A `tall` of 0 or 1 gives the 2D grid.
    pub const fn solid(wide: u32, deep: u32, tall: u32) -> Result<Self, Unfit> {
        let Some(rounded_tall) = tall.checked_next_power_of_two() else {
            return Err(Unfit::Side { asked: tall });
        };
        let flat = match Self::new(wide, deep) {
            Ok(flat) => flat,
            Err(why) => return Err(why),
        };
        let grid = Self {
            tall: rounded_tall,
            ..flat
        };
        let octs = flat.quads().checked_mul(rounded_tall / 2);
        let cells = flat.cells().checked_mul(rounded_tall as usize);
        if octs.is_none() || cells.is_none() {
            return Err(Unfit::TooTall {
                wide: grid.wide,
                deep: grid.deep,
                tall: rounded_tall,
            });
        }
        Ok(grid)
    }

    #[must_use]
    pub const fn of(wide: u32, deep: u32) -> Self {
        Self {
            wide: wide.next_power_of_two(),
            deep: deep.next_power_of_two(),
            tall: 1,
        }
    }

    /// 2 for a flat grid, 3 for a solid one.
    #[must_use]
    pub const fn axes(self) -> u32 {
        if self.tall > 1 { 3 } else { 2 }
    }

    #[must_use]
    pub const fn cells(self) -> usize {
        self.wide as usize * self.deep as usize * self.tall as usize
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

    /// The blocks of two cells along every axis: quads in 2D, octs in 3D.
    #[must_use]
    pub const fn blocks(self) -> u32 {
        if self.tall > 1 {
            self.quads() * (self.tall / 2)
        } else {
            self.quads()
        }
    }
}

/// Why a kernel built for a flat grid refuses a solid one: it would read
/// only x and y, and step every layer as if it were layer zero.
pub const SOLID: &str = "the grid is solid (tall > 1), and this kernel steps a flat grid";

/// Refuses a solid grid at the entry of `operation`, a kernel built for a
/// flat grid.
pub const fn flat(operation: &'static str, grid: Grid) -> Result<(), LaneError> {
    if grid.tall > 1 {
        Err(LaneError::NoSuchForm {
            operation,
            because: SOLID,
        })
    } else {
        Ok(())
    }
}

pub fn settle(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    flat("sand::settle", grid)?;
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

    let places = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        Plane::block(&mut Emit::<LANES>(&mut lanes), grid, which, now)?.places
    };
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
    flat("sand::at", grid)?;
    Plane::at(
        &mut Emit::<LANES>(lanes),
        Plane::extents(grid),
        [column, row],
    )
}

pub fn low_bits<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    value: Vector<U32, LANES>,
    bits: u32,
) -> Result<Vector<U32, LANES>, LaneError> {
    geometry::low_bits(&mut Emit::<LANES>(lanes), value, bits)
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

/// Where in `DENSE` the brush's dispatch count lies: the three words right
/// after the claim count at the dump base, which no list reaches and which
/// `Plan::sizes` always holds.
#[must_use]
pub fn counted_at(plan: &crate::compact::Plan) -> u32 {
    plan.dump().saturating_add(1)
}

/// The brush's dispatch in the engine's frame. The count is the one
/// [`reckon`] writes earlier in the same frame, so a brush that paints
/// nothing is dispatched with no group at all.
#[must_use]
pub fn seats(plan: &crate::compact::Plan) -> crate::Groups {
    crate::Groups::From {
        binding: crate::layout::DENSE as usize,
        at: counted_at(plan) as usize,
    }
}

/// Writes the brush's dispatch count at `at` in `DENSE`: `SEATS / subgroup`
/// groups while the brush has reach and pours something, and none while its
/// reach is 0 or it pours `AIR`. Those are the two words every seat of the
/// brush tests before it paints or claims a cell, so a brush they stop runs
/// no seat and leaves the world as it was.
pub fn reckon(kernel: &mut Kernel<U32>, subgroup: u32, at: u32) -> Result<(), LaneError> {
    let word = |kernel: &mut Kernel<U32>, part: usize| -> Result<Id, LaneError> {
        let slot = kernel
            .module()
            .constant_u32(u32::try_from(BRUSH + part).unwrap_or(0))?;
        kernel.load_at(PARAMS, slot)
    };
    let how_wide = word(kernel, 2)?;
    let what_of = word(kernel, 3)?;

    let groups = {
        let mut lanes = kernel.lanes()?;
        let reach = lanes.splat_id::<U32, 1>(how_wide)?;
        let material = lanes.splat_id::<U32, 1>(what_of)?;
        let none = lanes.splat_bits::<U32, 1>(0)?;
        let every = lanes.splat_bits::<U32, 1>(BOX * BOX / subgroup)?;
        let wide = lanes.greater_than(reach, none)?;
        let pouring = lanes.greater_than(material, none)?;
        let some = lanes.select(wide, every, none)?;
        lanes.select(pouring, some, none)?
    };

    let one = kernel.module().constant_u32(1)?;
    for (step, value) in [(0, groups.id()), (1, one), (2, one)] {
        let slot = kernel.module().constant_u32(at.saturating_add(step))?;
        kernel.store_at(crate::layout::DENSE, slot, value)?;
    }
    Ok(())
}

pub fn brush(subgroup: u32, grid: Grid, waking: bool) -> Result<Vec<u32>, LaneError> {
    flat("sand::brush", grid)?;
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
        let bx = lanes.splat_id::<U32, LANES>(where_x)?;
        let by_row = lanes.splat_id::<U32, LANES>(where_y)?;
        let mut arith = Emit::<LANES>(&mut lanes);
        let point = Plane::seated(&mut arith, grid, BOX, within, [bx, by_row])?;
        let cell = Plane::at(&mut arith, Plane::extents(grid), point)?;
        let [column, row] = point;
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

        let air = lanes.splat_bits::<U32, LANES>(AIR)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let inside = {
            let mut arith = Emit::<LANES>(&mut lanes);
            let apart = Plane::apart(&mut arith, [column, row], [bx, by_row])?;
            Plane::inside(&mut arith, apart, reach)?
        };
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

        let reaching = Reaching {
            grid,
            seat,
            column,
            row,
            brush: [where_x, where_y, how_wide, what_of],
        };
        claim::<LANES>(&mut kernel, subgroup, reaching)?;
    }

    kernel.finish()
}

struct Reaching<const LANES: u32> {
    grid: Grid,
    seat: Id,
    column: Vector<U32, LANES>,
    row: Vector<U32, LANES>,
    brush: [Id; 4],
}

/// Whether the brush can paint the cell at `column`, `row`: inside its box,
/// inside its disc, and pouring something. The emptiness of the cell is left
/// out, so a quad is claimed whenever the brush reaches it.
fn reaches<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    grid: Grid,
    column: Vector<U32, LANES>,
    row: Vector<U32, LANES>,
    brush: [Id; 4],
) -> Result<Vector<U32, LANES>, LaneError> {
    let [where_x, where_y, how_wide, what_of] = brush;
    let column = low_bits::<LANES>(lanes, column, grid.wide.trailing_zeros())?;
    let row = low_bits::<LANES>(lanes, row, grid.deep.trailing_zeros())?;

    let bx = lanes.splat_id::<U32, LANES>(where_x)?;
    let by_row = lanes.splat_id::<U32, LANES>(where_y)?;
    let reach = lanes.splat_id::<U32, LANES>(how_wide)?;
    let material = lanes.splat_id::<U32, LANES>(what_of)?;
    let half = lanes.splat_bits::<U32, LANES>(BOX / 2)?;
    let side = lanes.splat_bits::<U32, LANES>(BOX)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    let (boxed, inside) = {
        let mut arith = Emit::<LANES>(lanes);
        let apart = Plane::apart(&mut arith, [column, row], [bx, by_row])?;
        let boxed = Plane::boxed(&mut arith, grid, apart, [half, side])?;
        (boxed, Plane::inside(&mut arith, apart, reach)?)
    };
    let pouring = lanes.greater_than(material, none)?;

    let mut all = one;
    for test in boxed.into_iter().chain([inside, pouring]) {
        let held = lanes.select(test, one, none)?;
        all = lanes.mul(all, held)?;
    }
    Ok(all)
}

/// Puts on this frame's list every quad of this frame's phase that the brush
/// reaches and that the list does not hold, so that the settle after the
/// brush steps it in the frame it is painted. One seat claims each such
/// quad: the seat of the first corner, in the order the settle reads them,
/// that the brush reaches.
///
/// The list, `DENSE` below `GRAND`, is in ascending order and is searched by
/// halving. The quads claimed go after it, their count in `DENSE` at the
/// dump base, and the settle's dispatch grows by a group for every
/// `subgroup` of them. A seat that claims nothing writes 1 over the last
/// word of the brush's own dispatch count, which already holds 1, so the
/// count this dispatch was read from keeps its value.
fn claim<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    subgroup: u32,
    reaching: Reaching<LANES>,
) -> Result<(), LaneError> {
    use crate::layout::{DENSE, GRAND, INDIRECT};

    let Reaching {
        grid,
        seat,
        column,
        row,
        brush,
    } = reaching;
    let head = kernel.module().constant_u32(0)?;
    let second = kernel.module().constant_u32(1)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let dump = kernel.load_at(PARAMS, second)?;
    let grand = kernel.load_at(GRAND, head)?;

    let (quad, mine) = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let phase = low_bits::<LANES>(&mut lanes, now, 1)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;

        let mut arith = Emit::<LANES>(&mut lanes);
        let Held {
            block,
            index: quad,
            corner,
        } = Plane::holder(&mut arith, grid, [column, row], phase)?;
        let corners = Plane::corners(&mut arith, block, phase)?;

        let mut earlier = none;
        let mut own = none;
        for (index, [x, y]) in (0_u32..).zip(corners) {
            let reached = reaches::<LANES>(&mut lanes, grid, x, y, brush)?;
            let at = lanes.splat_bits::<U32, LANES>(index)?;
            let before = lanes.greater_than(corner, at)?;
            let before = lanes.select(before, one, none)?;
            let here = lanes.equal(corner, at)?;
            let here = lanes.select(here, one, none)?;
            let counted = lanes.mul(reached, before)?;
            earlier = lanes.add(earlier, counted)?;
            let counted = lanes.mul(reached, here)?;
            own = lanes.add(own, counted)?;
        }

        let within = lanes.splat_id::<U32, LANES>(seat)?;
        let [across_box, down_box] = Plane::seat(&mut Emit::<LANES>(&mut lanes), BOX, within)?;
        let wide = lanes.splat_bits::<U32, LANES>(grid.wide)?;
        let deep = lanes.splat_bits::<U32, LANES>(grid.deep)?;
        let fits_x = lanes.greater_than(wide, across_box)?;
        let fits_x = lanes.select(fits_x, one, none)?;
        let fits_y = lanes.greater_than(deep, down_box)?;
        let fits_y = lanes.select(fits_y, one, none)?;

        let first = lanes.equal(earlier, none)?;
        let first = lanes.select(first, one, none)?;
        let mine = lanes.mul(own, first)?;
        let mine = lanes.mul(mine, fits_x)?;
        let mine = lanes.mul(mine, fits_y)?;
        (quad, mine)
    };

    let steps = 33 - grid.quads().leading_zeros();
    let (mut base, mut left) = {
        let mut lanes = kernel.lanes()?;
        (
            lanes.splat_bits::<U32, LANES>(0)?,
            lanes.splat_id::<U32, LANES>(grand)?,
        )
    };
    for _ in 0..steps {
        let (middle, half) = {
            let mut lanes = kernel.lanes()?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let half = lanes.shift_right_logical(left, one)?;
            (lanes.add(base, half)?, half)
        };
        let seen = kernel.load_at(DENSE, middle.id())?;
        let mut lanes = kernel.lanes()?;
        let seen = lanes.splat_id::<U32, LANES>(seen)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;
        let active = lanes.greater_than(left, none)?;
        let active = lanes.select(active, one, none)?;
        let below = lanes.greater_than(quad, seen)?;
        let below = lanes.select(below, one, none)?;
        let go = lanes.mul(active, below)?;
        let go = lanes.greater_than(go, none)?;

        let past = lanes.add(middle, one)?;
        let taken = lanes.add(half, one)?;
        let taken = lanes.mul(taken, minus)?;
        let rest = lanes.add(left, taken)?;
        base = lanes.select(go, past, base)?;
        left = lanes.select(go, rest, half)?;
    }
    let seen = kernel.load_at(DENSE, base.id())?;

    let (claimed, junk) = {
        let mut lanes = kernel.lanes()?;
        let seen = lanes.splat_id::<U32, LANES>(seen)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let within = lanes.greater_than(total, base)?;
        let within = lanes.select(within, one, none)?;
        let same = lanes.equal(seen, quad)?;
        let same = lanes.select(same, one, none)?;
        let listed = lanes.mul(within, same)?;
        let missing = lanes.equal(listed, none)?;
        let missing = lanes.select(missing, one, none)?;
        let claimed = lanes.mul(mine, missing)?;
        let start = lanes.splat_id::<U32, LANES>(dump)?;
        let last = lanes.splat_bits::<U32, LANES>(3)?;
        (claimed, lanes.add(start, last)?)
    };

    let before = kernel.atomic_add_at(DENSE, dump, claimed.id())?;
    let (into, what, grows) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.splat_id::<U32, LANES>(before)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let at = lanes.add(total, before)?;
        let taking = lanes.greater_than(claimed, none)?;
        let into = lanes.select(taking, at, junk)?;
        let what = lanes.select(taking, quad, one)?;
        let start = low_bits::<LANES>(&mut lanes, at, subgroup.trailing_zeros())?;
        let start = lanes.equal(start, none)?;
        let start = lanes.select(start, one, none)?;
        (into, what, lanes.mul(start, claimed)?)
    };
    kernel.store_at(DENSE, into.id(), what.id())?;
    let _ = kernel.atomic_add_at(INDIRECT, head, grows.id())?;
    Ok(())
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
    flat("sand::paint", grid)?;
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
