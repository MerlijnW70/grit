use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, Predicate, U32, Vector};
use simdr::module::Id;

use crate::geometry::{Emit, Solid};
use crate::layout::{BINDINGS, PARAMS, STATE};
use crate::sand::{AIR, Grid, PATIENCE, WALL, WATER};

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

/// Why [`settle`] or [`Spread::of`] refuses a grid that is not solid.
pub const FLAT: &str = "the grid is flat (tall 1), and the 3D kernel steps a solid grid";

/// Why [`settle`] or [`Spread::of`] refuses a device that dispatches no group.
pub const NO_GROUPS: &str = "the device dispatches no workgroup at all";

/// Why [`settle`] or [`Spread::of`] refuses a grid whose sides are not
/// powers of two of at least two, or whose octs pass a `u32`.
pub const UNEVEN: &str = "a side is not a power of two of at least two, or the octs pass a u32";

const OPERATION: &str = "sand3::settle";

const fn refused(because: &'static str) -> LaneError {
    LaneError::NoSuchForm {
        operation: OPERATION,
        because,
    }
}

/// How [`settle`] covers every oct of a grid: `groups` workgroups of
/// `workgroup` invocations, each stepping `per` octs. Invocation `i` steps
/// octs `i + k * stride` for `k` below `per`, with `stride` the invocations
/// of the whole dispatch, so every oct is stepped once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spread {
    pub workgroup: u32,
    pub per: u32,
    pub groups: u32,
}

impl Spread {
    /// The spread of `grid` at a subgroup of `subgroup` on a device that
    /// dispatches at most `most` groups, as `Host::limits().groups` gives it.
    /// A workgroup is one subgroup, or every oct when there are fewer. Each
    /// invocation takes the fewest octs, a power of two, that keep the
    /// groups at or under `most`.
    pub fn of(grid: Grid, subgroup: u32, most: u32) -> Result<Self, LaneError> {
        if grid.tall < 2 {
            return Err(refused(FLAT));
        }
        let sides = [grid.wide, grid.deep, grid.tall];
        if sides
            .iter()
            .any(|side| *side < 2 || !side.is_power_of_two())
        {
            return Err(refused(UNEVEN));
        }
        let Some(octs) = (grid.wide / 2)
            .checked_mul(grid.deep / 2)
            .and_then(|plane| plane.checked_mul(grid.tall / 2))
        else {
            return Err(refused(UNEVEN));
        };
        if most == 0 {
            return Err(refused(NO_GROUPS));
        }
        if !subgroup.is_power_of_two() {
            return Err(LaneError::BadWidth { width: subgroup });
        }
        let workgroup = subgroup.min(octs);
        let invocations = octs / workgroup;
        let per = invocations.div_ceil(most).next_power_of_two();
        Ok(Self {
            workgroup,
            per,
            groups: invocations / per,
        })
    }

    /// The invocations of the whole dispatch.
    #[must_use]
    pub const fn stride(self) -> u32 {
        self.groups * self.workgroup
    }

    /// The dispatch the kernel is built for.
    #[must_use]
    pub const fn dispatch(self) -> crate::Groups {
        crate::Groups::Fixed(self.groups)
    }
}

/// A built [`settle`] kernel and the spread it was built for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whole {
    pub words: Vec<u32>,
    pub spread: Spread,
}

/// The kernel that steps every oct of a solid grid by one frame of
/// [`settle_oct`], the frame read from `PARAMS[0]`: its phase from bit 0 and
/// its axis order from [`x_first`]. Bit for bit it is [`settle_reference3`].
/// `most` is the most groups the device dispatches; the kernel is built so
/// that its [`Spread::dispatch`] stays at or under it.
pub fn settle(subgroup: u32, grid: Grid, most: u32) -> Result<Whole, LaneError> {
    let spread = Spread::of(grid, subgroup, most)?;
    let words = match subgroup {
        4 => whole::<4>(subgroup, grid, spread),
        8 => whole::<8>(subgroup, grid, spread),
        16 => whole::<16>(subgroup, grid, spread),
        32 => whole::<32>(subgroup, grid, spread),
        64 => whole::<64>(subgroup, grid, spread),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }?;
    Ok(Whole { words, spread })
}

