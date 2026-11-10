use simdr::lanes::{LaneError, Lanes, Predicate, U32};

use super::{CORNERS, Oct, Rule, Ruled, Word};
use crate::sand::{AIR, PATIENCE, SAND, WALL, WATER};

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

/// One step of one oct, its corners laid out as [`crate::sand3::Grid3::corners`] gives them.
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

/// The corners the two axis orders trade: those whose x and z bits differ.
const TRADED: [(usize, usize); 2] = [(1, 2), (5, 6)];

type Truth<const LANES: u32> = Predicate<LANES>;

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

/// [`settle_oct`] with x first, on words in the order of [`crate::sand3::Grid3::corners`]:
/// the words after the held step, and after the whole rule.
fn rule<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    oct: [Word<LANES>; CORNERS],
) -> Result<([Word<LANES>; CORNERS], [Word<LANES>; CORNERS]), LaneError> {
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

    Ok((before, cells))
}

/// Sand and water: [`settle_oct`] as a rule of the 3D engine. It reads no
/// parameters and no noise. Its tally words are the cells that moved past
/// the held step, the water cells and the sand cells after the step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sand;

/// The cells of `out` whose material is `want`.
fn of_material(out: &[u32; CORNERS], want: u32) -> u32 {
    let mut count = 0;
    for &word in out {
        count += u32::from(material_of(word) == want);
    }
    count
}

impl Rule for Sand {
    fn params(&self) -> usize {
        0
    }

    fn step(&self, oct: Oct<'_, u32>) -> Ruled<u32> {
        let mut held = oct.words;
        for top in UPPER {
            let (word, under) = (read(&held, top), read(&held, top + LOWER));
            put(&mut held, top, hold_of(word, under));
        }
        let out = settle_oct(oct.words, x_first(oct.frame));
        let mut moved = 0;
        for (before, after) in held.iter().zip(&out) {
            moved += u32::from(before != after);
        }
        Ruled {
            out,
            counts: [moved, of_material(&out, WATER), of_material(&out, SAND)],
        }
    }

    /// A frame that takes z first is stepped as its mirror across the plane
    /// x = z: the words whose x and z differ trade places, a running word
    /// trades its x and z ways, the x-first rule runs, and both trade back.
    /// The rule treats x and z alike but for their order, so the mirror of an
    /// x-first step is the z-first step.
    fn emit<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        oct: Oct<'_, Word<LANES>>,
    ) -> Result<Ruled<Word<LANES>>, LaneError> {
        let mirrored = z_first::<LANES>(lanes, oct.frame)?;
        let mut cells = traded_places::<LANES>(lanes, oct.words, mirrored)?;
        for cell in &mut cells {
            *cell = traded::<LANES>(lanes, *cell, mirrored)?;
        }
        let (held, ruled) = rule::<LANES>(lanes, cells)?;
        let mut back = ruled;
        for cell in &mut back {
            *cell = traded::<LANES>(lanes, *cell, mirrored)?;
        }
        let out = traded_places::<LANES>(lanes, back, mirrored)?;

        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let mut moved = none;
        for (before, after) in held.into_iter().zip(ruled) {
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            moved = lanes.add(moved, differs)?;
        }
        let water = lanes.splat_bits::<U32, LANES>(WATER)?;
        let grain = lanes.splat_bits::<U32, LANES>(SAND)?;
        let mut wet = none;
        let mut dry = none;
        for cell in out {
            let kind = crate::sand::material::<LANES>(lanes, cell)?;
            let is_wet = lanes.equal(kind, water)?;
            let is_dry = lanes.equal(kind, grain)?;
            let add_wet = lanes.select(is_wet, one, none)?;
            let add_dry = lanes.select(is_dry, one, none)?;
            wet = lanes.add(wet, add_wet)?;
            dry = lanes.add(dry, add_dry)?;
        }
        Ok(Ruled {
            out,
            counts: [moved, wet, dry],
        })
    }
}

/// `cells` with the corners of [`TRADED`] swapped where `mirrored` holds.
fn traded_places<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cells: [Word<LANES>; CORNERS],
    mirrored: Truth<LANES>,
) -> Result<[Word<LANES>; CORNERS], LaneError> {
    let mut out = cells;
    for (one, other) in TRADED {
        let (Some(here), Some(there)) = (word_at(&cells, one), word_at(&cells, other)) else {
            continue;
        };
        put_word(&mut out, one, lanes.select(mirrored, there, here)?);
        put_word(&mut out, other, lanes.select(mirrored, here, there)?);
    }
    Ok(out)
}
