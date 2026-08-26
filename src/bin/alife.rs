//! `cargo run --release --bin alife` — programs that choose which kernel runs next, and evolve.
//!
//! # What this is for
//!
//! `tests/floor.rs` puts a pass the device switched off at 46–47 ns and a pass it left on at
//! 117–118, and `Groups::Counting` lets a kernel write the workgroup count Vulkan reads at dispatch
//! time. Put those together and the *shape* of a frame can be decided on the card: a command buffer
//! holding every instruction there is, of which the device picks the few that run.
//!
//! So this is an instruction set of fifteen kernels and a colony of programs written in them. Each
//! organism holds sixteen instruction numbers — that is its whole genome — and every tick it appends
//! its own index to the list belonging to the instruction it is on. One pass turns the fifteen list
//! lengths into fifteen workgroup counts. Fifteen kernels are then dispatched over those lists, and
//! the ones nobody chose cost 47 ns each.
//!
//! **The host never learns which instruction anything chose.** It submits one recorded command
//! buffer over and over and asks for a number eleven times in twenty thousand generations, so that
//! the numbers can be seen to exist. What changes between submissions is what the buffers hold.
//!
//! # The task, and why this one
//!
//! Sort six numbers. The fifteen instructions are the fifteen compare-exchanges on six elements —
//! `swap(0,1)`, `swap(0,2)` … `swap(4,5)` — one kernel apiece, generated rather than written, which
//! is what `simdr` being an emitter rather than a language buys.
//!
//! **Six is chosen because the answer is checkable.** A comparator network sorts everything if and
//! only if it sorts every input of noughts and ones, of which there are sixty-four; so fitness here
//! is not a sample and not luck, and 320 — five adjacent pairs over sixty-four inputs — means a
//! sorting network. The host then checks the winner a second way, over all 720 orderings of six
//! distinct numbers, and the two answers have to agree.
//!
//! # What it does, measured
//!
//! It finds one. Twenty thousand generations of four thousand organisms takes about seventy
//! milliseconds of host clock, and the genome that comes out sorts 720 of 720. **It does not find
//! the shortest one**: twelve comparators is the proved minimum on six elements, the genome here has
//! room for sixteen, and at twelve the colony stalls two points short of 320 — those networks are
//! rare enough in 15¹² that a local search from a random start does not reach them.
//!
//! # What the tick costs, and what does not scale
//!
//! A tick is twenty-one passes in six stages. Two barriers are wide, because two passes write
//! workgroup counts, and `tests/floor.rs` prices those at 2373–2401 ns against 1097–1098 for the
//! other four.
//!
//! ```text
//!   colony    a tick
//!     1024   20.3 µs
//!     4096   32.6 µs
//!    16384   89.4 µs
//! ```
//!
//! **The scheduling is flat and the work is not.** A line through those three puts about 16 µs on
//! the arrangement — the barriers, the twenty-one dispatches, the fifteen lists — and 4.5 ns on each
//! organism, because a comparator moves sixty-four words for every one of them. Sixteen times the
//! colony costs four and a half times the tick.
//!
//! That is worth saying plainly, because it is not what the sand shows. There, the frame is flat
//! because the *work* is proportional to what moves rather than to what exists. Here every organism
//! works every tick, so only the half of the cost that is Vulkan stays still.
//!
//! # What it is not
//!
//! It appends with an atomic rather than a prefix sum, which is the thing `crate::compact` exists to
//! avoid. One atomic an organism against one scan a block is the trade `src/sweep.rs` argues, and
//! for showing who decides a workgroup count the atomic is the shortest road.
//!
//! Nothing else in the repository refers to this file. `rm src/bin/alife.rs` removes it whole.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

/// How many numbers an organism sorts.
const VALUES: u32 = 6;

/// How many inputs every organism is tested on at once.
///
/// # The nought-one principle, which is why this is the whole test and not a sample
///
/// A comparator network sorts every input it can be given **if and only if** it sorts every input
/// made only of noughts and ones. There are 2⁶ of those, they are the same for every organism and
/// every generation, and a network that sorts all of them is a sorting network — proved, not
/// sampled. So fitness here is not luck: 320 is 5 adjacent pairs over 64 inputs, and 320 means it.
///
/// It costs nothing extra to say so, because for values that are nought or one, `min` is *and* and
/// `max` is *or* — which is the comparator the lane API already has.
const INPUTS: u32 = 1 << VALUES;