fn whole<const LANES: u32>(
    subgroup: u32,
    grid: Grid,
    spread: Spread,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, spread.workgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(spread.workgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let first = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;

    if spread.per == 1 {
        step::<LANES>(&mut kernel, grid, first, frame)?;
    } else {
        let stride = kernel.module().constant_u32(spread.stride())?;
        let _ = kernel.repeat_rolled(spread.per, uint, first, |kernel, oct, _| {
            step::<LANES>(kernel, grid, oct, frame)?;
            Ok(kernel.module().i_add(uint, oct, stride)?)
        })?;
    }
    kernel.finish()
}

/// Where each corner of [`Grid3::corners`] lies among the corners of
/// [`Solid::corners`], whose bits run x, y, z where the reference's run
/// x, z, y.
const REFERENCE: [usize; CORNERS] = [0, 1, 4, 5, 2, 3, 6, 7];

/// The corners the two axis orders trade: those whose x and z bits differ.
const TRADED: [(usize, usize); 2] = [(1, 2), (5, 6)];

type Word<const LANES: u32> = Vector<U32, LANES>;
type Truth<const LANES: u32> = Predicate<LANES>;

/// Steps oct `oct` of `grid` at frame `frame`.
///
/// A frame that takes z first is stepped as its mirror across the plane
/// x = z: the corners whose x and z differ trade places, a running word
/// trades its x and z ways, the x-first rule runs, and the ways trade back.
/// The rule treats x and z alike but for their order, so the mirror of an
/// x-first step is the z-first step.
fn step<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    oct: Id,
    frame: Id,
) -> Result<(), LaneError> {
    let places = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(oct)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let solid = Solid::block(&mut Emit::<LANES>(&mut lanes), grid, which, now)?.places;
        let mut places = [which; CORNERS];
        for (place, from) in places.iter_mut().zip(REFERENCE) {
            if let Some(found) = solid.get(from) {
                *place = *found;
            }
        }
        let mirrored = z_first::<LANES>(&mut lanes, now)?;
        for (one, other) in TRADED {
            let (Some(here), Some(there)) = (word_at(&places, one), word_at(&places, other)) else {
                continue;
            };
            put_word(&mut places, one, lanes.select(mirrored, there, here)?);
            put_word(&mut places, other, lanes.select(mirrored, here, there)?);
        }
        places
    };

    let mut held = [frame; CORNERS];
    for (word, place) in held.iter_mut().zip(&places) {
        *word = kernel.load_at(STATE, place.id())?;
    }

    let settled = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;
        let mirrored = z_first::<LANES>(&mut lanes, now)?;
        let mut cells = [now; CORNERS];
        for (cell, word) in cells.iter_mut().zip(held) {
            let loaded = lanes.splat_id::<U32, LANES>(word)?;
            *cell = traded::<LANES>(&mut lanes, loaded, mirrored)?;
        }
        let stepped = rule::<LANES>(&mut lanes, cells)?;
        let mut out = stepped;
        for (cell, from) in out.iter_mut().zip(stepped) {
            *cell = traded::<LANES>(&mut lanes, from, mirrored)?;
        }
        out
    };

    for (place, value) in places.iter().zip(&settled) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    Ok(())
}

fn word_at<const LANES: u32>(cells: &[Word<LANES>; CORNERS], at: usize) -> Option<Word<LANES>> {
    cells.get(at).copied()
}

fn put_word<const LANES: u32>(cells: &mut [Word<LANES>; CORNERS], at: usize, value: Word<LANES>) {
    if let Some(slot) = cells.get_mut(at) {
        *slot = value;
    }
}

/// Whether the frame `now` takes z before x: bit 1 of the frame, as
/// [`x_first`] reads it.
fn z_first<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    now: Word<LANES>,
) -> Result<Truth<LANES>, LaneError> {
    let two = lanes.splat_bits::<U32, LANES>(2)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;
    let order = lanes.and(now, two)?;
    lanes.not_equal(order, none)
}

/// `word` with its x and z ways traded where `mirrored` holds. A running
/// field whose two low bits differ names a way, and its high bit picks z
/// over x; the other running fields name no way and stay.
fn traded<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    word: Word<LANES>,
    mirrored: Truth<LANES>,
) -> Result<Word<LANES>, LaneError> {
    let low = lanes.splat_bits::<U32, LANES>(MATERIAL_BITS)?;
    let high = lanes.splat_bits::<U32, LANES>(MATERIAL_BITS + 1)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let across = lanes.splat_bits::<U32, LANES>(MATERIAL_BITS + 2)?;
    let first = lanes.shift_right_logical(word, low)?;
    let second = lanes.shift_right_logical(word, high)?;
    let differ = lanes.xor(first, second)?;
    let differ = lanes.and(differ, one)?;
    let flip = lanes.shift_left(differ, across)?;
    let flipped = lanes.xor(word, flip)?;
    lanes.select(mirrored, flipped, word)
}

