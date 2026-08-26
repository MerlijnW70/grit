//! Falling sand, on a partition where the race cannot happen.
//!
//! # The problem every GPU sand simulation has
//!
//! Two cells want to move into the same empty one. On a CPU the loop order decides; on a GPU
//! nothing does, and the usual answers are atomics, a priority pass, or double buffering with a
//! tie-break — all of which cost more than the physics.
//!
//! This one has no race to resolve, because **no invocation writes outside its own 2×2 block**. The
//! grid is partitioned into blocks, each block is read and rewritten by one invocation, and the
//! partition's origin alternates between frames so material can cross a boundary on the next step.
//! That is the Margolus neighbourhood, and it is the reason a block cellular automaton is what a
//! parallel machine wants: conservation and locality fall out of the arrangement rather than being
//! enforced on top of it.
//!
//! ```text
//!   frame even          frame odd
//!   ┌──┬──┬──┬──┐       ─┬──┬──┬──┬─
//!   │ab│ab│ab│ab│        │ab│ab│ab│
//!   │cd│cd│cd│cd│        │cd│cd│cd│
//!   ├──┼──┼──┼──┤       ─┼──┼──┼──┼─
//! ```
//!
//! # The rule, as one memory and six conditional swaps
//!
//! `a b` on top, `c d` below. Applied in order, and the order matters — a later rule sees what an
//! earlier one did:
//!
//! 0. [`hold`]   the top row writes down what it is standing on, which only this block can see
//! 1. [`start`]  still water that remembers standing on water, with air along its row, takes it
//! 2. [`course`] whatever is running goes one cell that way, or stops against what is in the way
//! 3. [`fall`]   heavier above lighter swaps, straight down, each column
//! 4. the same again for the other column
//! 5. [`fall`]   the slump: still blocked below, into the free diagonal
//! 6. and its mirror
//!
//! Three and four give a straight fall; five and six give a pile that slumps instead of standing in
//! columns; nought through two are what a fluid needs and a solid does not. Every one is a
//! comparison and a `select`, because `decisions/DR-0003` refuses the branch a sand simulation is
//! otherwise written with.
//!
//! The order is not a preference. Put [`start`] after the falls and the diagonal slump has already
//! taken every configuration it looks for, so the rule becomes dead code that compiles. Put
//! [`hold`] after [`start`] and the memory it reads is always a frame older than it needs to be.
//!
//! # Everything a cell needs is in its own word
//!
//! There is no lookup table in the lane API, so a material's properties have to *be* its number:
//! see [`WEIGHTS`] for why that makes [`fall`] one comparison rather than one per pair of materials,
//! [`RUNNING_LEFT`] for how a fluid's direction fits in the same word without disturbing it, and
//! [`HELD`] for the one bit that is there because the alternative was a race.
//!
//! # No conditional writes, and nothing read from outside
//!
//! Every cell this pass reads, it writes; every cell it writes, it read. The four stores are
//! unconditional and land where the four loads came from, so there is nothing to skip and nothing to
//! skip it with — and, more to the point, **nothing another invocation could be rewriting while this
//! one looks at it**. That is not decoration: [`HELD`] exists because the obvious way to answer
//! [`start`]'s question was to read one row further down, and one row further down belongs to
//! somebody else.
//! block, because [`start`] has a question that cannot be answered inside four cells.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

pub use crate::layout::{BINDINGS, BRUSH, BRUSH_WORDS, LOOKING, PIXELS};
use crate::layout::{PARAMS, STATE};

/// Nothing at all: what everything falls into.
pub const AIR: u32 = 0;
/// What falls and spreads out flat.
pub const WATER: u32 = 1;
/// What falls and heaps up.
pub const SAND: u32 = 2;
/// What does not move and nothing moves into.
pub const WALL: u32 = 3;

/// **The numbers are the weights, and that is the whole of the material table.**
///
/// There is no lookup in the lane API — no table, no gather, only arithmetic and selects — so a
/// material's properties have to *be* its number. Ordering them by weight makes [`fall`] one
/// comparison instead of one equality per pair of materials: heavier above lighter swaps, whatever
/// the two are. Sand sinks through water for free, and adding a material after this one costs the
/// rule nothing as long as it is given the right place in the order.
///
/// [`WALL`] is heaviest so nothing can be above it and heavier, and the rule excludes it explicitly
/// so it never sinks itself.
pub const WEIGHTS: [u32; 4] = [AIR, WATER, SAND, WALL];

