//! `cargo run --release --bin chip` — a grid of logic gates that wires itself.
//!
//! # What this is for
//!
//! `decisions/DR-0009` says a work list buys time where a pass is many short independent steps and
//! the device was full of them, and almost nothing where the cost is one chain of dependent steps.
//! It was written on the second of those. **This is the first**, and it is here to try the condition
//! from the other side rather than take it on trust.
//!
//! It does hold. A pass over the chips that changed rather than over all of them runs in 73.6 µs
//! against 155.6 at four inputs, and 185.6 against 381.5 at six — 44% of the work for 47 and 49% of
//! the time, where the same arrangement in `src/bin/ticket.rs` took 98% of the time for 31% of the
//! work. An invocation here is a few loads, four gates and a store; there it walked 256 inputs by
//! 1281 connections in a carried loop.
//!
//! # The chip
//!
//! ```text
//!   column 0     columns 1 … 11
//!   4 inputs  →  a gate a cell  →  the answer, top right
//!   0s and a 1
//! ```
//!
//! Eight rows and twelve columns. Every cell past the first column is a gate fed by the two cells
//! diagonally to its left, and what kind of gate is the organism's genome — 88 numbers, one a cell:
//!
//! ```text
//!   0  wire   what is directly to the left
//!   1  and    min, which is *and* for values that are nought or one
//!   2  or     max
//!   3  xor    the low bit of the sum, because there is no exclusive-or in the lane API
//!   4  not    one less what is directly to the left
//! ```
//!
//! Signal moves one column a pass, so eleven passes carry it across, and each is dispatched over the
//! wires of the chips that changed — a count the device wrote and the host never sees.
//!
//! # What it solves, and where it stops
//!
//! The task is the **parity** of the inputs, on every input there is, which is the case a grid of
//! `min` and `max` alone cannot do at any width and the reason there is an exclusive-or on the list.
//! Four thousand generations of 4096 chips, and the winner is drawn at the end for reading:
//!
//! ```text
//!   inputs   cases   best
//!        3       8    8      solved
//!        4      16   16      solved
//!        5      32   30
//!        6      64   56
//! ```
//!
//! **Parity is the hard case in evolving circuits, and how hard is a lottery.** Five starts on six
//! inputs, changing nothing but where the random grids begin, came out at 50, 52, 56, 60 and 64 of
//! 64 — measured in `src/bin/oracle.rs`, which takes the start from its exam. So it solves six-input
//! parity about one run in five, and the table above is one run a size rather than a ceiling.
//!
//! **Which means the four selection schemes tried here were compared on one run each**, reading 48
//! to 56, and that spread sits inside the spread of the starts. It says nothing about the schemes.
//! The two that are kept — one gate a mutation rather than three, and a partner drawn afresh rather
//! than the chip next door — are kept on the argument for them, and the numbers that looked like
//! evidence were not.
//!
//! Two of those attempts failed in ways worth keeping: mutating only what lost froze the whole
//! colony at nought, because on the first tick no score beats any other and so nothing was ever put
//! on the list to be evaluated at all. `select` says out loud where that is handled.
//!
//! Nothing else in the repository refers to this file. `rm src/bin/chip.rs` removes it whole.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

/// How many inputs the task has.
const BITS: u32 = 4;
/// Every input there is, so the score is exhaustive.
const CASES: u32 = 1 << BITS;
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
/// What a chip has to score to be right about every input.
const PERFECT: u32 = CASES;

/// How many chips are in the arena. A power of two: the wrap is a shift.
const COLONY: u32 = 1 << 12;
/// How many generations to run.
const GENERATIONS: u32 = 4_000;
/// How many go into one submission.
const BATCH: u32 = 200;

/// How many words every kernel declares.
const BINDINGS: u32 = 8;