/// The most a genome can score: every adjacent pair in order, on every input.
const PERFECT: u32 = (VALUES - 1) * INPUTS;

/// How many compare-exchanges there are on [`VALUES`] elements, and so how many instructions.
const INSTRUCTIONS: u32 = VALUES * (VALUES - 1) / 2;

/// How many instructions a genome holds.
///
/// **Four more than it needs.** Twelve comparators is the proved shortest network on six elements,
/// and at twelve this colony stalls at 318 of 320: those genomes are too rare in 15¹² for a local
/// search to land on one. Sixteen is enough room to find a network at all.
const GENOME: u32 = 16;

/// How many organisms the colony holds. A power of two: the wrap is a shift.
const COLONY: u32 = 1 << 12;

/// How many words every kernel declares.
const BINDINGS: u32 = 8;

/// Four words: the tick, the step within a generation, and the two gates below.
const PARAMS: u32 = 0;
/// [`GENOME`] words an organism: which instruction it runs at each step.
const CODE: u32 = 1;
/// [`VALUES`] × [`INPUTS`] words an organism: element `k` of input `x` at `k * INPUTS + x`.
const WORK: u32 = 2;
/// One word an organism: how many adjacent pairs came out in order, over every input.
const SCORE: u32 = 3;
/// One word an organism: its own pseudo-random state.
const SEED: u32 = 4;
/// [`INSTRUCTIONS`] lists of [`COLONY`] organism indices, each starting at `instruction * COLONY`.
const LIST: u32 = 5;
/// [`INSTRUCTIONS`] words: how long each list turned out to be.
const COUNTS: u32 = 6;
/// Workgroup counts, three words apiece: the fifteen instructions, then `reset`, `score`, `select`.
const DISPATCH: u32 = 7;

/// Where in [`DISPATCH`] the three once-a-generation passes keep their counts.
const RESET_AT: u32 = INSTRUCTIONS * 3;
/// As [`RESET_AT`].
const SCORE_AT: u32 = RESET_AT + 3;
/// As [`RESET_AT`].
const SELECT_AT: u32 = SCORE_AT + 3;

/// How many generations to run.
const GENERATIONS: u32 = 20_000;

/// How many ticks go into one submission. A whole number of generations, so a batch never lands
/// halfway through one.
const BATCH: u32 = GENOME * 10;

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

/// `then` where `held` equals `want`, otherwise `otherwise`.
fn when(
    kernel: &mut Kernel<U32>,
    held: Id,
    want: u32,
    then: u32,
    otherwise: Id,
) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let mark = lanes.splat_bits::<U32, 1>(want)?;
    let hit = lanes.equal(now, mark)?;
    let yes = lanes.splat_bits::<U32, 1>(then)?;
    let no = lanes.splat_id::<U32, 1>(otherwise)?;
    Ok(lanes.select(hit, yes, no)?.id())
}

/// One step of a linear congruential generator, which wraps because unsigned multiply does.
fn stir(kernel: &mut Kernel<U32>, seed: Id) -> Result<Id, LaneError> {
    let uint = kernel.element();
    let by = kernel.module().constant_u32(1_664_525)?;
    let plus = kernel.module().constant_u32(1_013_904_223)?;
    let spun = kernel.module().i_mul(uint, seed, by)?;
    Ok(kernel.module().i_add(uint, spun, plus)?)
}

/// Which organism this invocation is acting on, and `COLONY` — the dump — where it is acting on
/// none.
///
/// **The dump is the whole of the bounds check.** A dispatch is rounded up to whole workgroups, so
/// the last one holds lanes past the end of the list; they read a clamped slot, write to `COLONY`,
/// and nothing reads `COLONY`. `crate::compact` calls the same slot a dump and uses it for the same
/// reason.
fn listed(kernel: &mut Kernel<U32>, subgroup: u32, list: u32, held: Id) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let slot = mine(kernel, subgroup)?;
    let safe = at_most(kernel, slot, COLONY - 1)?;
    let base = kernel.module().constant_u32(list * COLONY)?;
    let at = kernel.module().i_add(uint, base, safe)?;
    let who = kernel.load_at(LIST, at)?;

    let mut lanes = kernel.lanes()?;
    let index = lanes.splat_id::<U32, 1>(slot)?;
    let length = lanes.splat_id::<U32, 1>(held)?;
    let inside = lanes.greater_than(length, index)?;
    let real = lanes.splat_id::<U32, 1>(who)?;
    let dump = lanes.splat_bits::<U32, 1>(COLONY)?;
    Ok(lanes.select(inside, real, dump)?.id())
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

