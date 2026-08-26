//! `cargo run --release --bin oracle -- exam.txt` — write the exam, and the card writes the logic.
//!
//! # What changes here
//!
//! `src/bin/chip.rs` evolves a grid of logic gates against a task written into the file. This is the
//! same grid with the task taken out of it: what the circuit has to do arrives as text, at runtime,
//! and the kernels are built around it. `simdr` emits SPIR-V from Rust, so the number of inputs is
//! an argument rather than a constant — which is the whole of what makes this a tool instead of an
//! experiment.
//!
//! You do not say how. You say what.
//!
//! ```text
//!   # a comment
//!   inputs 4        how many inputs, one to six
//!   outputs 3       how many answer bits — one if left out
//!   seed 1234       where the random grids start, which decides more than it looks
//!   parity          fill the table: the parity of the inputs
//!   majority        fill the table: more ones than noughts
//!   equal           fill the table: every input the same
//!   count           fill the table: how many ones there are, in binary
//!   add             fill the table: the first half plus the second
//!   10-1 -> 01      the inputs that pattern stands for, and what they answer
//!   ---- -> --1     a dash on the left is every value; a dash on the right is not asked
//! ```
//!
//! **A dash on the left is both.** `10-1` is two inputs at once and `----` is all sixteen, so one
//! line says what otherwise takes a table.
//!
//! **A dash on the right is silence.** The score counts answer *bits*, not cases, and a bit nobody
//! named counts for nothing either way — so `1--- -> 1-` asks about one output and leaves the other
//! alone, and the report says `16 answer bits marked of 32` rather than pretending to judge 32.
//!
//! **The answer comes out down the right-hand edge**, output nought on the top wire of the last
//! column and the rest under it, and the drawing marks them.
//!
//! What is left out is left out rather than assumed, which is what makes a partial exam a partial
//! score instead of a wrong one.
//!
//! # What comes back
//!
//! A grid of gates, drawn, and a number: how many of the marked cases it gets right. That number is
//! worked out twice, once by the card while it was evolving and once by the host afterwards from the
//! genome alone, and the report prints both. They have to agree.
//!
//! # What this is not
//!
//! # What it does, measured
//!
//! Four exams, 4096 chips, at most four thousand generations, on an RTX 4080:
//!
//! ```text
//!   exam                                    marked   reached
//!   inputs 4, parity                            16        16
//!   inputs 4, outputs 2, one output = input a   16        16
//!   inputs 4, outputs 3, add                    48        40
//!   inputs 4, outputs 3, count                  48        41
//! ```
//!
//! The first two arrive inside twenty generations. The last two do not arrive at all, and the score
//! is what it is: 41 of 48 is a chip right about most of the bits of most of the answers, which the
//! host confirms from the genome afterwards.
//!
//! # What this is not
//!
//! **It is not an oracle that always answers.** `src/bin/chip.rs` measured where the same search
//! stops: parity of three and four inputs it solves exactly, five it gets to 30 of 32, six to 56 of
//! 64 — and four different selection schemes all stopped at that 56. Parity is the hard case in
//! evolving circuits and nothing here changes that. What this does honestly is tell you the score
//! rather than claim one.
//!
//! **What it finds is not compact.** Four-input parity is three exclusive-ors written by hand; what
//! comes out of here is 88 gates of which most do nothing, and the drawing shows that plainly. It is
//! smaller than nothing and larger than anything a person would write.
//!
//! **No derivative is computed**, which is true and is the interesting part. There is no gradient,
//! no loss surface and no training: 4096 grids are drawn at random, the ones that lose are copied
//! over and one gate is turned, and the card runs a few thousand generations of that in a second.
//! `decisions/DR-0009` is why the pass that does the running is dispatched over what changed.
//!
//! Nothing else in the repository refers to this file. `rm src/bin/oracle.rs` removes it whole.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

/// How many rows the chip has: the inputs, and a nought and a one to build with.
const ROWS: u32 = 8;
/// How many columns, of which the first is the inputs.
const COLUMNS: u32 = 12;
/// How many gates a chip holds, which is the length of a genome.
const CELLS: u32 = (COLUMNS - 1) * ROWS;
/// How many kinds of gate there are.
const KINDS: u32 = 5;

/// How many gates a mutation turns over at once.
const MUTATIONS: u32 = 1;

/// How many chips are in the arena. A power of two: the wrap is a shift.
const COLONY: u32 = 1 << 12;
/// How many generations to run.
const GENERATIONS: u32 = 4_000;
/// How many go into one submission.
const BATCH: u32 = 20;

/// How many words every kernel declares.
const BINDINGS: u32 = 9;