/// Water that is on its way to the left, and to the right.
///
/// # Why a fluid needs to remember something and sand does not
///
/// [`fall`] gives an angle of repose: a heap slumps into free diagonals and stops at forty-five
/// degrees. Water has no angle, and **no rule that reads only a 2×2 block can give it one**, because
/// going flat means moving material sideways at the same height, and a sideways rule that fires
/// whenever there is room fires just as happily in reverse. A lone drop would step left, then right,
/// for ever; the work list would never empty and the demo's one real claim would go with it.
///
/// The way out is the way actual fluid models take it: a cell that starts moving **keeps** moving.
/// Water under pressure — water with water directly above it and air beside it — takes a direction
/// and runs that way one cell a frame until something stops it. Running is not symmetric, so it does
/// not undo itself, and it ends: a runner stops when it is blocked and starts only where something
/// is about to fall.
///
/// # The word holds both
///
/// A second buffer would be a second binding and a second pass over the grid, so the direction lives
/// in the same word as the material, two bits up:
///
/// ```text
///   word    = material + 4 × direction
///   material = word mod 4        ...  which is `low_bits(word, 2)`
///   direction = word ÷ 4         ...  which is a shift
/// ```
///
/// So every comparison in [`fall`] is on the *material*, and a running grain of water weighs exactly
/// what a still one does — which is what stops the two sinking through each other.
pub const RUNNING_LEFT: u32 = WATER + 4;
/// Water on its way to the right. See [`RUNNING_LEFT`].
pub const RUNNING_RIGHT: u32 = WATER + 8;

/// How many bits of a cell's word are its material.
const MATERIAL_BITS: u32 = 2;
/// How many bits are the material and the direction together.
const CARRIED_BITS: u32 = 4;
/// The bit a cell sets to remember it was standing on water.
///
/// # Why a cell has to remember, when it could just look
///
/// [`start`] needs to know whether the cell below is water. For the top row of a block that is `c`
/// or `d` and the question is free. For the bottom row the answer is one row further down, in
/// another invocation's block — **and reading it is a race**, because that block is being rewritten
/// at the same moment. Caught in exactly that form: `tests/live.rs` disagreed at frame nought on an
/// eight-wide part and agreed on a thirty-two-wide one.
///
/// So the answer is written down instead, by the one invocation entitled to write it. A cell is the
/// top row of a block on one phase and the bottom row on the other, so every cell has this bit
/// refreshed from the truth every second frame and reads its own copy on the frames in between. One
/// frame stale, never wrong about whose cell it is.
///
/// # Why the top row alone was not enough
///
/// Because a bank at forty-five degrees keeps `column − row` constant along its whole length, and a
/// Margolus block only covers a pair whose column and row parities agree. Measured: water poured
/// into a basin levelled out to two deep across sixty columns and left a perfect ten-wide pyramid in
/// the middle that never moved again, on either flank, for four thousand frames.
pub const HELD: u32 = 1 << CARRIED_BITS;

/// How many bits count the times a cell has taken a direction and got nowhere.
const TIRED_BITS: u32 = 3;

/// One step of that counter: the bit above [`HELD`].
///
/// # Why a fluid needs to be able to give up
///
/// [`start`] fires on the one thing a 2×2 block can see — water directly underneath — and that
/// stands in for "there is somewhere lower to go" almost everywhere. Where it fails is the
/// remainder: a hundred and twenty-eight cells do not divide into sixty-two columns, and the four
/// left over sit on a level surface with air beside them, which is indistinguishable from water that
/// *should* flow. So they take a direction, run until something stops them, and are handed the same
/// direction again next frame. For ever.
///
/// **That is not a cosmetic fault.** The claim this repository exists to make is that the work list
/// reaches nought when nothing is happening, and a few dozen quads kept awake by cells pacing about
/// is that claim being false in the only case anyone would check.
///
/// No rule reading one block can tell those four cells from the rest — whether there is somewhere
/// lower is a property of the whole surface. What a rule *can* see is its own history, and this is
/// that history in three bits: how many times this cell has been given a direction without ever
/// descending. At [`PATIENCE`] it stops asking, the word stops changing, and the quad goes to sleep.
///
/// # What clears it
///
/// [`fall`] does, and nothing else has to: *what lands is bare*. A cell that moves down has got
/// somewhere, which is the whole of what the counter was measuring the absence of — so the rule that
/// strips a landing cell's direction and memory strips this with them, and it was already written
/// that way before there was anything here to strip.
pub const TIRED: u32 = HELD << 1;