/// Advance the tick, the step and the parity, clear the lists, and set the three once-a-generation
/// workgroup counts.
///
/// **Every lane writes the same words with the same values**, which `crate::live::tick` documents as
/// the case `Kernel::store_at` allows. It is one workgroup and the alternative is a branch, which
/// `decisions/DR-0003` will not have.
fn advance(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;

    let at_tick = kernel.module().constant_u32(0)?;
    let now = kernel.load_at(PARAMS, at_tick)?;
    let ticked = kernel.module().i_add(uint, now, one)?;
    kernel.store_at(PARAMS, at_tick, ticked)?;

    let at_step = kernel.module().constant_u32(1)?;
    let step = kernel.load_at(PARAMS, at_step)?;
    let raised = kernel.module().i_add(uint, step, one)?;
    let wrapped = when(&mut kernel, raised, GENOME, 0, raised)?;
    kernel.store_at(PARAMS, at_step, wrapped)?;

    // The parity turns over with the generation, and is what keeps `choose` from reading a genome
    // another invocation is writing: on an even generation the even organisms take, on an odd one
    // the odd ones do, and the ones being read from are standing still.
    let at_parity = kernel.module().constant_u32(2)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let bumped = kernel.module().i_add(uint, parity, one)?;
    let flipped = low_bits(&mut kernel, bumped, 1)?;
    let turning = is(&mut kernel, wrapped, none)?;
    let settled = pick(&mut kernel, turning, flipped, parity)?;
    kernel.store_at(PARAMS, at_parity, settled)?;

    for instruction in 0..INSTRUCTIONS {
        let at = kernel.module().constant_u32(instruction)?;
        kernel.store_at(COUNTS, at, none)?;
    }

    // The three passes that run once a generation, switched on by their workgroup count and off the
    // same way. `tests/floor.rs` prices a pass switched off at 46–47 ns.
    let full = kernel.module().constant_u32(COLONY / subgroup)?;
    let last = kernel.module().constant_u32(GENOME - 1)?;
    let starting = is(&mut kernel, wrapped, none)?;
    let ending = is(&mut kernel, wrapped, last)?;
    for (slot, gate) in [(RESET_AT, starting), (SCORE_AT, ending), (SELECT_AT, starting)] {
        let count = pick(&mut kernel, gate, full, none)?;
        let at = kernel.module().constant_u32(slot)?;
        kernel.store_at(DISPATCH, at, count)?;
        let at_y = kernel.module().constant_u32(slot + 1)?;
        kernel.store_at(DISPATCH, at_y, one)?;
        let at_z = kernel.module().constant_u32(slot + 2)?;
        kernel.store_at(DISPATCH, at_z, one)?;
    }
    kernel.finish()
}

/// Turn the fifteen list lengths into fifteen workgroup counts.
fn ready(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let pad = kernel.module().constant_u32(subgroup - 1)?;
    let by = subgroup.trailing_zeros();

    for instruction in 0..INSTRUCTIONS {
        let at = kernel.module().constant_u32(instruction)?;
        let held = kernel.load_at(COUNTS, at)?;
        let bumped = kernel.module().i_add(uint, held, pad)?;
        let groups = {
            let mut lanes = kernel.lanes()?;
            let now = lanes.splat_id::<U32, 1>(bumped)?;
            let shift = lanes.splat_bits::<U32, 1>(by)?;
            lanes.shift_right_logical(now, shift)?.id()
        };
        let slot = instruction * 3;
        let at_x = kernel.module().constant_u32(slot)?;
        kernel.store_at(DISPATCH, at_x, groups)?;
        let at_y = kernel.module().constant_u32(slot + 1)?;
        kernel.store_at(DISPATCH, at_y, one)?;
        let at_z = kernel.module().constant_u32(slot + 2)?;
        kernel.store_at(DISPATCH, at_z, one)?;
    }
    kernel.finish()
}

