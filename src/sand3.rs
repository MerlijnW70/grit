use crate::sand::{AIR, PATIENCE, WALL, WATER};

const MATERIAL_BITS: u32 = 2;
const RUNNING_BITS: u32 = 3;
const CARRIED_BITS: u32 = MATERIAL_BITS + RUNNING_BITS;
const TIRED_BITS: u32 = 3;

pub const HELD: u32 = 1 << CARRIED_BITS;
pub const TIRED: u32 = HELD << 1;

pub const MINUS_X: u32 = 1;
pub const PLUS_X: u32 = 2;
pub const MINUS_Z: u32 = 5;
pub const PLUS_Z: u32 = 6;

pub const RUNNING_MINUS_X: u32 = WATER + (MINUS_X << MATERIAL_BITS);
pub const RUNNING_PLUS_X: u32 = WATER + (PLUS_X << MATERIAL_BITS);
pub const RUNNING_MINUS_Z: u32 = WATER + (MINUS_Z << MATERIAL_BITS);
pub const RUNNING_PLUS_Z: u32 = WATER + (PLUS_Z << MATERIAL_BITS);

pub const CORNERS: usize = 8;

#[must_use]
pub const fn material_of(word: u32) -> u32 {
    word % (1 << MATERIAL_BITS)
}

#[must_use]
pub const fn running_of(word: u32) -> u32 {
    (word >> MATERIAL_BITS) % (1 << RUNNING_BITS)
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

/// A torus of `wide` columns (x), `deep` rows (y) and `tall` layers (z),
/// each a power of two. Rows are the 2D grid's rows: a grain falls from row
/// `y` to row `y + 1`. Cell `(x, y, z)` lies at `(z * deep + y) * wide + x`,
/// so layer zero is laid out as the 2D grid of the same width and depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid3 {
    pub wide: u32,
    pub deep: u32,
    pub tall: u32,
}

impl Grid3 {
    /// The grid with each side rounded up to a power of two, refused when a
    /// side rounds past `2^31`, rounds below two (no whole oct), or the cells
    /// pass a `u32`.
    #[must_use]
    pub const fn new(wide: u32, deep: u32, tall: u32) -> Option<Self> {
        let Some(wide) = wide.checked_next_power_of_two() else {
            return None;
        };
        let Some(deep) = deep.checked_next_power_of_two() else {
            return None;
        };
        let Some(tall) = tall.checked_next_power_of_two() else {
            return None;
        };
        if wide < 2 || deep < 2 || tall < 2 {
            return None;
        }
        let Some(plane) = wide.checked_mul(deep) else {
            return None;
        };
        if plane.checked_mul(tall).is_none() {
            return None;
        }
        Some(Self { wide, deep, tall })
    }

    #[must_use]
    pub const fn cells(self) -> usize {
        self.wide as usize * self.deep as usize * self.tall as usize
    }

    #[must_use]
    pub const fn octs(self) -> u32 {
        (self.wide / 2) * (self.deep / 2) * (self.tall / 2)
    }

    #[must_use]
    pub const fn index(self, x: u32, y: u32, z: u32) -> usize {
        let x = x & (self.wide - 1);
        let y = y & (self.deep - 1);
        let z = z & (self.tall - 1);
        ((z as usize * self.deep as usize) + y as usize) * self.wide as usize + x as usize
    }

    /// The eight cells of oct `oct` in frame `frame`, as cell indices. Octs
    /// are numbered row-major, x fastest. The oct's least corner is twice its
    /// coordinates plus the phase `frame & 1` on all three axes, wrapped.
    /// Corner `k` sits at `dx = k & 1`, `dz = (k >> 1) & 1`, `dy = k >> 2`:
    /// corners 0 to 3 are the upper layer, 4 to 7 the lower one.
    #[must_use]
    pub const fn corners(self, frame: u32, oct: u32) -> [usize; CORNERS] {
        let phase = frame & 1;
        let across = self.wide / 2;
        let down = self.deep / 2;
        let x = (oct % across) * 2 + phase;
        let y = ((oct / across) % down) * 2 + phase;
        let z = (oct / across / down) * 2 + phase;
        [
            self.index(x, y, z),
            self.index(x + 1, y, z),
            self.index(x, y, z + 1),
            self.index(x + 1, y, z + 1),
            self.index(x, y + 1, z),
            self.index(x + 1, y + 1, z),
            self.index(x, y + 1, z + 1),
            self.index(x + 1, y + 1, z + 1),
        ]
    }
}

/// Whether frame `frame` takes the x axis before the z axis. The order holds
/// for two frames, one of each phase, and then flips, so each phase sees both
/// orders and every oct of a frame runs the same rule.
#[must_use]
pub const fn x_first(frame: u32) -> bool {
    (frame >> 1) & 1 == 0
}

#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Z,
}

impl Axis {
    const fn bit(self) -> usize {
        match self {
            Self::X => 1,
            Self::Z => 2,
        }
    }