/// Four words: the tick and the parity.
const PARAMS: u32 = 0;
/// [`CELLS`] words a chip: what kind of gate sits in each.
const GENE: u32 = 1;
/// One word an input: what the answer should be. As many as the exam has inputs.
const TARGET: u32 = 2;
/// Every wire of every chip: `((chip × COLUMNS + column) × ROWS + row) × CASES + case`.
const WIRE: u32 = 3;
/// One word a chip: how many of the marked inputs it gets right.
const SCORE: u32 = 4;
/// One word a chip: its own pseudo-random state.
const SEED: u32 = 5;
/// The chips that changed this generation, which are the only ones worth running.
const CHANGED: u32 = 6;
/// One word an input: whether the exam asks about it at all.
const KEEP: u32 = 8;

/// Seven words: how many chips changed, then two workgroup counts of three words — one for a pass
/// over every wire of them, one for a pass over the chips themselves.
const COUNTS: u32 = 7;
// ---------- arithmetic the lane API has no operator for ----------

/// This invocation's organism, or its slot in a list: workgroup × width + lane.
fn mine(kernel: &mut Kernel<U32>, subgroup: u32) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let along = kernel.module().i_mul(uint, group, width)?;
    Ok(kernel.module().i_add(uint, along, lane)?)
}

/// `held` less `off`, floored at nought.
///
/// **There is no subtraction in the lane API.** Floor the value at what is coming off, then add the
/// two's complement — the shape `src/sand.rs` uses, for the reason `DESIGN.md` gives.
fn minus(kernel: &mut Kernel<U32>, held: Id, off: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let take = lanes.splat_id::<U32, 1>(off)?;
    let floored = lanes.max(now, take)?;
    let ones = lanes.splat_bits::<U32, 1>(u32::MAX)?;
    let back = lanes.mul(take, ones)?;
    Ok(lanes.add(floored, back)?.id())
}

/// The low `bits` of `held`: a modulo by a power of two, done in shifts.
fn low_bits(kernel: &mut Kernel<U32>, held: Id, bits: u32) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let by = lanes.splat_bits::<U32, 1>(bits)?;
    let up = lanes.shift_right_logical(now, by)?;
    let down = lanes.shift_left(up, by)?;
    let ones = lanes.splat_bits::<U32, 1>(u32::MAX)?;
    let back = lanes.mul(down, ones)?;
    Ok(lanes.add(now, back)?.id())
}

/// `held`, but no larger than `ceiling`.
fn at_most(kernel: &mut Kernel<U32>, held: Id, ceiling: u32) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let top = lanes.splat_bits::<U32, 1>(ceiling)?;
    Ok(lanes.min(now, top)?.id())
}

/// One step of a linear congruential generator, which wraps because unsigned multiply does.
fn stir(kernel: &mut Kernel<U32>, seed: Id) -> Result<Id, LaneError> {
    let uint = kernel.element();
    let by = kernel.module().constant_u32(1_664_525)?;
    let plus = kernel.module().constant_u32(1_013_904_223)?;
    let spun = kernel.module().i_mul(uint, seed, by)?;
    Ok(kernel.module().i_add(uint, spun, plus)?)
}

/// Which chip this invocation is acting on, and `COLONY` — the dump — where it is acting on none.
///
/// **The dump is the whole of the bounds check.** A dispatch is rounded up to whole workgroups, so
/// the last one holds lanes past the end of the list; they read a clamped slot, write into the chip
/// at `COLONY`, and nothing reads it. `crate::compact` calls the same slot a dump.
fn listed(kernel: &mut Kernel<U32>, slot: Id, held: Id) -> Result<Id, LaneError> {
    let safe = at_most(kernel, slot, COLONY - 1)?;
    let who = kernel.load_at(CHANGED, safe)?;

    let mut lanes = kernel.lanes()?;
    let index = lanes.splat_id::<U32, 1>(slot)?;
    let length = lanes.splat_id::<U32, 1>(held)?;
    let inside = lanes.greater_than(length, index)?;
    let real = lanes.splat_id::<U32, 1>(who)?;
    let dump = lanes.splat_bits::<U32, 1>(COLONY)?;
    Ok(lanes.select(inside, real, dump)?.id())
}

/// `held` shifted down by however much `by` says.
fn shift_down(kernel: &mut Kernel<U32>, held: Id, by: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let much = lanes.splat_id::<U32, 1>(by)?;
    Ok(lanes.shift_right_logical(now, much)?.id())
}

/// `held` shifted down by a constant.
fn down_by(kernel: &mut Kernel<U32>, held: Id, by: u32) -> Result<Id, LaneError> {
    let much = kernel.module().constant_u32(by)?;
    shift_down(kernel, held, much)
}

/// Bit `at` of `held`.
fn bit(kernel: &mut Kernel<U32>, held: Id, at: u32) -> Result<Id, LaneError> {
    let shifted = down_by(kernel, held, at)?;
    low_bits(kernel, shifted, 1)
}

/// One where `a` is above `b`, nought where it is not.
fn above(kernel: &mut Kernel<U32>, a: Id, b: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let x = lanes.splat_id::<U32, 1>(a)?;
    let y = lanes.splat_id::<U32, 1>(b)?;
    let hit = lanes.greater_than(x, y)?;
    let yes = lanes.splat_bits::<U32, 1>(1)?;
    let no = lanes.splat_bits::<U32, 1>(0)?;
    Ok(lanes.select(hit, yes, no)?.id())
}