/// How many times a cell may take a direction before it stops asking.
///
/// The counter saturates rather than wraps, and it does so by never being incremented past this:
/// [`start`] refuses a cell that has reached it, so the add below it can only ever see a value one
/// short. A counter that wrapped would make a cell fresh again and the pacing would resume.
///
/// # What the number was chosen from
///
/// Too small and a cell gives up while the surface under it is still moving, which is not a fluid
/// that has come to rest but one that has been stopped mid-level. Measured on the basin in
/// `tests/sand.rs` — a hundred and twenty-eight cells poured into sixty-two columns:
///
/// ```text
///   bits  patience   water lies      still running
///      1         1   1 to 7 deep                 0     not level, and the test says so
///      2         3   2 to 3 deep                 0
///      3         7   2 to 3 deep                 0
/// ```
///
/// So **two bits is the measured minimum and three is what is here**, because what the third buys is
/// headroom that nothing measures: a cell restarts when something stops it, and how often that
/// happens over a basin sixty-two columns wide says little about one four thousand wide. The bit is
/// there to be had — two of material, two of direction, one of memory and three of this is **eight
/// of the word's thirty-two** — and being stopped mid-level is the failure that a picture does not
/// show and this test does.
pub const PATIENCE: u32 = (1 << TIRED_BITS) - 1;

/// What [`running`] gives back for [`RUNNING_LEFT`]. Nought is water going nowhere.
const LEFTWARD: u32 = 1;
/// What [`running`] gives back for [`RUNNING_RIGHT`].
const RIGHTWARD: u32 = 2;

/// The material a word holds, with any direction taken off it.
///
/// The mask is [`low_bits`], which is how a crate with no bitwise and spells one.
pub fn material<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    word: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    low_bits::<LANES>(lanes, word, MATERIAL_BITS)
}

/// Which way a word is running: nought for still, one for leftward, two for rightward.
pub fn running<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    word: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(MATERIAL_BITS)?;
    let above = lanes.shift_right_logical(word, by)?;
    low_bits::<LANES>(lanes, above, MATERIAL_BITS)
}

/// Refresh a cell's [`HELD`] bit from what is actually under it.
///
/// Called for the top row of a block, where `under` is the cell below inside the same block. The old
/// bit is dropped by keeping only the material and the direction, which is a mask and therefore a
/// [`low_bits`]; the new one is a multiplication of two zero-or-ones, because a `Predicate` is not a
/// value this API combines.
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

    // **And the fatigue rides through.** This rebuilds the word from its low bits up, so anything
    // above the memory has to be put back by hand or a cell would arrive fresh every second frame —
    // which is exactly how often this is called, and would have made [`TIRED`] count nothing.
    let step = lanes.splat_bits::<U32, LANES>(TIRED)?;
    let worn = tired::<LANES>(lanes, cell)?;
    let worn = lanes.mul(worn, step)?;
    lanes.add(kept, worn)
}

/// Whether a cell remembers standing on water.
///
/// **One bit, and it has to say so.** This used to be the whole of the word above the direction,
/// which was the same thing right up until [`TIRED`] went in above it — and then every cell that had
/// ever tried to run would have read as standing on water whatever it was standing on.
pub fn held<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cell: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(CARRIED_BITS)?;
    let above = lanes.shift_right_logical(cell, by)?;
    low_bits::<LANES>(lanes, above, 1)
}

/// How many times this cell has taken a direction without ever getting lower. See [`TIRED`].
pub fn tired<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    cell: Vector<U32, LANES>,
) -> Result<Vector<U32, LANES>, LaneError> {
    let by = lanes.splat_bits::<U32, LANES>(CARRIED_BITS + 1)?;
    let above = lanes.shift_right_logical(cell, by)?;
    low_bits::<LANES>(lanes, above, TIRED_BITS)
}

