//! `cargo run --release --bin ticket` — a mask over a random matrix, found without a derivative.
//!
//! # What this is, and what it is not
//!
//! **The Lottery Ticket Hypothesis does not say this.** Frankle and Carbin say a dense random
//! network contains a subnetwork that, *retrained from the same initialisation*, matches the whole:
//! the ticket still has to be trained. The stronger claim — that a random network already contains a
//! subnetwork which works with **no weight ever changed** — is Ramanujan and others, and they find
//! the mask with gradients.
//!
//! This finds one without them. A matrix of 513 weights is drawn once and never touched again.
//! Every organism carries one bit a connection, and selection alone decides which bits stay on. No
//! derivative is computed anywhere. It is a search over 2⁵¹³ masks by mutation and tournament, which
//! is not training and does not pretend to be.
//!
//! # What it finds
//!
//! ```text
//!   6 inputs and a bias  →  64 hidden  →  1 output
//! ```
//!
//! The task is *at least four of the six inputs are one*, on all 64 inputs there are — so the score
//! is exhaustive rather than sampled, and 64 means it. In a thousand generations of 65 536 masks,
//! about eight seconds, one comes out that is right about all 64, keeping 256 of the 513
//! connections. The host works the same mask out again from the same matrix and agrees.
//!
//! **A wider random network holds a better ticket**, which is the hypothesis's own intuition and is
//! measurable here. Ten thousand generations of 4096 masks, best of 64 reached:
//!
//! ```text
//!   hidden   connections   best   a generation
//!       16           129     58        129.9 µs
//!       32           257     62        307.6 µs
//!       64           513     64       1307.7 µs
//!      128          1025     64       3954.0 µs
//! ```
//!
//! At 129 and 257 connections the search plateaus and stays there — forty thousand generations at
//! 257 does not move off 62. Widening is what raises the ceiling, not patience.
//!
//! # The one thing here that is grit's, and what it turned out to be worth
//!
//! A mask that did not change this generation has the score it had last generation, so it does not
//! need evaluating again. `select` appends the ones that changed to a list, one pass turns that
//! length into a workgroup count (`Groups::Counting`), and `evaluate` is dispatched over it
//! (`Groups::From`). The host never learns how long the list is.
//!
//! **It is worth nine per cent, and that is the honest figure.** At 65 536 masks, 28 545 of them
//! change in a generation — 44% — and the generation runs in 5273 µs against 5819 with the same
//! kernel forced over all of them. Two reasons it is not 44%:
//!
//! * `select` has to touch every mask, because each decides for itself, and it is the expensive
//!   pass. Shortening `evaluate` shortens the smaller half.
//! * At 4096 masks it saves **nothing at all** — 1254 µs either way — because 1780 masks is 56
//!   workgroups and a card this size runs 56 as fast as 128. A work list buys nothing on a device
//!   that was not full.
//!
//! That second one is worth more than the demonstration: it is the condition under which
//! `crate::compact` is worth anything, stated as a number rather than assumed.
//!
//! # How a unit fires
//!
//! A weight is a number 0 to 8 standing for −4 to +4, and a unit fires when the masked positive side
//! outweighs the masked negative one — two unsigned sums and one comparison, because the lane API
//! has no signed compare and `decisions/DR-0003` has no branches.
//!
//! `evaluate` is a **rolled** loop over the 64 inputs rather than an unrolled one: 64 × 513 is
//! 32 832 operations, and `Kernel::repeat_rolled` emits one body for all of them.
//!
//! Nothing else in the repository refers to this file. `rm src/bin/ticket.rs` removes it whole.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

/// How many inputs the task has.
const BITS: u32 = 6;
/// Inputs plus the bias, which is always one.
const FAN: u32 = BITS + 1;
/// How wide the hidden layer is.
const HIDDEN: u32 = 64;
/// Connections in the whole network: the hidden layer, then the output and its bias.
const EDGES: u32 = FAN * HIDDEN + HIDDEN + 1;
/// Every input there is, so the score is exhaustive.
const CASES: u32 = 1 << BITS;
/// What a mask has to score to be right about all of them.
const PERFECT: u32 = CASES;
/// How many of the inputs have to be one for the answer to be one.
const WANTED: u32 = 4;

/// How many masks are in the arena. A power of two: the wrap is a shift.
const COLONY: u32 = 1 << 16;
/// How many generations to run.
const GENERATIONS: u32 = 1_000;
/// How many go into one submission.
const BATCH: u32 = 200;

/// How many words every kernel declares.
const BINDINGS: u32 = 9;