/// One where `held` equals `want`, nought where it does not.
fn is(kernel: &mut Kernel<U32>, held: Id, want: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let x = lanes.splat_id::<U32, 1>(held)?;
    let y = lanes.splat_id::<U32, 1>(want)?;
    let hit = lanes.equal(x, y)?;
    let yes = lanes.splat_bits::<U32, 1>(1)?;
    let no = lanes.splat_bits::<U32, 1>(0)?;
    Ok(lanes.select(hit, yes, no)?.id())
}

/// `yes` where `flag` is not nought, `no` where it is.
fn pick(kernel: &mut Kernel<U32>, flag: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let f = lanes.splat_id::<U32, 1>(flag)?;
    let none = lanes.splat_bits::<U32, 1>(0)?;
    let on = lanes.greater_than(f, none)?;
    let a = lanes.splat_id::<U32, 1>(yes)?;
    let b = lanes.splat_id::<U32, 1>(no)?;
    Ok(lanes.select(on, a, b)?.id())
}



// ---------- the kernels ----------

/// Advance the tick and the parity, and clear the list of chips that changed.
fn advance(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;

    let at_tick = kernel.module().constant_u32(0)?;
    let now = kernel.load_at(PARAMS, at_tick)?;
    let ticked = kernel.module().i_add(uint, now, one)?;
    kernel.store_at(PARAMS, at_tick, ticked)?;

    let at_parity = kernel.module().constant_u32(1)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let bumped = kernel.module().i_add(uint, parity, one)?;
    let flipped = low_bits(&mut kernel, bumped, 1)?;
    kernel.store_at(PARAMS, at_parity, flipped)?;

    kernel.store_at(COUNTS, none, none)?;
    kernel.finish()
}

/// Take the neighbour's chip where the neighbour did better, change one gate, and say so.
///
/// **Half the colony at a time**, as the other two demonstrations do and for the same reason: a
/// genome is read while others are written, so the ones being read from stand still.
fn select(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let who = mine(&mut kernel, subgroup)?;
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;

    let at_parity = kernel.module().constant_u32(1)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let turn = low_bits(&mut kernel, who, 1)?;
    let my_turn = is(&mut kernel, turn, parity)?;

    // **A different partner every generation, and always one of the other parity.** Measuring against
    // the chip next door only is an island of two: a good genome never travels and a bad one loses
    // to the same opponent for ever. The stride comes from the tick and is odd, so the partner is
    // the parity that is standing still this turn.
    let at_tick = kernel.module().constant_u32(0)?;
    let tick = kernel.load_at(PARAMS, at_tick)?;
    let spread = low_bits(&mut kernel, tick, COLONY.trailing_zeros())?;
    let two = kernel.module().constant_u32(2)?;
    let doubled = kernel.module().i_mul(uint, spread, two)?;
    let stride = kernel.module().i_add(uint, doubled, one)?;
    let raised = kernel.module().i_add(uint, who, stride)?;
    let neighbour = low_bits(&mut kernel, raised, COLONY.trailing_zeros())?;
    let ours = kernel.load_at(SCORE, who)?;
    let theirs = kernel.load_at(SCORE, neighbour)?;
    let better = above(&mut kernel, theirs, ours)?;
    let take = kernel.module().i_mul(uint, my_turn, better)?;
    // **Only what lost changes.** Mutating whenever a chip is not *ahead* of its neighbour means two
    // equals churn for ever and a good genome is destroyed by the next turn it takes; this way a
    // chip that drew or won keeps what it has, and only one that was beaten copies and is changed.
    //
    // **Except on the first tick**, where every score is nought, nobody is better than anybody, and
    // so nothing would ever be put on the list to be evaluated at all — which is a colony that sits
    // at nought for ever, and is what this did before the line below.
    let first = is(&mut kernel, tick, one)?;
    let ahead = above(&mut kernel, ours, theirs)?;
    let behind = minus(&mut kernel, one, ahead)?;
    let churn = kernel.module().i_mul(uint, my_turn, behind)?;
    let either = kernel.module().i_add(uint, churn, first)?;
    let restless = at_most(&mut kernel, either, 1)?;

    // **Three gates at a time, not one.** With one the colony settles at 56 of 64 by generation two
    // hundred and four times the colony does not move it: a single gate turned over is too small a
    // step to leave the hollow it is in. `MUTATIONS` is what that number cost.
    let mut seed = kernel.load_at(SEED, who)?;
    let mut spots = Vec::with_capacity(MUTATIONS as usize);
    for _ in 0..MUTATIONS {
        seed = stir(&mut kernel, seed)?;
        let drawn = low_bits(&mut kernel, seed, CELLS.next_power_of_two().trailing_zeros())?;
        let spot = at_most(&mut kernel, drawn, CELLS - 1)?;
        seed = stir(&mut kernel, seed)?;
        let rolled = low_bits(&mut kernel, seed, KINDS.next_power_of_two().trailing_zeros())?;
        let fresh = at_most(&mut kernel, rolled, KINDS - 1)?;
        spots.push((spot, fresh));
    }
    let again = seed;

    let span = kernel.module().constant_u32(CELLS)?;
    let ours_at = kernel.module().i_mul(uint, who, span)?;
    let theirs_at = kernel.module().i_mul(uint, neighbour, span)?;
    for cell in 0..CELLS {
        let off = kernel.module().constant_u32(cell)?;
        let here = kernel.module().i_add(uint, ours_at, off)?;
        let there = kernel.module().i_add(uint, theirs_at, off)?;
        let held = kernel.load_at(GENE, here)?;
        let other = kernel.load_at(GENE, there)?;
        let mut settled = pick(&mut kernel, take, other, held)?;
        for &(spot, fresh) in &spots {
            let at_spot = is(&mut kernel, spot, off)?;
            let changing = kernel.module().i_mul(uint, at_spot, restless)?;
            settled = pick(&mut kernel, changing, fresh, settled)?;
        }
        kernel.store_at(GENE, here, settled)?;
    }
    let last = stir(&mut kernel, again)?;
    kernel.store_at(SEED, who, last)?;

    let slot = kernel.atomic_add_at(COUNTS, none, restless)?;
    let safe = at_most(&mut kernel, slot, COLONY - 1)?;
    let landing = pick(&mut kernel, restless, safe, none)?;
    let standing = kernel.load_at(CHANGED, landing)?;
    let keeping = pick(&mut kernel, restless, who, standing)?;
    kernel.store_at(CHANGED, landing, keeping)?;
    kernel.finish()
}