const WORN: u32 = ((1 << TIRED_BITS) - 1) * TIRED;

struct Fields<const LANES: u32> {
    one: Word<LANES>,
    none: Word<LANES>,
}

impl<const LANES: u32> Fields<LANES> {
    fn new(lanes: &mut Lanes<'_>) -> Result<Self, LaneError> {
        Ok(Self {
            one: lanes.splat_bits::<U32, LANES>(1)?,
            none: lanes.splat_bits::<U32, LANES>(0)?,
        })
    }

    fn masked(
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
        mask: u32,
    ) -> Result<Word<LANES>, LaneError> {
        let mask = lanes.splat_bits::<U32, LANES>(mask)?;
        lanes.and(word, mask)
    }

    fn field(
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
        from: u32,
        bits: u32,
    ) -> Result<Word<LANES>, LaneError> {
        let by = lanes.splat_bits::<U32, LANES>(from)?;
        let above = lanes.shift_right_logical(word, by)?;
        Self::masked(lanes, above, (1 << bits) - 1)
    }

    fn is(
        &self,
        lanes: &mut Lanes<'_>,
        value: Word<LANES>,
        want: u32,
    ) -> Result<Word<LANES>, LaneError> {
        let want = lanes.splat_bits::<U32, LANES>(want)?;
        let same = lanes.equal(value, want)?;
        lanes.select(same, self.one, self.none)
    }

    fn is_material(
        &self,
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
        want: u32,
    ) -> Result<Word<LANES>, LaneError> {
        let kind = Self::masked(lanes, word, (1 << MATERIAL_BITS) - 1)?;
        self.is(lanes, kind, want)
    }

    fn truth(&self, lanes: &mut Lanes<'_>, flag: Word<LANES>) -> Result<Truth<LANES>, LaneError> {
        lanes.not_equal(flag, self.none)
    }

    /// [`hold_of`].
    fn hold(
        &self,
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
        under: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let kept = Self::masked(lanes, word, (HELD - 1) | WORN)?;
        let wet = self.is_material(lanes, word, WATER)?;
        let standing = self.is_material(lanes, under, WATER)?;
        let both = lanes.and(wet, standing)?;
        let both = self.truth(lanes, both)?;
        let bit = lanes.splat_bits::<U32, LANES>(HELD)?;
        let set = lanes.add(kept, bit)?;
        lanes.select(both, set, kept)
    }

    /// Whether `word` is ready to start running, as 1 or 0.
    fn ready(&self, lanes: &mut Lanes<'_>, word: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        let wet = self.is_material(lanes, word, WATER)?;
        let way = Self::field(lanes, word, MATERIAL_BITS, RUNNING_BITS)?;
        let still = self.is(lanes, way, 0)?;
        let held = Self::field(lanes, word, CARRIED_BITS, 1)?;
        let worn = Self::field(lanes, word, CARRIED_BITS + 1, TIRED_BITS)?;
        let patience = lanes.splat_bits::<U32, LANES>(PATIENCE)?;
        let spare = lanes.greater_than(patience, worn)?;
        let spare = lanes.select(spare, self.one, self.none)?;
        let all = lanes.and(wet, still)?;
        let all = lanes.and(all, held)?;
        lanes.and(all, spare)
    }

    /// `word` started running along `way`: one more tired step, held.
    fn started(
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
        way: u32,
    ) -> Result<Word<LANES>, LaneError> {
        let worn = Self::masked(lanes, word, WORN)?;
        let fixed =
            lanes.splat_bits::<U32, LANES>(WATER + (way << MATERIAL_BITS) + HELD + TIRED)?;
        lanes.add(worn, fixed)
    }

    /// `word` stopped: its material, held bit and tired steps.
    fn halted(lanes: &mut Lanes<'_>, word: Word<LANES>) -> Result<Word<LANES>, LaneError> {
        Self::masked(lanes, word, ((1 << MATERIAL_BITS) - 1) | HELD | WORN)
    }