/// Four words: the tick and the parity.
const PARAMS: u32 = 0;
/// [`EDGES`] words: the matrix, drawn once and never written again.
const WEIGHT: u32 = 1;
/// [`CASES`] words: what the answer should be for each input.
const TARGET: u32 = 2;
/// [`EDGES`] words a mask, and one more mask past the end that nothing reads.
const MASK: u32 = 3;
/// One word a mask: how many of the [`CASES`] it gets right.
const SCORE: u32 = 4;
/// One word a mask: its own pseudo-random state.
const SEED: u32 = 5;
/// The masks that changed this generation, which are the only ones worth evaluating.
const CHANGED: u32 = 6;
/// One word: how many of them there were. Written by an atomic, read by `ready`.
const COUNTS: u32 = 7;
/// Three words Vulkan reads as `evaluate`'s workgroup count.
const DISPATCH: u32 = 8;
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

/// Which mask this invocation is acting on, and `COLONY` — the dump — where it is acting on none.
///
/// **The dump is the whole of the bounds check.** A dispatch is rounded up to whole workgroups, so
/// the last one holds lanes past the end of the list; they read a clamped slot, write to `COLONY`,
/// and nothing reads `COLONY`. `crate::compact` calls the same slot a dump and uses it for the same
/// reason.
fn listed(kernel: &mut Kernel<U32>, subgroup: u32, held: Id) -> Result<Id, LaneError> {
    let slot = mine(kernel, subgroup)?;
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

/// Bit `at` of `held`.
fn bit(kernel: &mut Kernel<U32>, held: Id, at: u32) -> Result<Id, LaneError> {
    let shifted = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let by = lanes.splat_bits::<U32, 1>(at)?;
        lanes.shift_right_logical(now, by)?.id()
    };
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

/// Advance the tick and the parity, and clear the list of masks that changed.
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

/// Take the neighbour's mask where the neighbour did better, change one bit of it, and say so.
///
/// **Half the colony at a time**, as `alife` does and for the same reason: a mask is read while
/// others are being written, so on an even generation the even masks take and on an odd one the odd
/// ones do. The ones being read from are standing still.
///
/// A mask that changed is appended to [`CHANGED`], and that list is the only thing `evaluate` runs
/// over. **The host never learns how long it is.**
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

    let raised = kernel.module().i_add(uint, who, one)?;
    let neighbour = low_bits(&mut kernel, raised, COLONY.trailing_zeros())?;
    let ours = kernel.load_at(SCORE, who)?;
    let theirs = kernel.load_at(SCORE, neighbour)?;
    let better = above(&mut kernel, theirs, ours)?;
    let take = kernel.module().i_mul(uint, my_turn, better)?;

    // Copy where the neighbour did better, change one bit where it did as well or better, and leave
    // a mask that is ahead of its neighbour alone. `alife` learned that mutating only on a copy is
    // how a colony freezes.
    let ahead = above(&mut kernel, ours, theirs)?;
    let not_ahead = minus(&mut kernel, one, ahead)?;
    let restless = kernel.module().i_mul(uint, my_turn, not_ahead)?;

    let seed = kernel.load_at(SEED, who)?;
    let spun = stir(&mut kernel, seed)?;
    let drawn = low_bits(&mut kernel, spun, EDGES.next_power_of_two().trailing_zeros())?;
    let spot = at_most(&mut kernel, drawn, EDGES - 1)?;

    let span = kernel.module().constant_u32(EDGES)?;
    let ours_at = kernel.module().i_mul(uint, who, span)?;
    let theirs_at = kernel.module().i_mul(uint, neighbour, span)?;
    for edge in 0..EDGES {
        let off = kernel.module().constant_u32(edge)?;
        let here = kernel.module().i_add(uint, ours_at, off)?;
        let there = kernel.module().i_add(uint, theirs_at, off)?;
        let held = kernel.load_at(MASK, here)?;
        let other = kernel.load_at(MASK, there)?;
        let copied = pick(&mut kernel, take, other, held)?;
        // The bit at `spot` turns over rather than being drawn again, so a change is always a
        // change — a redraw that landed on the same value would leave the score right and the mask
        // on the list for nothing.
        let at_spot = is(&mut kernel, spot, off)?;
        let flipping = kernel.module().i_mul(uint, at_spot, restless)?;
        let turned = minus(&mut kernel, one, copied)?;
        let settled = pick(&mut kernel, flipping, turned, copied)?;
        kernel.store_at(MASK, here, settled)?;
    }
    let last = stir(&mut kernel, spun)?;
    kernel.store_at(SEED, who, last)?;

    // Anything that changed goes on the list, and nothing else does.
    let slot = kernel.atomic_add_at(COUNTS, none, restless)?;
    let safe = at_most(&mut kernel, slot, COLONY - 1)?;
    let landing = pick(&mut kernel, restless, safe, none)?;
    let standing = kernel.load_at(CHANGED, landing)?;
    let keeping = pick(&mut kernel, restless, who, standing)?;
    kernel.store_at(CHANGED, landing, keeping)?;
    kernel.finish()
}