/// Four words: the tick and the parity.
const PARAMS: u32 = 0;
/// [`CELLS`] words a chip: what kind of gate sits in each.
const GENE: u32 = 1;
/// [`CASES`] words: what the answer should be for each input.
const TARGET: u32 = 2;
/// Every wire of every chip: `((chip × COLUMNS + column) × ROWS + row) × CASES + case`.
const WIRE: u32 = 3;
/// One word a chip: how many of the [`CASES`] it gets right.
const SCORE: u32 = 4;
/// One word a chip: its own pseudo-random state.
const SEED: u32 = 5;
/// The chips that changed this generation, which are the only ones worth running.
const CHANGED: u32 = 6;
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
fn ready(subgroup: u32) -> Result<Vec<u32>, LaneError> {
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
        .constant_u32((ROWS * CASES).trailing_zeros())?;
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
fn wire_at(kernel: &mut Kernel<U32>, subgroup: u32, held: Id) -> Result<(Id, Id, Id), LaneError> {
    let index = mine(kernel, subgroup)?;
    let case = low_bits(kernel, index, CASES.trailing_zeros())?;
    let rest = down_by(kernel, index, CASES.trailing_zeros())?;
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
) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let wide = kernel.module().constant_u32(COLUMNS)?;
    let along = kernel.module().i_mul(uint, chip, wide)?;
    let at = kernel.module().constant_u32(column)?;
    let placed = kernel.module().i_add(uint, along, at)?;
    let tall = kernel.module().constant_u32(ROWS)?;
    let down = kernel.module().i_mul(uint, placed, tall)?;
    let seated = kernel.module().i_add(uint, down, row)?;
    let many = kernel.module().constant_u32(CASES)?;
    let spread = kernel.module().i_mul(uint, seated, many)?;
    Ok(kernel.module().i_add(uint, spread, case)?)
}

/// Put the inputs into the first column: bit `row` of the input, and a one on the bottom row.
///
/// Rows past the sixth get nought for free, because an input is under 64 and its higher bits are
/// already nought — so only the one has to be said.
fn feed(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, held)?;

    let shifted = shift_down(&mut kernel, case, row)?;
    let value = low_bits(&mut kernel, shifted, 1)?;
    let one = kernel.module().constant_u32(1)?;
    let bottom = kernel.module().constant_u32(ROWS - 1)?;
    let lowest = is(&mut kernel, row, bottom)?;
    let settled = pick(&mut kernel, lowest, one, value)?;
    let at = wire(&mut kernel, chip, 0, row, case)?;
    kernel.store_at(WIRE, at, settled)?;
    kernel.finish()
}