/// The same things on the host, so the reference and the kernel cannot drift apart.
#[must_use]
pub const fn material_of(word: u32) -> u32 {
    word % (1 << MATERIAL_BITS)
}

/// The host's [`running`].
#[must_use]
pub const fn running_of(word: u32) -> u32 {
    (word >> MATERIAL_BITS) % (1 << MATERIAL_BITS)
}

/// The host's [`held`]. One bit, for the reason that function gives.
#[must_use]
pub const fn held_of(word: u32) -> u32 {
    (word >> CARRIED_BITS) % 2
}

/// The host's [`tired`].
#[must_use]
pub const fn tired_of(word: u32) -> u32 {
    (word >> (CARRIED_BITS + 1)) % (1 << TIRED_BITS)
}

/// The host's [`hold`].
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

/// How big the world is, in cells.
///
/// **Both sides powers of two**, so every wrap is a shift and a difference — there is no division
/// and none is wanted. A grid that was not would need one at every address, four times a quad.
///
/// Carried rather than compiled in: the kernels are built from Rust at run time, so the size is an
/// argument like the subgroup width is. Sixteen million cells and a hundred and thirty thousand are
/// the same code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// Cells across.
    pub wide: u32,
    /// Cells down.
    pub deep: u32,
}

impl Grid {
    /// A grid of `wide` by `deep`, both of which must be powers of two.
    ///
    /// # Panics
    ///
    /// Never — a size that is not a power of two is rounded up to one, because every address in
    /// this crate is a shift and a grid that broke that would be wrong rather than slow.
    #[must_use]
    pub const fn of(wide: u32, deep: u32) -> Self {
        Self {
            wide: wide.next_power_of_two(),
            deep: deep.next_power_of_two(),
        }
    }

    /// How many cells there are.
    #[must_use]
    pub const fn cells(self) -> usize {
        self.wide as usize * self.deep as usize
    }

    /// Quads across: half the grid.
    #[must_use]
    pub const fn across(self) -> u32 {
        self.wide / 2
    }

    /// Quads down.
    #[must_use]
    pub const fn down(self) -> u32 {
        self.deep / 2
    }

    /// How many 2×2 quads the grid holds.
    #[must_use]
    pub const fn quads(self) -> u32 {
        self.across() * self.down()
    }
}

/// One Margolus step: every block read, its four cells rewritten.
///
/// The partition's origin comes from the frame number in [`PARAMS`], so the host never writes it —
/// the device increments the frame itself and the phase is its low bit.
///
/// # Errors
///
/// [`LaneError::NoMapping`] for a width with no arm, otherwise as the module builder.
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

/// The step, at a width the caller has matched to the device.
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

    // Where this block's four cells are. Every division here is a shift and every remainder a
    // difference, because the grid's sides are powers of two — chosen for that reason.
    let (top_left, top_right, bottom_left, bottom_right) = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let now = lanes.splat_id::<U32, LANES>(frame)?;

        // The phase: the frame's low bit, and the origin the partition sits at this step.
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
        // Four adopted one by one rather than through a collection, so the four names below are
        // four bindings and not four indexings that might not be there.
        let a = lanes.splat_id::<U32, LANES>(held[0])?;
        let b = lanes.splat_id::<U32, LANES>(held[1])?;
        let c = lanes.splat_id::<U32, LANES>(held[2])?;
        let d = lanes.splat_id::<U32, LANES>(held[3])?;

        // **Before the falls, and that is not a preference.** The diagonal slump on the fourth line
        // below takes material from `a` into an empty `d`, so by the time it has run there is no
        // block left in which water has air below and beside it — which is the exact shape `start`
        // looks for. Put after, the rule is dead code that compiles, and water is sand.
        // First the memory, refreshed from what this block can actually see.
        let a = hold::<LANES>(&mut lanes, a, c)?;
        let b = hold::<LANES>(&mut lanes, b, d)?;

        let [a, b, c, d] = start::<LANES>(&mut lanes, [a, b, c, d])?;

        // Then whatever is running goes one along its row, and whatever is blocked stops.
        let (a, b) = course::<LANES>(&mut lanes, a, b)?;
        let (c, d) = course::<LANES>(&mut lanes, c, d)?;

        // Straight down, each column.
        let (a, c) = fall::<LANES>(&mut lanes, a, c)?;
        let (b, d) = fall::<LANES>(&mut lanes, b, d)?;

        // And the slump: material still on top, blocked underneath, into the free diagonal. Three
        // before four, so a block with one free corner and two grains fills it once. Water is
        // refused it — see `down` for what that refusal buys.
        let (a, d) = fall::<LANES>(&mut lanes, a, d)?;
        let (b, c) = fall::<LANES>(&mut lanes, b, c)?;

        [a, b, c, d]
    };

    for (place, value) in places.iter().zip(&settled) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }

    kernel.finish()
}