/// Turn the length of that list into a workgroup count.
fn ready(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;
    let pad = kernel.module().constant_u32(subgroup - 1)?;

    let held = kernel.load_at(COUNTS, none)?;
    let bumped = kernel.module().i_add(uint, held, pad)?;
    let groups = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(bumped)?;
        let by = lanes.splat_bits::<U32, 1>(subgroup.trailing_zeros())?;
        lanes.shift_right_logical(now, by)?.id()
    };
    kernel.store_at(DISPATCH, none, groups)?;
    let at_y = kernel.module().constant_u32(1)?;
    kernel.store_at(DISPATCH, at_y, one)?;
    let at_z = kernel.module().constant_u32(2)?;
    kernel.store_at(DISPATCH, at_z, one)?;
    kernel.finish()
}

/// Run the masked network over every input there is, and count the ones it gets right.
///
/// **A rolled loop, not an unrolled one.** Sixty-four inputs times 257 connections is 16 448
/// operations and `Kernel::repeat_rolled` emits one body for all of them, which is the difference
/// between a shader and a shader that will not compile.
///
/// A unit fires when the masked positive side outweighs the masked negative one. Two unsigned sums
/// and one comparison: the lane API has no signed compare, and a branch is what
/// `decisions/DR-0003` refuses.
fn evaluate(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let who = listed(&mut kernel, subgroup, held)?;
    let span = kernel.module().constant_u32(EDGES)?;
    let base = kernel.module().i_mul(uint, who, span)?;

    let right = kernel.repeat_rolled(CASES, uint, none, |kernel, carried, case| {
        let uint = kernel.index_type();
        let four = kernel.module().constant_u32(4)?;

        // The hidden layer.
        let mut fired = Vec::with_capacity(HIDDEN as usize);
        for unit in 0..HIDDEN {
            let mut up = kernel.module().constant_u32(0)?;
            let mut down = kernel.module().constant_u32(0)?;
            for input in 0..FAN {
                let value = if input < BITS {
                    bit(kernel, case, input)?
                } else {
                    kernel.module().constant_u32(1)?
                };
                let edge = kernel.module().constant_u32(unit * FAN + input)?;
                let weight = kernel.load_at(WEIGHT, edge)?;
                let at = kernel.module().i_add(uint, base, edge)?;
                let mask = kernel.load_at(MASK, at)?;
                let live = kernel.module().i_mul(uint, mask, value)?;
                let plus = minus(kernel, weight, four)?;
                let less = minus(kernel, four, weight)?;
                let a = kernel.module().i_mul(uint, live, plus)?;
                let b = kernel.module().i_mul(uint, live, less)?;
                up = kernel.module().i_add(uint, up, a)?;
                down = kernel.module().i_add(uint, down, b)?;
            }
            fired.push(above(kernel, up, down)?);
        }

        // The output, over the hidden layer and its own bias.
        let mut up = kernel.module().constant_u32(0)?;
        let mut down = kernel.module().constant_u32(0)?;
        for unit in 0..=HIDDEN {
            let value = if unit < HIDDEN {
                *fired.get(unit as usize).unwrap_or(&none)
            } else {
                kernel.module().constant_u32(1)?
            };
            let edge = kernel.module().constant_u32(FAN * HIDDEN + unit)?;
            let weight = kernel.load_at(WEIGHT, edge)?;
            let at = kernel.module().i_add(uint, base, edge)?;
            let mask = kernel.load_at(MASK, at)?;
            let live = kernel.module().i_mul(uint, mask, value)?;
            let plus = minus(kernel, weight, four)?;
            let less = minus(kernel, four, weight)?;
            let a = kernel.module().i_mul(uint, live, plus)?;
            let b = kernel.module().i_mul(uint, live, less)?;
            up = kernel.module().i_add(uint, up, a)?;
            down = kernel.module().i_add(uint, down, b)?;
        }
        let said = above(kernel, up, down)?;
        let wanted = kernel.load_at(TARGET, case)?;
        let hit = is(kernel, said, wanted)?;
        Ok(kernel.module().i_add(uint, carried, hit)?)
    })?;
    kernel.store_at(SCORE, who, right)?;
    kernel.finish()
}

// ---------- the host, which draws the matrix once and then asks almost nothing ----------