/// Every organism appends its own index to the list of the instruction it is on this step.
///
/// **The one atomic in the file**, and the thing `crate::compact` exists to do without. `COLONY` is
/// a power of two and a multiple of every width this builds for, so there are no lanes past the end
/// here and no dump is needed.
fn route(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let one = kernel.module().constant_u32(1)?;
    let who = mine(&mut kernel, subgroup)?;

    let at_step = kernel.module().constant_u32(1)?;
    let step = kernel.load_at(PARAMS, at_step)?;
    let span = kernel.module().constant_u32(GENOME)?;
    let base = kernel.module().i_mul(uint, who, span)?;
    let at = kernel.module().i_add(uint, base, step)?;
    let raw = kernel.load_at(CODE, at)?;
    let instruction = at_most(&mut kernel, raw, INSTRUCTIONS - 1)?;

    let slot = kernel.atomic_add_at(COUNTS, instruction, one)?;
    let safe = at_most(&mut kernel, slot, COLONY - 1)?;
    let stride = kernel.module().constant_u32(COLONY)?;
    let head = kernel.module().i_mul(uint, instruction, stride)?;
    let into = kernel.module().i_add(uint, head, safe)?;
    kernel.store_at(LIST, into, who)?;
    kernel.finish()
}

/// One compare-exchange, over the list of organisms that asked for it.
///
/// **Fifteen of these are generated rather than written**, one for each pair, which is what an
/// emitter buys over a shader language: the instruction set is a `for` loop in Rust.
fn swap(subgroup: u32, instruction: u32, first: u32, second: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let at = kernel.module().constant_u32(instruction)?;
    let held = kernel.load_at(COUNTS, at)?;
    let who = listed(&mut kernel, subgroup, instruction, held)?;

    let span = kernel.module().constant_u32(VALUES * INPUTS)?;
    let base = kernel.module().i_mul(uint, who, span)?;

    // **`min` is and, `max` is or**, for values that are nought or one. So one comparator over all
    // sixty-four inputs is sixty-four of the same instruction, and the whole nought-one test is one
    // pass rather than sixty-four generations.
    for input in 0..INPUTS {
        let off_a = kernel.module().constant_u32(first * INPUTS + input)?;
        let off_b = kernel.module().constant_u32(second * INPUTS + input)?;
        let at_a = kernel.module().i_add(uint, base, off_a)?;
        let at_b = kernel.module().i_add(uint, base, off_b)?;
        let a = kernel.load_at(WORK, at_a)?;
        let b = kernel.load_at(WORK, at_b)?;
        let (low, high) = {
            let mut lanes = kernel.lanes()?;
            let x = lanes.splat_id::<U32, 1>(a)?;
            let y = lanes.splat_id::<U32, 1>(b)?;
            let small = lanes.min(x, y)?.id();
            let large = lanes.max(x, y)?.id();
            (small, large)
        };
        kernel.store_at(WORK, at_a, low)?;
        kernel.store_at(WORK, at_b, high)?;
    }
    kernel.finish()
}

/// Put the same six numbers in front of every organism, drawn from the tick.
///
/// **The same six**, so the tournament below compares two genomes on one puzzle rather than on two.
fn reset(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let who = mine(&mut kernel, subgroup)?;
    let span = kernel.module().constant_u32(VALUES * INPUTS)?;
    let base = kernel.module().i_mul(uint, who, span)?;

    // Element `k` of input `x` is bit `k` of `x`, and both are known here, so this is a store of a
    // constant — no arithmetic and no generator.
    for element in 0..VALUES {
        for input in 0..INPUTS {
            let bit = kernel.module().constant_u32((input >> element) & 1)?;
            let off = kernel.module().constant_u32(element * INPUTS + input)?;
            let at = kernel.module().i_add(uint, base, off)?;
            kernel.store_at(WORK, at, bit)?;
        }
    }
    kernel.finish()
}