/// Swap `upper` and `lower` when the first is heavier than the second, and is not a wall.
///
/// The whole rule in one shape, used four times: two straight falls and two slumps. What differs
/// between them is which pair is handed over, which is why there is one of these rather than four.
///
/// **One comparison, not one per pair of materials.** Because [`WEIGHTS`] puts the materials in
/// order, "heavier above lighter" covers sand into air, water into air *and* sand sinking through
/// water without naming any of them. The second condition is the only special case there is: a wall
/// is the heaviest thing here, so without it a wall would sink through everything else.
pub fn fall<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    upper: Vector<U32, LANES>,
    lower: Vector<U32, LANES>,
) -> Result<(Vector<U32, LANES>, Vector<U32, LANES>), LaneError> {
    let wall = lanes.splat_bits::<U32, LANES>(WALL)?;
    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let none = lanes.splat_bits::<U32, LANES>(0)?;

    // On the material, never on the word: a running grain of water must weigh what a still one
    // does, or the two would sink through each other for ever.
    let above = material::<LANES>(lanes, upper)?;
    let below = material::<LANES>(lanes, lower)?;

    // Two conditions, multiplied rather than and-ed: a `Predicate` is not a value the lane API
    // combines, and a product of two zero-or-ones is the conjunction.
    let heavier = lanes.greater_than(above, below)?;
    let movable = lanes.greater_than(wall, above)?;
    let falling = lanes.select(heavier, one, none)?;
    let loose = lanes.select(movable, one, none)?;
    let both = lanes.mul(falling, loose)?;
    let moving = lanes.greater_than(both, none)?;

    // **What lands is bare.** The direction and the memory both go: water that drops off a ledge
    // arrives with nothing left over, and what it is standing on now is a fresh question the next
    // block to own it will ask.
    let upper_next = lanes.select(moving, lower, upper)?;
    let lower_next = lanes.select(moving, above, lower)?;
    Ok((upper_next, lower_next))
}

/// Carry a runner one cell along its row, or stop it if there is no room.
///
/// The two halves are mutually exclusive by construction — a leftward runner in the right seat can
/// only move if the left seat is air, and air is not running anywhere — so both are read off the
/// same two inputs and applied as one swap.
///
/// A runner in the *far* seat of its direction cannot be judged here: the cell it wants is outside
/// the block, and this rule writes only inside one. It is judged next frame, when the partition has
/// shifted and it is in the near seat instead. That is the same argument the whole crate rests on,
/// applied one rule further down.
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

    // Blocked is stopped, and stopping drops the direction and keeps the memory: a runner that has
    // been brought up short is standing on whatever it was standing on a moment ago.
    let bit = lanes.splat_bits::<U32, LANES>(HELD)?;
    let step = lanes.splat_bits::<U32, LANES>(TIRED)?;
    let halt = |lanes: &mut Lanes<'_>, cell: Vector<U32, LANES>, kind: Vector<U32, LANES>| {
        let memory = held::<LANES>(lanes, cell)?;
        let up = lanes.mul(memory, bit)?;
        let stilled = lanes.add(kind, up)?;
        // The fatigue too, and this is the one place it matters most: being brought up short is
        // what the counter is counting, so a halt that dropped it would count nothing at all.
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