/// Turn the length of that list into two workgroup counts: one a wire, one a chip.
fn ready(subgroup: u32, cases: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;
    let pad = kernel.module().constant_u32(subgroup - 1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let by = subgroup.trailing_zeros();

    // A pass over every wire of every changed chip runs `ROWS × CASES` invocations for each of
    // them, and a pass over the chips runs one.
    let wide = kernel
        .module()
        .constant_u32((ROWS * cases).trailing_zeros())?;
    let wires = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let much = lanes.splat_id::<U32, 1>(wide)?;
        lanes.shift_left(now, much)?.id()
    };
    for (slot, count) in [(1, wires), (4, held)] {
        let bumped = kernel.module().i_add(uint, count, pad)?;
        let groups = down_by(&mut kernel, bumped, by)?;
        let at_x = kernel.module().constant_u32(slot)?;
        kernel.store_at(COUNTS, at_x, groups)?;
        let at_y = kernel.module().constant_u32(slot + 1)?;
        kernel.store_at(COUNTS, at_y, one)?;
        let at_z = kernel.module().constant_u32(slot + 2)?;
        kernel.store_at(COUNTS, at_z, one)?;
    }
    kernel.finish()
}

/// This invocation's chip, row and input, taken apart from one index.
///
/// A pass over wires runs one invocation for every (chip, row, input) there is among the changed
/// chips, and `CASES` and `ROWS` are powers of two, so this is two shifts and two masks.
fn wire_at(
    kernel: &mut Kernel<U32>,
    subgroup: u32,
    cases: u32,
    held: Id,
) -> Result<(Id, Id, Id), LaneError> {
    let index = mine(kernel, subgroup)?;
    let case = low_bits(kernel, index, cases.trailing_zeros())?;
    let rest = down_by(kernel, index, cases.trailing_zeros())?;
    let row = low_bits(kernel, rest, ROWS.trailing_zeros())?;
    let slot = down_by(kernel, rest, ROWS.trailing_zeros())?;
    let chip = listed(kernel, slot, held)?;
    Ok((chip, row, case))
}

/// Where one wire of one chip lives.
fn wire(
    kernel: &mut Kernel<U32>,
    chip: Id,
    column: u32,
    row: Id,
    case: Id,
    cases: u32,
) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let wide = kernel.module().constant_u32(COLUMNS)?;
    let along = kernel.module().i_mul(uint, chip, wide)?;
    let at = kernel.module().constant_u32(column)?;
    let placed = kernel.module().i_add(uint, along, at)?;
    let tall = kernel.module().constant_u32(ROWS)?;
    let down = kernel.module().i_mul(uint, placed, tall)?;
    let seated = kernel.module().i_add(uint, down, row)?;
    let many = kernel.module().constant_u32(cases)?;
    let spread = kernel.module().i_mul(uint, seated, many)?;
    Ok(kernel.module().i_add(uint, spread, case)?)
}