/// One column of gates, fed by the column to its left.
///
/// **Eleven of these are generated**, one a column, which is what an emitter buys over a shader
/// language. Every gate is computed and the genome picks which answer is kept: a `Predicate` is not
/// a value, and `decisions/DR-0003` has no branches.
fn propagate(subgroup: u32, column: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, held)?;

    let up = {
        let back = kernel.module().constant_u32(ROWS - 1)?;
        let raised = kernel.module().i_add(uint, row, back)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let down = {
        let raised = kernel.module().i_add(uint, row, one)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let at_a = wire(&mut kernel, chip, column - 1, up, case)?;
    let at_b = wire(&mut kernel, chip, column - 1, down, case)?;
    let at_w = wire(&mut kernel, chip, column - 1, row, case)?;
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
    let at = wire(&mut kernel, chip, column, row, case)?;
    kernel.store_at(WIRE, at, answer)?;
    kernel.finish()
}

/// How many of the inputs the top-right wire got right.
fn tally(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let slot = mine(&mut kernel, subgroup)?;
    let chip = listed(&mut kernel, slot, held)?;

    let mut fit = kernel.module().constant_u32(0)?;
    for case in 0..CASES {
        let which = kernel.module().constant_u32(case)?;
        let at = wire(&mut kernel, chip, COLUMNS - 1, none, which)?;
        let said = kernel.load_at(WIRE, at)?;
        let want = kernel.load_at(TARGET, which)?;
        let hit = is(&mut kernel, said, want)?;
        fit = kernel.module().i_add(uint, fit, hit)?;
    }
    kernel.store_at(SCORE, chip, fit)?;
    kernel.finish()
}

// ---------- the host, which lays out the arena and then asks almost nothing ----------

/// One step of the same generator the kernels use.
fn stirred(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

/// What the answer should be: the parity of the inputs.
fn wanted(case: u32) -> u32 {
    case.count_ones() & 1
}

/// The chip, run again here exactly as the columns run it on the card.
fn runs(genome: &[u32], case: u32) -> u32 {
    let mut column: Vec<u32> = (0..ROWS)
        .map(|row| {
            if row == ROWS - 1 {
                1
            } else {
                (case >> row) & 1
            }
        })
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
    column.first().copied().unwrap_or(0)
}

/// How many of the [`CASES`] this chip gets right, worked out on the host.
fn scored(genome: &[u32]) -> u32 {
    (0..CASES)
        .filter(|&case| runs(genome, case) == wanted(case))
        .count()
        .try_into()
        .unwrap_or(0)
}

/// The chip as something to look at: one letter a gate, the inputs down the left.
fn drawn(genome: &[u32]) -> String {
    let mut out = String::new();
    for row in 0..ROWS {
        out.push_str("    ");
        out.push(if row + 1 == ROWS {
            '1'
        } else if row < BITS {
            char::from(b'a' + u8::try_from(row).unwrap_or(0))
        } else {
            '0'
        });
        out.push(' ');
        for at in 1..COLUMNS {
            out.push(
                match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                    Some(1) => '&',
                    Some(2) => 'v',
                    Some(3) => '^',
                    Some(4) => '!',
                    _ => '-',
                },
            );
        }
        out.push('\n');
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}", host.name());

    let mut modules = vec![advance(width)?, select(width)?, ready(width)?, feed(width)?];
    for column in 1..COLUMNS {
        modules.push(propagate(width, column)?);
    }
    modules.push(tally(width)?);
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();

    let wires = Groups::From {
        binding: COUNTS as usize,
        at: 1,
    };
    let chips = Groups::From {
        binding: COUNTS as usize,
        at: 4,
    };
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
        "  {COLUMNS}×{ROWS} chips, {CELLS} gates, {CASES} inputs — {} passes in {} stages",
        modules.len(),
        stages.len()
    );

    let sizes = [
        4,
        (COLONY * CELLS) as usize,
        CASES as usize,
        ((COLONY + 1) * COLUMNS * ROWS * CASES) as usize,
        (COLONY + 1) as usize,
        (COLONY + 1) as usize,
        COLONY as usize,
        7,
    ];
    let mut arena = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        arena.write(binding, &vec![0_u32; *words])?;
    }
    let answers: Vec<u32> = (0..CASES).map(wanted).collect();
    arena.write(TARGET as usize, &answers)?;
    let mut seed = 0xC417_0000_u32;
    let genes: Vec<u32> = (0..COLONY * CELLS)
        .map(|_| stirred(&mut seed) % KINDS)
        .collect();
    arena.write(GENE as usize, &genes)?;
    let seeds: Vec<u32> = (0..=COLONY).map(|_| stirred(&mut seed)).collect();
    arena.write(SEED as usize, &seeds)?;

    let batches = GENERATIONS / BATCH;
    let mut spent = Duration::ZERO;
    println!(
        "\n  {:>11}  {:>11}  {:>8}  {:>9}  {:>10}",
        "generation",
        format!("best of {PERFECT}"),
        "mean",
        "run again",
        "us a tick"
    );
    for batch in 0..batches {
        spent += arena.staged_frames(&groups, BATCH)?;
        if batch % (batches / 10).max(1) == 0 || batch + 1 == batches {
            let scores = arena.read(SCORE as usize, COLONY as usize)?;
            let best = scores.iter().copied().max().unwrap_or(0);
            let mean = scores.iter().map(|&s| u64::from(s)).sum::<u64>() / u64::from(COLONY);
            let changed = arena.read(COUNTS as usize, 1)?.first().copied().unwrap_or(0);
            println!(
                "  {:>11}  {:>11}  {mean:>8}  {changed:>9}  {:>10.3}",
                (batch + 1) * BATCH,
                best,
                arena.on_device().unwrap_or_default().as_secs_f64() * 1e6
            );
        }
    }

    let scores = arena.read(SCORE as usize, COLONY as usize)?;
    let genes = arena.read(GENE as usize, (COLONY * CELLS) as usize)?;
    let (winner, _) = scores
        .iter()
        .enumerate()
        .max_by_key(|&(_, s)| *s)
        .unwrap_or((0, &0));
    let genome = genes
        .get(winner * CELLS as usize..(winner + 1) * CELLS as usize)
        .unwrap_or(&[]);
    println!("\n{}", drawn(genome));
    println!("    - wire   & and   v or   ^ xor   ! not   (answer is the top right)");
    println!(
        "\n  worked out on the host it gets {} of {PERFECT} right — the card said {}",
        scored(genome),
        scores.get(winner).copied().unwrap_or(0)
    );
    println!(
        "  {batches} submissions, {:.3} us a generation of host clock",
        spent.as_secs_f64() * 1e6 / f64::from(batches)
    );
    Ok(())
}