    /// One pair of cells along `axis`, the lower first.
    fn along(
        &self,
        lanes: &mut Lanes<'_>,
        axis: Axis,
        [here, there]: [Word<LANES>; 2],
    ) -> Result<[Word<LANES>; 2], LaneError> {
        let going_here = Self::field(lanes, here, MATERIAL_BITS, RUNNING_BITS)?;
        let going_there = Self::field(lanes, there, MATERIAL_BITS, RUNNING_BITS)?;
        let goes_up = self.is(lanes, going_here, axis.plus())?;
        let goes_down = self.is(lanes, going_there, axis.minus())?;
        let up_free = self.is_material(lanes, there, AIR)?;
        let down_free = self.is_material(lanes, here, AIR)?;
        let up_shut = lanes.xor(up_free, self.one)?;
        let down_shut = lanes.xor(down_free, self.one)?;

        let stops_here = lanes.and(goes_up, up_shut)?;
        let stops_here = self.truth(lanes, stops_here)?;
        let stops_there = lanes.and(goes_down, down_shut)?;
        let stops_there = self.truth(lanes, stops_there)?;
        let stilled_here = Self::halted(lanes, here)?;
        let stilled_there = Self::halted(lanes, there)?;
        let kept_here = lanes.select(stops_here, stilled_here, here)?;
        let kept_there = lanes.select(stops_there, stilled_there, there)?;

        let rises = lanes.and(goes_up, up_free)?;
        let sinks = lanes.and(goes_down, down_free)?;
        let swapping = lanes.or(rises, sinks)?;
        let swapping = self.truth(lanes, swapping)?;
        Ok([
            lanes.select(swapping, kept_there, kept_here)?,
            lanes.select(swapping, kept_here, kept_there)?,
        ])
    }
}

/// [`settle_oct`] with x first, on words in the order of [`Grid3::corners`].
fn rule<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    oct: [Word<LANES>; CORNERS],
) -> Result<[Word<LANES>; CORNERS], LaneError> {
    let fields = Fields::<LANES>::new(lanes)?;
    let axes = [Axis::X, Axis::Z];
    let mut cells = oct;

    for top in UPPER {
        let (Some(word), Some(under)) = (word_at(&cells, top), word_at(&cells, top + LOWER)) else {
            continue;
        };
        put_word(&mut cells, top, fields.hold(lanes, word, under)?);
    }

    let before = cells;
    for (at, word) in before.iter().enumerate() {
        let ready = fields.ready(lanes, *word)?;
        let mut open = Vec::with_capacity(axes.len());
        for axis in axes {
            let Some(other) = word_at(&before, at ^ axis.bit()) else {
                continue;
            };
            let free = fields.is_material(lanes, other, AIR)?;
            let way = if at & axis.bit() == 0 {
                axis.plus()
            } else {
                axis.minus()
            };
            open.push((free, Fields::<LANES>::started(lanes, *word, way)?));
        }
        let mut next = *word;
        for (free, started) in open.into_iter().rev() {
            let goes = lanes.and(ready, free)?;
            let goes = fields.truth(lanes, goes)?;
            next = lanes.select(goes, started, next)?;
        }
        put_word(&mut cells, at, next);
    }

    for axis in axes {
        for low in (0..CORNERS).filter(|low| low & axis.bit() == 0) {
            let high = low | axis.bit();
            let (Some(here), Some(there)) = (word_at(&cells, low), word_at(&cells, high)) else {
                continue;
            };
            let [here, there] = fields.along(lanes, axis, [here, there])?;
            put_word(&mut cells, low, here);
            put_word(&mut cells, high, there);
        }
    }

    let straight = UPPER.map(|top| (top, top + LOWER));
    let edges = axes.map(|axis| UPPER.map(|top| (top, LOWER + (top ^ axis.bit()))));
    let corner = UPPER.map(|top| (top, LOWER + (top ^ CORNER)));
    let falls = straight
        .into_iter()
        .chain(edges.into_iter().flatten())
        .chain(corner);
    for (upper, lower) in falls {
        let (Some(up), Some(low)) = (word_at(&cells, upper), word_at(&cells, lower)) else {
            continue;
        };
        let (up, low) = crate::sand::fall::<LANES>(lanes, up, low)?;
        put_word(&mut cells, upper, up);
        put_word(&mut cells, lower, low);
    }

    Ok(cells)
}