/// Put the inputs into the first column: bit `row` of the input, and a one on the bottom row.
///
/// Rows past the sixth get nought for free, because an input is under 64 and its higher bits are
/// already nought — so only the one has to be said.
fn feed(subgroup: u32, cases: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, held)?;

    let shifted = shift_down(&mut kernel, case, row)?;
    let value = low_bits(&mut kernel, shifted, 1)?;
    let one = kernel.module().constant_u32(1)?;
    let bottom = kernel.module().constant_u32(ROWS - 1)?;
    let lowest = is(&mut kernel, row, bottom)?;
    let settled = pick(&mut kernel, lowest, one, value)?;
    let at = wire(&mut kernel, chip, 0, row, case, cases)?;
    kernel.store_at(WIRE, at, settled)?;
    kernel.finish()
}

/// One column of gates, fed by the column to its left.
///
/// **Eleven of these are generated**, one a column, which is what an emitter buys over a shader
/// language. Every gate is computed and the genome picks which answer is kept: a `Predicate` is not
/// a value, and `decisions/DR-0003` has no branches.
fn propagate(subgroup: u32, column: u32, cases: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, held)?;

    let up = {
        let back = kernel.module().constant_u32(ROWS - 1)?;
        let raised = kernel.module().i_add(uint, row, back)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let down = {
        let raised = kernel.module().i_add(uint, row, one)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let at_a = wire(&mut kernel, chip, column - 1, up, case, cases)?;
    let at_b = wire(&mut kernel, chip, column - 1, down, case, cases)?;
    let at_w = wire(&mut kernel, chip, column - 1, row, case, cases)?;
    let a = kernel.load_at(WIRE, at_a)?;
    let b = kernel.load_at(WIRE, at_b)?;
    let straight = kernel.load_at(WIRE, at_w)?;

    // `min` is and and `max` is or, for values that are nought or one; exclusive-or is the low bit
    // of the sum, because the lane API has neither an exclusive-or nor a bitwise and.
    let (both, either) = {
        let mut lanes = kernel.lanes()?;
        let x = lanes.splat_id::<U32, 1>(a)?;
        let y = lanes.splat_id::<U32, 1>(b)?;
        let small = lanes.min(x, y)?.id();
        let large = lanes.max(x, y)?.id();
        (small, large)
    };
    let sum = kernel.module().i_add(uint, a, b)?;
    let odd = low_bits(&mut kernel, sum, 1)?;
    let inverted = minus(&mut kernel, one, straight)?;

    let span = kernel.module().constant_u32(CELLS)?;
    let mine_at = kernel.module().i_mul(uint, chip, span)?;
    let cell = kernel.module().constant_u32((column - 1) * ROWS)?;
    let along = kernel.module().i_add(uint, mine_at, cell)?;
    let gene_at = kernel.module().i_add(uint, along, row)?;
    let kind = kernel.load_at(GENE, gene_at)?;

    let mut answer = straight;
    for (number, value) in [(1, both), (2, either), (3, odd), (4, inverted)] {
        let want = kernel.module().constant_u32(number)?;
        let hit = is(&mut kernel, kind, want)?;
        answer = pick(&mut kernel, hit, value, answer)?;
    }
    let at = wire(&mut kernel, chip, column, row, case, cases)?;
    kernel.store_at(WIRE, at, answer)?;
    kernel.finish()
}

/// How many of the inputs the top-right wire got right.
fn tally(subgroup: u32, cases: u32, outputs: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let slot = mine(&mut kernel, subgroup)?;
    let chip = listed(&mut kernel, slot, held)?;

    // **One tap a row, down the right-hand edge.** Output nought is the top wire of the last column,
    // output one the wire under it, and so on — the same place a one-bit exam reads, extended.
    let mut fit = kernel.module().constant_u32(0)?;
    for case in 0..cases {
        let which = kernel.module().constant_u32(case)?;
        let want = kernel.load_at(TARGET, which)?;
        let asked = kernel.load_at(KEEP, which)?;
        for output in 0..outputs {
            let row = kernel.module().constant_u32(output)?;
            let at = wire(&mut kernel, chip, COLUMNS - 1, row, which, cases)?;
            let said = kernel.load_at(WIRE, at)?;
            // **Both sides carry a mask.** A bit the exam left out counts for nothing either way, on
            // the input side and the output side alike, so a partial specification gives a partial
            // score rather than a wrong one.
            let wanted = bit(&mut kernel, want, output)?;
            let marked = bit(&mut kernel, asked, output)?;
            let hit = is(&mut kernel, said, wanted)?;
            let counted = kernel.module().i_mul(uint, hit, marked)?;
            fit = kernel.module().i_add(uint, fit, counted)?;
        }
    }
    kernel.store_at(SCORE, chip, fit)?;
    kernel.finish()
}

// ---------- the exam, which is the only thing the user writes ----------

/// What the user asked for: which inputs matter and what each should answer.
struct Exam {
    /// How many inputs, one to six.
    bits: u32,
    /// How many answer bits. Output nought is the top wire of the last column.
    outputs: u32,
    /// Where the random grids start.
    ///
    /// **It matters more than anything else in this file.** Five starts on the parity of six inputs
    /// came out at 50, 52, 56, 60 and 64 of 64, so one run says nothing on its own.
    seed: u32,
    /// The wanted answer for every input, its bits packed one an output.
    want: Vec<u32>,
    /// Which of those bits the exam asks about, packed the same way.
    ask: Vec<u32>,
    /// What to call it in the report.
    name: String,
}

impl Exam {
    /// How many inputs there are altogether.
    fn cases(&self) -> u32 {
        1 << self.bits
    }

    /// How many answer bits are marked, which is what a perfect score is.
    fn asked(&self) -> u32 {
        self.ask.iter().map(|it| it.count_ones()).sum()
    }
}

/// The example, printed and used when no file is named.
const WRITTEN: &str =
    "# The exam. Everything the machine is told, and nothing about how.\ninputs 4\noutputs 3\ncount\n";

/// Every input a pattern of noughts, ones and dashes stands for.
///
/// **A dash is both.** `10-1` is two inputs and `----` is all sixteen, so one line can say what
/// otherwise takes a table — and the leftmost character is input nought, the row it enters on.
fn spread_of(pattern: &str) -> Vec<u32> {
    let free: Vec<u32> = pattern
        .chars()
        .enumerate()
        .filter(|&(_, it)| it == '-')
        .filter_map(|(at, _)| u32::try_from(at).ok())
        .collect();
    let fixed = pattern
        .chars()
        .enumerate()
        .filter(|&(_, it)| it == '1')
        .filter_map(|(at, _)| u32::try_from(at).ok())
        .fold(0_u32, |held, at| held | (1 << at));
    (0..1_u32 << free.len())
        .map(|combination| {
            free.iter()
                .enumerate()
                .filter_map(|(which, &at)| u32::try_from(which).ok().map(|it| (it, at)))
                .fold(fixed, |held, (which, at)| {
                    held | (((combination >> which) & 1) << at)
                })
        })
        .collect()
}

/// Read an exam.
///
/// # The whole language
///
/// ```text
///   # a comment
///   inputs 4        how many inputs, one to six
///   outputs 3       how many answer bits — one if left out
///   seed 1234       where the random grids start, which decides more than it looks
///   parity          fill the table: the parity of the inputs
///   majority        fill the table: more ones than noughts
///   equal           fill the table: every input the same
///   count           fill the table: how many ones there are, in binary
///   add             fill the table: the first half plus the second
///   10-1 -> 01      the inputs that pattern stands for, and what they answer
///   ---- -> --1     a dash on the left is every value; a dash on the right is not asked
/// ```
///
/// **A bit nobody names is not scored**, on either side. Write four lines and the answer is judged
/// on those; write `count` and it is judged on everything. What is left out is left out rather than
/// assumed, which is what makes a partial exam a partial score instead of a wrong one.
fn read(text: &str) -> Result<Exam, String> {
    let mut bits = None;
    let mut outputs = 1_u32;
    let mut seed = 0x0AC1_E000_u32;
    let mut want: Vec<u32> = Vec::new();
    let mut ask: Vec<u32> = Vec::new();
    let mut name = String::from("an exam");

    for (number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let say = |what: &str| format!("line {}: {what}", number + 1);
        let mut word = line.split_whitespace();
        match (word.next(), bits) {
            (Some("seed"), _) => {
                seed = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`seed` wants a number"))?;
            }
            (Some("outputs"), _) => {
                outputs = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..ROWS).contains(&it))
                    .ok_or_else(|| say("`outputs` wants a number from one to seven"))?;
            }
            (Some("inputs"), _) => {
                let how = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..=6).contains(&it))
                    .ok_or_else(|| say("`inputs` wants a number from one to six"))?;
                bits = Some(how);
                want = vec![0; 1 << how];
                ask = vec![0; 1 << how];
            }
            (Some(_), None) => return Err(say("`inputs` has to come first")),
            (Some(filler @ ("parity" | "majority" | "equal" | "count" | "add")), Some(how)) => {
                let widest = match filler {
                    "count" => u32::BITS - how.leading_zeros(),
                    "add" => how / 2 + 1,
                    _ => 1,
                };
                if outputs < widest {
                    return Err(say(&format!("`{filler}` wants `outputs {widest}` or more")));
                }
                if filler == "add" && how % 2 != 0 {
                    return Err(say("`add` wants an even number of inputs"));
                }
                name = filler.to_owned();
                let full = (1_u32 << outputs) - 1;
                let half = how / 2;
                for case in 0..1_u32 << how {
                    let ones = case.count_ones();
                    let answer = match filler {
                        "parity" => ones & 1,
                        "majority" => u32::from(ones * 2 > how),
                        "equal" => u32::from(ones == 0 || ones == how),
                        "count" => ones,
                        _ => (case & ((1 << half) - 1)) + (case >> half),
                    };
                    if let (Some(a), Some(b)) =
                        (want.get_mut(case as usize), ask.get_mut(case as usize))
                    {
                        *a = answer;
                        *b = full;
                    }
                }
            }
            (Some(pattern), Some(how)) => {
                if pattern.len() != how as usize || pattern.chars().any(|it| !"01-".contains(it)) {
                    return Err(say("expected a pattern of noughts, ones and dashes"));
                }
                if word.next() != Some("->") {
                    return Err(say("expected `->` after the pattern"));
                }
                let answer = word
                    .next()
                    .filter(|it| it.len() == outputs as usize)
                    .filter(|it| it.chars().all(|c| "01-".contains(c)))
                    .ok_or_else(|| {
                        say("expected an answer of noughts, ones and dashes, `outputs` long")
                    })?;
                let (mut wants, mut asks) = (0_u32, 0_u32);
                for (at, it) in answer.chars().enumerate() {
                    let Ok(at) = u32::try_from(at) else { continue };
                    match it {
                        '1' => {
                            wants |= 1 << at;
                            asks |= 1 << at;
                        }
                        '0' => asks |= 1 << at,
                        _ => {}
                    }
                }
                for case in spread_of(pattern) {
                    if let (Some(a), Some(b)) =
                        (want.get_mut(case as usize), ask.get_mut(case as usize))
                    {
                        // What a line says replaces what it says about, and leaves the rest alone.
                        *a = (*a & !asks) | wants;
                        *b |= asks;
                    }
                }
                if name == "an exam" {
                    name = String::from("a table");
                }
            }
            (None, _) => {}
        }
    }
    let bits = bits.ok_or_else(|| String::from("the exam says nothing about `inputs`"))?;
    Ok(Exam { bits, outputs, seed, want, ask, name })
}