/// How many of the fifteen pairs came out in order — nought to fifteen, and fifteen is sorted.
fn tally(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let who = mine(&mut kernel, subgroup)?;
    let span = kernel.module().constant_u32(VALUES * INPUTS)?;
    let base = kernel.module().i_mul(uint, who, span)?;
    let one = kernel.module().constant_u32(1)?;
    let mut fit = kernel.module().constant_u32(0)?;

    // **Adjacent pairs only.** A sequence is in order exactly when every neighbouring pair is, so
    // five comparisons an input say everything fifteen would.
    for first in 0..VALUES - 1 {
        for input in 0..INPUTS {
            let off_a = kernel.module().constant_u32(first * INPUTS + input)?;
            let off_b = kernel.module().constant_u32((first + 1) * INPUTS + input)?;
            let at_a = kernel.module().i_add(uint, base, off_a)?;
            let at_b = kernel.module().i_add(uint, base, off_b)?;
            let a = kernel.load_at(WORK, at_a)?;
            let b = kernel.load_at(WORK, at_b)?;
            let wrong = above(&mut kernel, a, b)?;
            let ordered = minus(&mut kernel, one, wrong)?;
            fit = kernel.module().i_add(uint, fit, ordered)?;
        }
    }
    kernel.store_at(SCORE, who, fit)?;
    kernel.finish()
}

/// Take the neighbour's genome where the neighbour did better, and change one instruction of it.
///
/// **Half the colony at a time.** An organism reads its neighbour's genome while other invocations
/// are writing theirs, so on an even generation only the even organisms take and on an odd one only
/// the odd ones — the genomes being read from are the ones standing still. `advance` keeps the
/// parity.
fn choose(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let who = mine(&mut kernel, subgroup)?;
    let one = kernel.module().constant_u32(1)?;

    let at_parity = kernel.module().constant_u32(2)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let turn = low_bits(&mut kernel, who, 1)?;
    let my_turn = is(&mut kernel, turn, parity)?;

    let raised = kernel.module().i_add(uint, who, one)?;
    let neighbour = low_bits(&mut kernel, raised, COLONY.trailing_zeros())?;
    let ours = kernel.load_at(SCORE, who)?;
    let theirs = kernel.load_at(SCORE, neighbour)?;
    let better = above(&mut kernel, theirs, ours)?;
    let take = kernel.module().i_mul(uint, my_turn, better)?;

    // **Mutating only on a copy is how a colony freezes.** Once every genome scores alike nobody
    // copies, so nobody changes, and the search stops with the population sitting on one answer.
    // So: copy where the neighbour did better, and change one instruction where the neighbour did
    // as well or better. An organism ahead of its neighbour is left alone, which is the whole of
    // the elitism here.
    let ahead = above(&mut kernel, ours, theirs)?;
    let one_more = kernel.module().constant_u32(1)?;
    let not_ahead = minus(&mut kernel, one_more, ahead)?;
    let restless = kernel.module().i_mul(uint, my_turn, not_ahead)?;

    let seed = kernel.load_at(SEED, who)?;
    let spun = stir(&mut kernel, seed)?;
    let picked = low_bits(&mut kernel, spun, 4)?;
    let spot = at_most(&mut kernel, picked, GENOME - 1)?;
    let again = stir(&mut kernel, spun)?;
    let drawn = low_bits(&mut kernel, again, 4)?;
    let fresh = at_most(&mut kernel, drawn, INSTRUCTIONS - 1)?;

    let span = kernel.module().constant_u32(GENOME)?;
    let ours_at = kernel.module().i_mul(uint, who, span)?;
    let theirs_at = kernel.module().i_mul(uint, neighbour, span)?;
    for step in 0..GENOME {
        let off = kernel.module().constant_u32(step)?;
        let here = kernel.module().i_add(uint, ours_at, off)?;
        let there = kernel.module().i_add(uint, theirs_at, off)?;
        let held = kernel.load_at(CODE, here)?;
        let other = kernel.load_at(CODE, there)?;
        let copied = pick(&mut kernel, take, other, held)?;
        let at_spot = is(&mut kernel, spot, off)?;
        let changing = kernel.module().i_mul(uint, at_spot, restless)?;
        let settled = pick(&mut kernel, changing, fresh, copied)?;
        kernel.store_at(CODE, here, settled)?;
    }
    let last = stir(&mut kernel, again)?;
    kernel.store_at(SEED, who, last)?;
    kernel.finish()
}