    const fn minus(self) -> u32 {
        match self {
            Self::X => MINUS_X,
            Self::Z => MINUS_Z,
        }
    }

    const fn plus(self) -> u32 {
        match self {
            Self::X => PLUS_X,
            Self::Z => PLUS_Z,
        }
    }
}

fn read(cells: &[u32; CORNERS], at: usize) -> u32 {
    cells.get(at).copied().unwrap_or(WALL)
}

fn put(cells: &mut [u32; CORNERS], at: usize, value: u32) {
    if let Some(slot) = cells.get_mut(at) {
        *slot = value;
    }
}

const fn halted(word: u32) -> u32 {
    material_of(word) + held_of(word) * HELD + tired_of(word) * TIRED
}

const fn ready(word: u32) -> bool {
    material_of(word) == WATER
        && running_of(word) == 0
        && held_of(word) != 0
        && tired_of(word) < PATIENCE
}

const fn started(word: u32, way: u32) -> u32 {
    WATER + (way << MATERIAL_BITS) + HELD + (tired_of(word) + 1) * TIRED
}

fn along(cells: &mut [u32; CORNERS], axis: Axis, low: usize, high: usize) {
    let (here, there) = (read(cells, low), read(cells, high));
    let goes_up = running_of(here) == axis.plus();
    let goes_down = running_of(there) == axis.minus();
    let up_free = material_of(there) == AIR;
    let down_free = material_of(here) == AIR;

    let kept_here = if goes_up && !up_free {
        halted(here)
    } else {
        here
    };
    let kept_there = if goes_down && !down_free {
        halted(there)
    } else {
        there
    };
    if (goes_up && up_free) || (goes_down && down_free) {
        put(cells, low, kept_there);
        put(cells, high, kept_here);
    } else {
        put(cells, low, kept_here);
        put(cells, high, kept_there);
    }
}

fn fall(cells: &mut [u32; CORNERS], upper: usize, lower: usize) {
    let (up, low) = (read(cells, upper), read(cells, lower));
    let (above, below) = (material_of(up), material_of(low));
    if above > below && above != WALL {
        put(cells, upper, low);
        put(cells, lower, above);
    }
}

const UPPER: [usize; 4] = [0, 1, 2, 3];
const LOWER: usize = 4;
const CORNER: usize = 3;

/// One step of one oct, its corners laid out as [`Grid3::corners`] gives them.
///
/// In order: the four upper cells take their held bit from the cell under
/// them; held idle water starts to run toward an empty neighbour in its layer
/// along the first axis, else the second; runners swap with an empty
/// neighbour along the first axis and then the second; the four vertical
/// pairs fall; then the four upper cells fall along an edge diagonal of the
/// first axis, then of the second axis, then along the corner diagonal. Each
/// of these stages works on four disjoint pairs, so only the axis order can
/// change an outcome, and `x_first` sets it.
#[must_use]
pub fn settle_oct(oct: [u32; CORNERS], x_first: bool) -> [u32; CORNERS] {
    let axes = if x_first {
        [Axis::X, Axis::Z]
    } else {
        [Axis::Z, Axis::X]
    };
    let mut cells = oct;

    for top in UPPER {
        let (word, under) = (read(&cells, top), read(&cells, top + LOWER));
        put(&mut cells, top, hold_of(word, under));
    }

    let before = cells;
    for at in 0..CORNERS {
        let word = read(&before, at);
        if !ready(word) {
            continue;
        }
        for axis in axes {
            let other = at ^ axis.bit();
            if material_of(read(&before, other)) == AIR {
                let way = if at & axis.bit() == 0 {
                    axis.plus()
                } else {
                    axis.minus()
                };
                put(&mut cells, at, started(word, way));
                break;
            }
        }
    }

    for axis in axes {
        for low in 0..CORNERS {
            if low & axis.bit() == 0 {
                along(&mut cells, axis, low, low | axis.bit());
            }
        }
    }

    for top in UPPER {
        fall(&mut cells, top, top + LOWER);
    }
    for axis in axes {
        for top in UPPER {
            fall(&mut cells, top, LOWER + (top ^ axis.bit()));
        }
    }
    for top in UPPER {
        fall(&mut cells, top, LOWER + (top ^ CORNER));
    }

    cells
}

/// One frame of the whole torus on the CPU: every oct of the frame's phase
/// steps once with [`settle_oct`]. A cell index past `cells` reads as a wall
/// and is never written.
#[must_use]
pub fn settle_reference3(grid: Grid3, frame: u32, cells: &[u32]) -> Vec<u32> {
    let mut out = cells.to_vec();
    let first = x_first(frame);
    for oct in 0..grid.octs() {
        let places = grid.corners(frame, oct);
        let mut block = [WALL; CORNERS];
        for (slot, place) in block.iter_mut().zip(places) {
            *slot = out.get(place).copied().unwrap_or(WALL);
        }
        let settled = settle_oct(block, first);
        for (place, value) in places.into_iter().zip(settled) {
            if let Some(slot) = out.get_mut(place) {
                *slot = value;
            }
        }
    }
    out
}