// ---------- the host, which lays out the arena and then asks almost nothing ----------

/// One step of the same generator the kernels use.
fn stirred(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

/// The chip, run again here exactly as the columns run it on the card.
fn runs(genome: &[u32], case: u32, outputs: u32) -> u32 {
    let mut column: Vec<u32> = (0..ROWS)
        .map(|row| if row == ROWS - 1 { 1 } else { (case >> row) & 1 })
        .collect();
    for at in 1..COLUMNS {
        let read = |row: u32| column.get((row % ROWS) as usize).copied().unwrap_or(0);
        let next: Vec<u32> = (0..ROWS)
            .map(|row| {
                let (up, down, straight) = (read(row + ROWS - 1), read(row + 1), read(row));
                match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                    Some(1) => up.min(down),
                    Some(2) => up.max(down),
                    Some(3) => (up + down) & 1,
                    Some(4) => 1 - straight,
                    _ => straight,
                }
            })
            .collect();
        column = next;
    }
    // The answer is the top `outputs` wires of the last column, packed as the exam packs it.
    (0..outputs)
        .filter(|&at| column.get(at as usize).copied().unwrap_or(0) == 1)
        .fold(0_u32, |held, at| held | (1 << at))
}

/// How many of the marked answer bits this chip gets right, worked out on the host.
///
/// **One a bit, not one a case**, so a chip right about two outputs of three scores two.
fn scored(genome: &[u32], exam: &Exam) -> u32 {
    (0..exam.cases())
        .map(|case| {
            let said = runs(genome, case, exam.outputs);
            let want = exam.want.get(case as usize).copied().unwrap_or(0);
            let ask = exam.ask.get(case as usize).copied().unwrap_or(0);
            u32::try_from(
                (0..exam.outputs)
                    .filter(|&at| (ask >> at) & 1 == 1 && (said >> at) & 1 == (want >> at) & 1)
                    .count(),
            )
            .unwrap_or(0)
        })
        .sum()
}