// ---------- the host, which submits one recording and asks almost nothing ----------

/// The fifteen compare-exchanges on six elements, in instruction order.
fn pairs() -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for first in 0..VALUES {
        for second in first + 1..VALUES {
            out.push((first, second));
        }
    }
    out
}

/// One step of the same generator the kernels use, so the seeding matches in spirit.
fn stirred(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

/// Run `genome` over `given` and say what comes out.
fn run(genome: &[u32], given: &[u32], wiring: &[(u32, u32)]) -> Vec<u32> {
    let mut held = given.to_vec();
    for &instruction in genome {
        let Some(&(first, second)) = wiring.get(instruction as usize) else {
            continue;
        };
        let (Some(a), Some(b)) = (
            held.get(first as usize).copied(),
            held.get(second as usize).copied(),
        ) else {
            continue;
        };
        if let Some(slot) = held.get_mut(first as usize) {
            *slot = a.min(b);
        }
        if let Some(slot) = held.get_mut(second as usize) {
            *slot = a.max(b);
        }
    }
    held
}

/// The device's own measure, worked out again here: adjacent pairs in order over every nought-one
/// input.
///
/// **Two ways of asking the same thing.** If this disagrees with `SCORE` the kernel is wrong; if it
/// agrees with `SCORE` and `perfect` says otherwise, the principle is being misapplied.
fn zero_one(genome: &[u32], wiring: &[(u32, u32)]) -> u32 {
    let mut ordered = 0;
    for input in 0..INPUTS {
        let given: Vec<u32> = (0..VALUES).map(|k| (input >> k) & 1).collect();
        let out = run(genome, &given, wiring);
        let ok = out
            .windows(2)
            .filter(|two| matches!((two.first(), two.last()), (Some(a), Some(b)) if a <= b))
            .count();
        ordered += u32::try_from(ok).unwrap_or(0);
    }
    ordered
}

/// How many of the 720 orderings of six distinct numbers this genome sorts.
///
/// **This is the check the whole thing is for.** Fitness on the card is one puzzle a generation, so
/// a genome can be lucky; a network either sorts every permutation or it is not a sorting network.
fn perfect(genome: &[u32], wiring: &[(u32, u32)]) -> u32 {
    let check = |order: &[u32]| {
        let out = run(genome, order, wiring);
        u32::from(
            out.windows(2)
                .all(|two| matches!((two.first(), two.last()), (Some(a), Some(b)) if a <= b)),
        )
    };
    let mut order: Vec<u32> = (0..VALUES).collect();
    let mut counters = vec![0_usize; VALUES as usize];
    let mut sorted = check(&order);
    let mut at = 0;
    // Heap's algorithm, which walks every ordering in 720 swaps of two.
    while at < VALUES as usize {
        let held = counters.get(at).copied().unwrap_or(0);
        if held < at {
            order.swap(if at % 2 == 0 { 0 } else { held }, at);
            sorted += check(&order);
            if let Some(slot) = counters.get_mut(at) {
                *slot = held + 1;
            }
            at = 0;
        } else {
            if let Some(slot) = counters.get_mut(at) {
                *slot = 0;
            }
            at += 1;
        }
    }
    sorted
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    let wiring = pairs();
    println!("{} — subgroup {width}", host.name());

    // The instruction set, generated. Fifteen kernels that differ in two constants each.
    let mut modules = vec![
        advance(width)?,
        reset(width)?,
        choose(width)?,
        route(width)?,
        ready(width)?,
    ];
    for (instruction, &(first, second)) in wiring.iter().enumerate() {
        modules.push(swap(width, u32::try_from(instruction)?, first, second)?);
    }
    modules.push(tally(width)?);
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();

    let over = Groups::Fixed(COLONY / width);
    let gate = |at: u32| Groups::From {
        binding: DISPATCH as usize,
        at: at as usize,
    };
    let swaps: Vec<Groups> = (0..INSTRUCTIONS).map(|k| gate(k * 3)).collect();
    // **Selection at the head of a generation, not the tail.** `choose` reads the scores the last
    // generation left and rewrites the genomes before `route` reads them, so `SCORE` and `CODE` are
    // always of the same generation when a submission ends — which is what makes the check at the
    // bottom of this file mean anything.
    let stages: Vec<Vec<Groups>> = vec![
        vec![Groups::Counting(1)],
        vec![gate(RESET_AT), gate(SELECT_AT)],
        vec![over],
        vec![Groups::Counting(1)],
        swaps,
        vec![gate(SCORE_AT)],
    ];
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    println!(
        "  {} passes in {} stages, {INSTRUCTIONS} of them the instruction set",
        modules.len(),
        stages.len()
    );

    let sizes = [
        4,
        (COLONY * GENOME) as usize,
        ((COLONY + 1) * VALUES * INPUTS) as usize,
        (COLONY + 1) as usize,
        (COLONY + 1) as usize,
        (INSTRUCTIONS * COLONY) as usize,
        INSTRUCTIONS as usize,
        (INSTRUCTIONS * 3 + 9) as usize,
    ];
    let mut colony = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        colony.write(binding, &vec![0_u32; *words])?;
    }

    // **The step starts at the end**, so the first tick wraps it to nought and `reset` fires.
    colony.write(PARAMS as usize, &[0, GENOME - 1, 0, 0])?;
    let mut seed = 0x5EED_BEEF_u32;
    let code: Vec<u32> = (0..COLONY * GENOME)
        .map(|_| stirred(&mut seed) % INSTRUCTIONS)
        .collect();
    colony.write(CODE as usize, &code)?;
    let seeds: Vec<u32> = (0..=COLONY).map(|_| stirred(&mut seed)).collect();
    colony.write(SEED as usize, &seeds)?;

    let batches = GENERATIONS * GENOME / BATCH;
    let mut spent = Duration::ZERO;
    println!("\n  {:>12}  {:>10}  {:>9}  {:>10}", "generation", "best", "mean", "us a tick");
    for batch in 0..batches {
        spent += colony.staged_frames(&groups, BATCH)?;
        // **Asked for, on purpose.** Ten times in twenty thousand generations, so the numbers can be
        // seen to exist. The loop above does not need them and never waits for one.
        if batch % (batches / 10).max(1) == 0 || batch + 1 == batches {
            let scores = colony.read(SCORE as usize, COLONY as usize)?;
            let best = scores.iter().copied().max().unwrap_or(0);
            let mean = scores.iter().map(|&s| u64::from(s)).sum::<u64>() / u64::from(COLONY);
            let ticks = (batch + 1) * BATCH;
            println!(
                "  {:>12}  {best:>10}  {mean:>9}  {:>10.3}",
                ticks / GENOME,
                colony.on_device().unwrap_or_default().as_secs_f64() * 1e6
            );
        }
    }

    let scores = colony.read(SCORE as usize, COLONY as usize)?;
    let code = colony.read(CODE as usize, (COLONY * GENOME) as usize)?;
    let (winner, _) = scores
        .iter()
        .enumerate()
        .max_by_key(|&(_, s)| *s)
        .unwrap_or((0, &0));
    let genome = code
        .get(winner * GENOME as usize..(winner + 1) * GENOME as usize)
        .unwrap_or(&[]);

    println!("\n  the best genome, as comparators:");
    print!("   ");
    for &instruction in genome {
        if let Some(&(a, b)) = wiring.get(instruction as usize) {
            print!(" ({a},{b})");
        }
    }
    println!();
    let all = perfect(genome, &wiring);
    println!(
        "  the same genome, scored the way the card scores it: {} of {PERFECT}",
        zero_one(genome, &wiring)
    );
    println!(
        "\n  it sorts {all} of 720 orderings — {}",
        if all == 720 {
            "a sorting network. The shortest on six elements is twelve comparators, and this is not
  that: the genome has room for sixteen and a colony this size does not find the twelve"
        } else {
            "not a sorting network"
        }
    );
    println!(
        "  {batches} submissions, {:.1?} of host clock, {:.3} us a tick",
        spent,
        spent.as_secs_f64() * 1e6 / f64::from(batches * BATCH)
    );
    Ok(())
}