/// One step of the same generator the kernels use.
fn stirred(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

/// What the answer should be: at least [`WANTED`] of the inputs are one.
fn wanted(case: u32) -> u32 {
    u32::from(case.count_ones() >= WANTED)
}

/// The masked network, worked out again here, exactly as `evaluate` does it on the card.
fn says(weights: &[u32], mask: &[u32], case: u32) -> u32 {
    let side = |edge: u32, value: u32| {
        let weight = weights.get(edge as usize).copied().unwrap_or(4);
        let live = mask.get(edge as usize).copied().unwrap_or(0) * value;
        (live * weight.saturating_sub(4), live * 4_u32.saturating_sub(weight))
    };
    let mut fired = Vec::with_capacity(HIDDEN as usize);
    for unit in 0..HIDDEN {
        let (mut up, mut down) = (0, 0);
        for input in 0..FAN {
            let value = if input < BITS { (case >> input) & 1 } else { 1 };
            let (a, b) = side(unit * FAN + input, value);
            up += a;
            down += b;
        }
        fired.push(u32::from(up > down));
    }
    let (mut up, mut down) = (0, 0);
    for unit in 0..=HIDDEN {
        let value = if unit < HIDDEN {
            fired.get(unit as usize).copied().unwrap_or(0)
        } else {
            1
        };
        let (a, b) = side(FAN * HIDDEN + unit, value);
        up += a;
        down += b;
    }
    u32::from(up > down)
}

/// How many of the [`CASES`] this mask gets right, worked out on the host.
fn scored(weights: &[u32], mask: &[u32]) -> u32 {
    (0..CASES)
        .filter(|&case| says(weights, mask, case) == wanted(case))
        .count()
        .try_into()
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}", host.name());

    let modules = [advance(width)?, select(width)?, ready(width)?, evaluate(width)?];
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();
    let stages: [Vec<Groups>; 4] = [
        vec![Groups::Counting(1)],
        vec![Groups::Fixed(COLONY / width)],
        vec![Groups::Counting(1)],
        vec![Groups::From {
            binding: DISPATCH as usize,
            at: 0,
        }],
    ];
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    println!(
        "  {EDGES} connections, {COLONY} masks, {CASES} inputs — {} passes in {} stages",
        modules.len(),
        stages.len()
    );

    let sizes = [
        4,
        EDGES as usize,
        CASES as usize,
        ((COLONY + 1) * EDGES) as usize,
        (COLONY + 1) as usize,
        (COLONY + 1) as usize,
        COLONY as usize,
        1,
        3,
    ];
    let mut arena = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        arena.write(binding, &vec![0_u32; *words])?;
    }

    // **Drawn once.** Nothing writes this binding again, and no derivative is taken of it.
    let mut seed = 0x10D0_7C0Du32;
    let weights: Vec<u32> = (0..EDGES).map(|_| stirred(&mut seed) % 9).collect();
    arena.write(WEIGHT as usize, &weights)?;
    let answers: Vec<u32> = (0..CASES).map(wanted).collect();
    arena.write(TARGET as usize, &answers)?;
    let masks: Vec<u32> = (0..(COLONY + 1) * EDGES)
        .map(|_| stirred(&mut seed) & 1)
        .collect();
    arena.write(MASK as usize, &masks)?;
    let seeds: Vec<u32> = (0..=COLONY).map(|_| stirred(&mut seed)).collect();
    arena.write(SEED as usize, &seeds)?;

    let batches = GENERATIONS / BATCH;
    let mut spent = Duration::ZERO;
    println!(
        "\n  {:>11}  {:>12}  {:>8}  {:>9}  {:>10}",
        "generation",
        format!("best of {PERFECT}"),
        "mean",
        "evaluated",
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
                "  {:>11}  {:>12}  {mean:>8}  {changed:>9}  {:>10.3}",
                (batch + 1) * BATCH,
                best,
                arena.on_device().unwrap_or_default().as_secs_f64() * 1e6
            );
        }
    }

    let scores = arena.read(SCORE as usize, COLONY as usize)?;
    let masks = arena.read(MASK as usize, ((COLONY + 1) * EDGES) as usize)?;
    let (winner, _) = scores
        .iter()
        .enumerate()
        .max_by_key(|&(_, s)| *s)
        .unwrap_or((0, &0));
    let mask = masks
        .get(winner * EDGES as usize..(winner + 1) * EDGES as usize)
        .unwrap_or(&[]);
    let live = mask.iter().filter(|&&bit| bit == 1).count();
    let again = scored(&weights, mask);
    println!(
        "\n  the best mask keeps {live} of {EDGES} connections and, worked out on the host, gets\n  \
         {again} of {PERFECT} right — the card said {}",
        scores.get(winner).copied().unwrap_or(0)
    );
    println!(
        "  {batches} submissions, {:.3} us a generation of host clock",
        spent.as_secs_f64() * 1e6 / f64::from(batches)
    );
    Ok(())
}