/// The chip as something to look at: one letter a gate, the inputs down the left.
///
/// **An arrow marks an answer wire.** Output nought is the top row of the last column, output one
/// the row under it, and the exam says how many there are.
fn drawn(genome: &[u32], bits: u32, outputs: u32) -> String {
    let mut out = String::new();
    for row in 0..ROWS {
        out.push_str("    ");
        out.push(if row + 1 == ROWS {
            '1'
        } else if row < bits {
            char::from(b'a' + u8::try_from(row).unwrap_or(0))
        } else {
            '0'
        });
        out.push(' ');
        for at in 1..COLUMNS {
            out.push(match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                Some(1) => '&',
                Some(2) => 'v',
                Some(3) => '^',
                Some(4) => '!',
                _ => '-',
            });
        }
        if row < outputs {
            out.push_str(" <- ");
            out.push(char::from(b'0' + u8::try_from(row).unwrap_or(0)));
        }
        out.push('\n');
    }
    out
}
/// The colony's scores as a row of bars, so the collapse can be watched rather than read.
fn spread(scores: &[u32], perfect: u32) -> String {
    const BINS: usize = 24;
    let mut held = [0_u32; BINS];
    for &score in scores {
        let at = (score as usize * (BINS - 1)) / (perfect.max(1) as usize);
        if let Some(slot) = held.get_mut(at.min(BINS - 1)) {
            *slot += 1;
        }
    }
    let most = held.iter().copied().max().unwrap_or(1).max(1);
    held.iter()
        .map(|&count| {
            let height = (count * 8) / most;
            char::from_u32(0x2581 + height.min(7)).unwrap_or(' ')
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // **The exam is the only input.** Name a file and it is read; name nothing and the one below is
    // used, printed, so there is something to copy and change.
    let named = std::env::args().nth(1);
    let text = match &named {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            println!("no exam named, so this one is used — put it in a file to change it:\n");
            for line in WRITTEN.lines() {
                println!("    {line}");
            }
            println!();
            WRITTEN.to_owned()
        }
    };
    let exam = read(&text).map_err(|why| format!("the exam does not read: {why}"))?;
    let cases = exam.cases();
    let perfect = exam.asked();
    if perfect == 0 {
        return Err("the exam asks about nothing".into());
    }

    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}", host.name());
    println!(
        "  {}: {} inputs, {} outputs, {perfect} answer bits marked of {}",
        exam.name,
        exam.bits,
        exam.outputs,
        cases * exam.outputs
    );

    let mut modules = vec![
        advance(width)?,
        select(width)?,
        ready(width, cases)?,
        feed(width, cases)?,
    ];
    for column in 1..COLUMNS {
        modules.push(propagate(width, column, cases)?);
    }
    modules.push(tally(width, cases, exam.outputs)?);
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();

    let wires = Groups::From { binding: COUNTS as usize, at: 1 };
    let chips = Groups::From { binding: COUNTS as usize, at: 4 };
    let mut stages: Vec<Vec<Groups>> = vec![
        vec![Groups::Counting(1)],
        vec![Groups::Fixed(COLONY / width)],
        vec![Groups::Counting(1)],
        vec![wires],
    ];
    for _ in 1..COLUMNS {
        stages.push(vec![wires]);
    }
    stages.push(vec![chips]);
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    println!(
        "  {COLONY} chips of {COLUMNS}×{ROWS} gates — {} passes in {} stages\n",
        modules.len(),
        stages.len()
    );

    let sizes = [
        4,
        (COLONY * CELLS) as usize,
        cases as usize,
        ((COLONY + 1) * COLUMNS * ROWS * cases) as usize,
        (COLONY + 1) as usize,
        (COLONY + 1) as usize,
        COLONY as usize,
        7,
        cases as usize,
    ];
    let mut arena = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        arena.write(binding, &vec![0_u32; *words])?;
    }
    arena.write(TARGET as usize, &exam.want)?;
    arena.write(KEEP as usize, &exam.ask)?;
    let mut seed = exam.seed;
    let genes: Vec<u32> = (0..COLONY * CELLS).map(|_| stirred(&mut seed) % KINDS).collect();
    arena.write(GENE as usize, &genes)?;
    let seeds: Vec<u32> = (0..=COLONY).map(|_| stirred(&mut seed)).collect();
    arena.write(SEED as usize, &seeds)?;

    let batches = GENERATIONS / BATCH;
    let mut spent = Duration::ZERO;
    println!("  {:>10}  {:>5}  {:>24}  {:>10}", "generation", "best", "the colony", "us a tick");
    let mut best;
    let mut ran = 0;
    for batch in 0..batches {
        ran = batch + 1;
        spent += arena.staged_frames(&groups, BATCH)?;
        let scores = arena.read(SCORE as usize, COLONY as usize)?;
        best = scores.iter().copied().max().unwrap_or(0);
        // Densely at the start, where the colony is actually moving, and thinly after.
        if batch < 8 || batch % (batches / 8).max(1) == 0 || best == perfect || batch + 1 == batches {
            println!(
                "  {:>10}  {best:>5}  {}  {:>10.3}",
                (batch + 1) * BATCH,
                spread(&scores, perfect),
                arena.on_device().unwrap_or_default().as_secs_f64() * 1e6
            );
        }
        if best == perfect {
            break;
        }
    }

    let scores = arena.read(SCORE as usize, COLONY as usize)?;
    let genes = arena.read(GENE as usize, (COLONY * CELLS) as usize)?;
    let (winner, _) = scores.iter().enumerate().max_by_key(|&(_, s)| *s).unwrap_or((0, &0));
    let genome = genes
        .get(winner * CELLS as usize..(winner + 1) * CELLS as usize)
        .unwrap_or(&[]);
    println!("\n{}", drawn(genome, exam.bits, exam.outputs));
    println!("    - wire   & and   v or   ^ xor   ! not   (<- marks an answer wire)");
    let again = scored(genome, &exam);
    println!(
        "\n  worked out on the host it gets {again} of {perfect} — the card said {}{}",
        scores.get(winner).copied().unwrap_or(0),
        if again == perfect { ", and that is every case the exam asked about" } else { "" }
    );
    println!(
        "  {:.3} us a generation of host clock",
        spent.as_secs_f64() * 1e6 / f64::from(ran.max(1))
    );
    Ok(())
}