/// Give still water that is standing on water a direction to run in.
///
/// # What pressure is, once you have to say it exactly
///
/// "Water that should flow sideways" is water with somewhere lower to go, and no rule reading one
/// 2x2 block can know whether there is somewhere lower - that is a property of the whole surface. So
/// the rule uses the one local thing that stands in for it: **water directly underneath**.
///
/// * Water standing on water is at least two deep, so it is holding something up. It flows.
/// * Water standing on rock is a puddle one cell deep, or a lone drop. It stays, and that is what
///   lets [`crate::live::decay`] take the work list back towards nothing.
///
/// Where the answer comes from is [`HELD`]: refreshed in the block that owns the cell, read on the
/// frames in between. All four cells are asked, because a bank at forty-five degrees hides one whole
/// parity from any rule that needs the pair to be inside the same block.
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
    // Still water, and still means not already running - a cell keeps the direction it has rather
    // than being told a new one every frame.
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

    // **And a fourth condition: that this cell has not already tried and got nowhere.** See
    // [`TIRED`] for what that is for. It is a condition on the cell's own past rather than on its
    // neighbourhood, which is the only kind of thing left for a rule that has read every cell it is
    // allowed to read and still cannot tell a runner from a remainder.
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
        // The direction replaces whatever the word carried, and the memory bit rides along, because
        // a cell that starts running is still standing on the same thing it was.
        let bit = held::<LANES>(lanes, cell)?;
        let up = lanes.splat_bits::<U32, LANES>(HELD)?;
        let kept = lanes.mul(bit, up)?;
        let running = lanes.add(way, kept)?;

        // **One higher, and it cannot wrap.** A cell only reaches here with fewer than [`PATIENCE`]
        // behind it, because that is the fourth condition above — so this add tops out at the
        // largest value the counter holds and stays there. A counter that wrapped would hand the
        // cell its patience back and the pacing would start again.
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

/// A cell's index from its column and row, both wrapped into the grid.
///
/// The grid is a torus, which is what makes every edge case disappear: a block whose origin sits on
/// the last column reads the first, and nothing has to be clamped or skipped. A wall along the
/// bottom row is what stops sand actually falling through — geometry, not a special case.
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

/// The low `bits` of a value — a mask, spelled by an engine whose lane API has no bitwise and.
///
/// Down and back up, and the difference: `v − ((v >> bits) << bits)`.
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

/// The same step on the host, block by block.
///
/// **Written from the rule rather than from the kernel**, and in the same order, because the order
/// is what is being checked: rule three sees what rule one did.
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

        // Written from the rule rather than from the kernel — the two agree because they were
        // derived from the same four lines above, not because one was transcribed from the other.
        let put = |cells: &mut [u32; 4], at: usize, value: u32| {
            if let Some(slot) = cells.get_mut(at) {
                *slot = value;
            }
        };
        let read = |cells: &[u32; 4], at: usize| cells.get(at).copied().unwrap_or(WALL);

        // `course`: a runner moves one along its row, or stops against what is in the way.
        let along = |cells: &mut [u32; 4], left: usize, right: usize| {
            let (here, there) = (read(cells, left), read(cells, right));
            let goes_right = running_of(here) == RIGHTWARD;
            let goes_left = running_of(there) == LEFTWARD;
            let right_free = material_of(there) == AIR;
            let left_free = material_of(here) == AIR;

            let halt = |word: u32| {
                material_of(word) + held_of(word) * HELD + tired_of(word) * TIRED
            };
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
        // `hold`: the top row remembers what it is standing on, which only this block can see.
        let (top, under) = (read(&cells, 0), read(&cells, 2));
        put(&mut cells, 0, hold_of(top, under));
        let (top, under) = (read(&cells, 1), read(&cells, 3));
        put(&mut cells, 1, hold_of(top, under));

        // `start`: still water that remembers standing on water, with air along its row.
        let (a, b) = (read(&cells, 0), read(&cells, 1));
        let (c, d) = (read(&cells, 2), read(&cells, 3));
        let idle = |word: u32| material_of(word) == WATER && running_of(word) == 0;
        // `held_of` is one or nought and `tired_of` is below `PATIENCE` here, so the words written
        // below are the whole of what the rule produces rather than a common case of it.
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

        // `fall` and `down`: heavier above lighter swaps, whatever lands is still, and the diagonal
        // refuses anything as light as water.
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

/// How wide a brush dispatch is, in cells each way.
///
/// **A box around the pointer, not a pass over the grid.** The brush used to visit every cell and
/// ask whether it was near — one load, one compare and one store apiece, which at sixteen million
/// cells is as much work as the simulation. The pointer is somewhere; only the cells around it can
/// be painted; so only those are dispatched. A thousand and twenty-four invocations, whatever the
/// world is.
pub const BOX: u32 = 32;

/// Pour whatever the brush names into every empty cell of a box around it.
///
/// # Errors
///
/// As [`settle`].
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

/// The brush, at a width the caller has matched to the device.
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

    // Which cell of the box this invocation owns, and where that is in the world.
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

        // The distance, squared on both sides so there is no square root, and squared from a
        // difference so there is no sign either: `(x − bx)²` is the same whichever way it wrapped.
        let away_x = lanes.mul(bx, minus)?;
        let away_y = lanes.mul(by_row, minus)?;
        let dx = lanes.add(column, away_x)?;
        let dy = lanes.add(row, away_y)?;
        let across_sq = lanes.mul(dx, dx)?;
        let down_sq = lanes.mul(dy, dy)?;
        let far = lanes.add(across_sq, down_sq)?;

        // Three conditions, multiplied: inside the brush, empty here, and something to pour.
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

    // **A painted cell has to be looked at**, and by the quads either side of it, because the
    // partition shifts next frame. Without this, poured sand lands and is never in a work list.
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

/// How much of the world the window shows.
///
/// **A window rather than the whole thing.** Painting sixteen million cells and copying them back is
/// sixty-seven megabytes a frame — around thirty-eight milliseconds on this machine, against a
/// hundred and sixty *micro*seconds for the simulation that produced them. At that size the picture,
/// not the physics, is what stops the world being watchable, and a viewport is the only honest fix:
/// the cost of looking becomes the size of the window instead of the size of the world.
///
/// Both sides powers of two, for the reason [`Grid`]'s are — every address is a shift.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Cells across the window shows.
    pub wide: u32,
    /// Cells down.
    pub deep: u32,
}

impl View {
    /// A window of `wide` by `deep`, each rounded up to a power of two.
    #[must_use]
    pub const fn of(wide: u32, deep: u32) -> Self {
        Self {
            wide: wide.next_power_of_two(),
            deep: deep.next_power_of_two(),
        }
    }

    /// How many pixels it holds.
    #[must_use]
    pub const fn pixels(self) -> usize {
        self.wide as usize * self.deep as usize
    }
}

/// Turn the visible part of the grid into a picture: one word a pixel, as `0RGB`.
///
/// One invocation a *pixel*, not a cell. Where the window sits comes from [`LOOKING`] — the host's
/// two words, like the brush's four — so panning costs a write of eight bytes and no recompilation,
/// and the grid being a torus means the corner can be anywhere without an edge case.
///
/// # Errors
///
/// As [`settle`].
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

/// The picture, at a width the caller has matched to the device.
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

    // Pixel to cell: split the pixel index across the *window*, then offset by the corner and wrap
    // into the grid. `at` does the wrapping, so a window hanging off the right edge shows the left.
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
        // On the material, so running water is the same blue as still water is.
        let there = material::<LANES>(&mut lanes, word)?;

        let water = lanes.splat_bits::<U32, LANES>(WATER)?;
        let sand = lanes.splat_bits::<U32, LANES>(SAND)?;
        let wall = lanes.splat_bits::<U32, LANES>(WALL)?;
        let sky = lanes.splat_bits::<U32, LANES>(0x0011_1622)?;
        let wave = lanes.splat_bits::<U32, LANES>(0x0024_6FA8)?;
        let grain = lanes.splat_bits::<U32, LANES>(0x00D9_B36A)?;
        let stone = lanes.splat_bits::<U32, LANES>(0x0044_4A52)?;

        // A chain of selects rather than a table: four materials is three questions, and the lane
        // API has no gather to make it anything else.
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

/// Which word of [`PARAMS`] the window's corner lives in, as a number a constant can be made from.
fn looking(part: usize) -> u32 {
    u32::try_from(LOOKING + part).unwrap_or(u32::MAX)
}
