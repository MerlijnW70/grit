//! The sand drives the compaction, and the compaction drives the sand.
//!
//! Everything before this was two systems: a settle over the whole grid, and a compaction of a
//! predicate that had nothing to do with it. This is the loop that makes them one.
//!
//! ```text
//!   decay    the hot list   every wake counter, one closer to nothing
//!   brush    a small box    what the pointer holds, and the quads it touched, woken
//!   settle   the work list  one Margolus step over what moved last frame
//!   hot ×5   tiles          which blocks hold anything, compacted into a list
//!   mark     the hot list   the flags, scanned into buckets and counts
//!   scan ×2                 the blocks against each other
//!   gather   the hot list   the buckets, into one contiguous work list
//!   prepare  1              how many workgroups the *next* settle runs
//!   paint    every pixel    the cells the window is over, as colours
//! ```
//!
//! [`wired`] is that table as code, and the one place the order is decided.
//!
//! # Waking, and why nine writes race with nothing
//!
//! A quad that changed must be looked at again, and so must its neighbours: material crosses a
//! block boundary when the partition's origin shifts, so what moves in one quad this frame is in a
//! different quad next frame. Each moving quad therefore writes a **two** for itself and its eight
//! neighbours, and [`decay`] takes one off every frame — a two-frame window, for the reason that
//! function gives.
//!
//! Nine invocations may write the same flag in the same submission and that is not a race, because
//! **they all write the number one**. `Kernel::store_at` documents exactly this: identical writes to
//! one address need no ordering. A flag that counted rather than marked would need an atomic; a flag
//! that marks needs nothing.
//!
//! # Why one index space serves both phases
//!
//! The partition alternates between origin `(0,0)` and `(1,1)`, so a quad's four cells are not the
//! same four on consecutive frames. A flag indexed by quad would therefore name the wrong cells half
//! the time — except that the dilation already covers it: a phase-one quad `(x, y)` overlaps
//! phase-zero quads `(x, y)` through `(x+1, y+1)`, and a phase-zero quad overlaps phase-one quads
//! `(x-1, y-1)` through `(x, y)`. Both are inside the eight neighbours, so one index space and a
//! dilation of one is enough for either.
//!
//! # What proves it
//!
//! `tests/live.rs` runs the same pour twice — once over the work list and once over the whole grid —
//! and demands the two grids be **identical**, cell for cell, every frame. A quad that should have
//! woken and did not is a grain that stops, and the full-grid run is a different algorithm rather
//! than a second copy of the same mistake.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};

use crate::compact::{Over, Tier, owned};
use crate::layout::{AWAKE, BINDINGS, DENSE, GRAND, HOT, PARAMS, STATE};
use crate::sand::{self, Grid};

/// What [`wired`] hands back: one module a pass, and the workgroup counts grouped into stages.
///
/// The two are separate lists and have to stay in one order — `Resident` counts a pass index through
/// the flattened stages — which is why [`wired`] builds them as pairs and splits them only on the
/// way out.
pub type Loop = (Vec<Vec<u32>>, Vec<Vec<crate::Groups>>);

/// The frame number, one higher, on the device.
///
/// The one word the host never writes after the first frame — the partition's phase is its low bit,
/// so a host that incremented it would be deciding the phase, and that is the device's business.
///
/// Every invocation of the single workgroup writes the same word, which is the case
/// `Kernel::store_at` documents as safe.
///
/// # Errors
///
/// As [`settle`].
pub fn tick(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();

    let now = kernel.load_at(PARAMS, slot)?;
    let next = kernel.module().i_add(element, now, one)?;
    kernel.store_at(PARAMS, slot, next)?;
    kernel.finish()
}

/// How many blocks it takes before the coarse hierarchy pays for what it costs to build.
///
/// # This is a measurement, not a rule
///
/// The hierarchy shortens `decay`, `mark` and `gather` from a workgroup a block to a workgroup a
/// *listed* block, and building it is five passes that cost 6.2–6.7 µs of device clock at every
/// size. Below this many blocks the three are short enough that the five are not worth running:
/// at 256 blocks the arrangement costs 10 µs of 24, and at 16 384 it saves 84 µs of 109.
///
/// `tests/coarse.rs` measures both arrangements either side of this number, and
/// `decisions/DR-0003` carries the figures.
pub const COARSE_FROM: u32 = 2048;

/// Whether a plan of this many blocks is dispatched over the coarse list.
///
/// [`wired`] asks this; [`arranged`] is how a caller says otherwise.
#[must_use]
pub const fn coarse_pays(blocks: u32) -> bool {
    blocks >= COARSE_FROM
}

/// Every pass of the loop, and how many workgroups each runs, in the order they have to run in.
///
/// # Why this is a function and not a list in four callers
///
/// Because one of the orderings is a correctness property that a flat list does not show. The
/// coarse hierarchy — the five passes that compact [`HOT`] into a list of the blocks with something
/// in them — has to sit **after the settle and before the mark**:
///
/// * after, because `brush` and `settle` are what write [`HOT`], and a list built before them would
///   miss every block woken this frame. Those blocks' quads would never reach the work list, and a
///   grain would stop where the whole-grid run keeps falling — which is the failure `tests/live.rs`
///   exists to catch;
/// * before, because `mark` and `gather` are dispatched over that list and want this frame's.
///
/// [`decay`] reads the same list one frame later, which is sound for the reason that function
/// gives: nothing writes [`HOT`] between the list being built and the next frame's decay.
///
/// Getting that wrong in one of four wirings and right in the other three is exactly the bug this
/// repository writes tests against, so there is one wiring.
///
/// ```text
///    0  tick             1            the frame number, and the partition's phase
///    1  decay            hot list     every wake counter, one closer to nothing
///    2  wipe             1            both tile totals and the tally, back to nothing
///    3  wipe_counts      tiles        every block's count, back to nothing
///    4  brush            a box        what the pointer holds, and the quads it touched
///    5  settle           work list    one Margolus step over what the last frame woke
///    6  hot_mark         tiles      ┐
///    7  hot_scan_counts  hot_tiles  │ the coarse hierarchy: which blocks have
///    8  hot_scan_totals  1          │ anything in them, compacted into a list
///    9  hot_gather       tiles      │
///   10  hot_prepare      1          ┘ and how many workgroups that list is worth
///   11  mark             hot list     the counters, scanned into buckets and counts
///   12  scan_counts      tiles        the blocks against each other
///   13  scan_totals      1
///   14  gather           hot list     the buckets into one contiguous list
///   15  prepare          1            how many workgroups the *next* settle runs
/// ```
///
/// # What comes back is stages, not passes
///
/// A barrier sits between stages and not inside one, and `tests/floor.rs` prices one at 996–1009 ns
/// against 210–245 ns for a dispatch. At a size where no pass has any work to do, ten of them are
/// most of the frame. So the second half of this function is a dependency argument: sixteen passes
/// in eleven stages, or eleven in seven without the coarse hierarchy, and the comment beside them
/// names every seam. [`crate::Resident::staged`] and [`crate::Resident::staged_frames`] are what
/// take them.
///
/// The grouping was worth 60 µs to 44 when it went in, against a barrier that then cost about
/// 3.8 µs — `Resident` named `DRAW_INDIRECT` in all ten of them at the time, and `DR-0002` is what
/// that turned out to be worth.
///
/// # And why the picture is an argument rather than something a caller appends
///
/// Because `Resident` counts a pass index straight through the flattened stages, so the modules and
/// the counts have to be in one order and not two. A caller appending [`sand::paint`] to the end of
/// the module list and to the *middle* of the stage list — which is where it belongs, beside the
/// coarse mark — would dispatch the wrong kernel over the wrong count, and nothing in either list
/// would look wrong. So the two travel as pairs in here and are split apart only on the way out, and
/// `picture` is how a caller says it wants one. `src/bin/scale.rs` passes `None`.
///
/// # Errors
///
/// As [`settle`], for whichever pass could not be built.
pub fn wired(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
) -> Result<Loop, LaneError> {
    arranged(subgroup, grid, plan, picture, coarse_pays(plan.blocks))
}

/// [`wired`], with the coarse hierarchy said out loud instead of decided by [`coarse_pays`].
///
/// # Why the choice is reachable at all
///
/// [`COARSE_FROM`] is a number taken on one card, and the only way to say whether it is the right
/// one is to build both arrangements and time them. `tests/coarse.rs` does that, and it cannot if
/// the choice is not reachable.
///
/// **The two compute the same simulation.** `false` dispatches `decay`, `mark` and `gather` over
/// every block rather than over the coarse list, and leaves out the five passes that build that
/// list. Nothing else moves, and `tests/live.rs` passes either way.
///
/// # Errors
///
/// As [`settle`], for whichever pass could not be built.
pub fn arranged(
    subgroup: u32,
    grid: Grid,
    plan: &crate::compact::Plan,
    picture: Option<sand::View>,
    coarse: bool,
) -> Result<Loop, LaneError> {
    use crate::Groups;
    use crate::layout::{HOT_BASES, HOT_COUNTS, HOT_DENSE, HOT_GRAND, HOT_TOTALS};
    use crate::sweep::COUNTS;

    // **Where the list comes from, and how many workgroups read it.** Over the coarse list, the
    // count is a word the device wrote and Vulkan reads at dispatch time; over every block it is a
    // number the host already knows. The three passes below take whichever pair this is.
    let (listed, hot) = if coarse {
        (Over::Listed(HOT_DENSE), crate::compact::Plan::hot_listed())
    } else {
        (Over::Every, Groups::Fixed(plan.blocks))
    };
    let tiles = Groups::Fixed(plan.tiles);
    let one = Groups::Fixed(1);

    // **A pass and how many workgroups it runs, together.** They used to be two lists that had to
    // stay in the same order, and now that the order is *stages* rather than passes the two could
    // disagree without looking wrong: `Resident` counts a pass index through the flattened stages,
    // so a picture appended to the end of one list and the middle of the other would silently
    // dispatch the wrong kernel over the wrong count. Pairing them makes that unsayable.
    let looking = {
        let mut stage = Vec::new();
        if coarse {
            stage.push((mark(subgroup, Tier::COARSE, Over::Every)?, tiles));
        }
        if let Some(view) = picture {
            // **Beside the coarse mark where there is one, not on the end of the frame.** It reads
            // the state the settle wrote and writes pixels nothing else touches, so it has no reason
            // to wait behind the compaction — and it is the largest fixed dispatch in the loop.
            let over = u32::try_from(view.pixels())
                .unwrap_or(u32::MAX)
                .div_ceil(subgroup);
            stage.push((sand::paint(subgroup, grid, view)?, Groups::Fixed(over)));
        }
        stage
    };

    // **The five that build the coarse list, or nothing at all.** An empty stage is dropped where
    // the pairs are flattened rather than recorded, so leaving these out costs no barrier either.
    let hot_scan_counts = if coarse {
        vec![(
            crate::compact::scan(subgroup, HOT_COUNTS, HOT_BASES, HOT_TOTALS)?,
            Groups::Fixed(plan.hot_tiles),
        )]
    } else {
        Vec::new()
    };
    let hot_scan_totals = if coarse {
        vec![(
            crate::compact::scan(subgroup, HOT_TOTALS, HOT_TOTALS, HOT_GRAND)?,
            one,
        )]
    } else {
        Vec::new()
    };
    let hot_gather = if coarse {
        vec![
            (
                crate::compact::gather(subgroup, Tier::COARSE, Over::Every, plan.hot_dump())?,
                tiles,
            ),
            // **Divided by one**, because this list is dispatched a workgroup an entry rather than a
            // workgroup a subgroup's worth. Dividing by the wrong one is a pass that silently covers
            // part of the list.
            (crate::compact::prepare(subgroup, 1, Tier::COARSE)?, one),
        ]
    } else {
        Vec::new()
    };

    let stages = vec![
        vec![
            (tick(subgroup)?, one),
            (decay(subgroup, listed)?, hot),
            (wipe(subgroup)?, one),
            (wipe_counts(subgroup, COUNTS)?, tiles),
        ],
        vec![(
            sand::brush(subgroup, grid, true)?,
            Groups::Fixed(sand::BOX * sand::BOX / subgroup),
        )],
        vec![(settle(subgroup, grid)?, crate::compact::Plan::listed())],
        looking,
        hot_scan_counts,
        hot_scan_totals,
        hot_gather,
        vec![(mark(subgroup, Tier::FINE, listed)?, hot)],
        vec![(
            crate::compact::scan(subgroup, COUNTS, crate::layout::BASES, crate::layout::TOTALS)?,
            tiles,
        )],
        vec![(
            crate::compact::scan(
                subgroup,
                crate::layout::TOTALS,
                crate::layout::TOTALS,
                crate::layout::GRAND,
            )?,
            one,
        )],
        vec![
            (
                crate::compact::gather(subgroup, Tier::FINE, listed, plan.dump())?,
                hot,
            ),
            (crate::compact::prepare(subgroup, subgroup, Tier::FINE)?, one),
        ],
    ];

    let mut passes = Vec::new();
    let mut groups = Vec::with_capacity(stages.len());
    for stage in stages {
        // **An empty stage is not a stage.** Recording one would put a barrier between two passes
        // with nothing in between, and `tests/floor.rs` prices a barrier at about a microsecond.
        if stage.is_empty() {
            continue;
        }
        let mut counts = Vec::with_capacity(stage.len());
        for (module, over) in stage {
            passes.push(module);
            counts.push(over);
        }
        groups.push(counts);
    }

    Ok((passes, groups))
}

/// Seed every buffer the loop reads before it has written it, for the first frame.
///
/// **The first frame has been told nothing**, so "look at everything" has to be said out loud: every
/// quad awake, every block hot, and both lists the identity. Without the coarse half of this the
/// first `decay` is dispatched over a list of nothing, no block is ever marked, and the world sits
/// perfectly still — which looks exactly like a simulation that does not work.
///
/// # Errors
///
/// As [`crate::Resident::write`].
pub fn seed(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &crate::compact::Plan,
) -> Result<(), crate::Error> {
    use crate::layout::{HOT_DENSE, HOT_GRAND, HOT_INDIRECT, INDIRECT};

    let quads = grid.quads();
    held.write(AWAKE as usize, &vec![2_u32; quads as usize])?;
    held.write(DENSE as usize, &(0..quads).collect::<Vec<u32>>())?;
    held.write(GRAND as usize, &[quads])?;
    held.write(INDIRECT as usize, &[quads.div_ceil(plan.subgroup), 1, 1])?;

    held.write(HOT as usize, &vec![2_u32; plan.blocks as usize])?;
    held.write(HOT_DENSE as usize, &(0..plan.blocks).collect::<Vec<u32>>())?;
    held.write(HOT_GRAND as usize, &[plan.blocks])?;
    held.write(HOT_INDIRECT as usize, &[plan.blocks, 1, 1])
}

/// Every wake counter, one closer to nothing.
///
/// # Two, and why one is not enough
///
/// A quad that moves is set to two, and this takes one off every frame. That is a **two-frame**
/// window rather than one, and the second frame is the one that makes it sound.
///
/// The partition's origin alternates, so a quad has two different neighbourhoods on consecutive
/// frames. A grain that has just landed on the bottom row of its quad has nowhere to go *this*
/// phase — the cells below it are not in this quad — and somewhere to go the next. Waking only what
/// moved would drop it from the list in between, and it would hang there forever with nothing
/// nearby moving to wake it.
///
/// Two frames covers both phases with the same contents. If it is still static after that it cannot
/// move at all until something near it does, and that something will set it to two again.
///
/// The step down is `max(v, 1) − 1`, because there is no signed compare on the lane API and a
/// counter that wrapped past nothing would be awake for four billion frames.
///
/// One invocation a quad, its own slot, so nothing races. The dump past the counters is left alone —
/// nothing reads it, so nothing has to clear it.
///
/// # Why the list this runs over is the *previous* frame's
///
/// [`Over::Listed`] hands this a list of the blocks with something in them, and it is built after
/// the settle — which is after this pass has already run. That is not a mistake and not a
/// compromise: between the moment the list is built and the moment this reads it, the only thing
/// that runs is the frame counter. **Nothing writes [`HOT`] in between**, so a list one frame old
/// is the current set exactly.
///
/// A block that is not in it holds nought, and `max(0, 1) − 1` is nought, so the pass that was
/// skipped was the pass that would have changed nothing.
///
/// # Errors
///
/// As [`sand::settle`].
pub fn decay(subgroup: u32, over: Over) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => decayed::<4>(subgroup, over),
        8 => decayed::<8>(subgroup, over),
        16 => decayed::<16>(subgroup, over),
        32 => decayed::<32>(subgroup, over),
        64 => decayed::<64>(subgroup, over),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The decay, at a width the caller has matched to the device.
fn decayed<const LANES: u32>(subgroup: u32, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    // **A workgroup a block, not a lane a quad**, and which block comes from the hot-block list
    // rather than from the workgroup's own number. The block is the same one `mark` sweeps, which
    // is what keeps the two in step.
    let block = owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along)?;

    let step = |kernel: &mut Kernel<U32>, held: simdr::module::Id| {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let one = lanes.splat_bits::<U32, 1>(1)?;
        let minus = lanes.splat_bits::<U32, 1>(u32::MAX)?;
        let floored = lanes.max(now, one)?;
        let back = lanes.mul(one, minus)?;
        Ok::<_, LaneError>(lanes.add(floored, back)?.id())
    };

    // **No gate any more.** Every block this is dispatched over is one the list said had something
    // in it, so the vote that used to stand here would pass every time. The saving moved from
    // skipping the body to not being dispatched at all.
    for offset in 0..PER_LANE {
        let along = kernel.module().constant_u32(offset)?;
        let quad = kernel.module().i_add(uint, mine, along)?;
        let held = kernel.load_at(AWAKE, quad)?;
        let lower = step(&mut kernel, held)?;
        kernel.store_at(AWAKE, quad, lower)?;
    }

    // And the block's own counter, written by every lane because every lane has the same answer —
    // the argument `layout::HOT` makes about identical writes, one level up.
    let held = kernel.load_at(HOT, block)?;
    let lower = step(&mut kernel, held)?;
    kernel.store_at(HOT, block, lower)?;
    kernel.finish()
}

/// The tile totals, back to nothing.
///
/// **The in-place scan leaves its answer where the next frame's input goes.** `scan_totals` reads
/// `TOTALS` and writes its exclusive prefix back over it, and the frame after that `scan_counts`
/// writes only the `tiles` entries it has — so the rest still hold last frame's prefix, and the
/// reduction adds them all again. It compounds: the grand total reached three billion by frame
/// sixty-one, the workgroup count with it, and the settle ran over a list that was not there.
///
/// One workgroup, because there are never more than `per_block` tiles — `Plan::over` refuses a
/// third level rather than pretend otherwise.
///
/// # Errors
///
/// As [`settle`].
pub fn wipe(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    use crate::layout::{HOT_TOTALS, TALLY, TALLY_WORDS, TOTALS};
    use crate::sweep::PER_LANE;

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let lane = kernel.local_index();
    let run = kernel.module().constant_u32(PER_LANE)?;
    let mine = kernel.module().i_mul(uint, lane, run)?;
    let nothing = kernel.module().constant_u32(0)?;

    // **Both hierarchies' tile totals**, because both are scanned in place and both would otherwise
    // carry last frame's prefix into this frame's reduction. The coarse one is the same array with
    // one entry where the fine one has a tile, and it goes wrong in exactly the same way.
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        kernel.store_at(TOTALS, at, nothing)?;
        kernel.store_at(HOT_TOTALS, at, nothing)?;
    }

    // **The tally is accumulated with atomics, so it has to start at nothing every frame** — for
    // exactly the reason the tile totals do, and found the same way: a sum that is never cleared is
    // a sum of the whole run, and it looks perfectly plausible until you notice it only goes up.
    // Written by every lane rather than one, because they all write the same nought to the same
    // slot and a uniform store is cheaper than deciding which lane may do it.
    for word in 0..TALLY_WORDS {
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        kernel.store_at(TALLY, at, nothing)?;
    }
    kernel.finish()
}

/// Every block's count, back to nothing.
///
/// # The pass the hot-block list made necessary
///
/// [`mark`] used to write `counts[block] = 0` before its gate, so a block it skipped said *nothing*
/// rather than last frame's *something*. That worked because it was dispatched for every block
/// whether it was hot or not. Dispatched over the hot list instead, a cold block is not visited at
/// all — and nobody writes its count.
///
/// Which matters because `scan_counts` reads **every** block: it is a prefix, and a prefix has to
/// see the zeros. One stale count inflates every base after it and the grand total with them, and
/// the settle then runs over a list whose tail is whatever the buckets happened to hold. That is
/// the same failure `wipe` exists for, arriving by a different road.
///
/// One workgroup a tile, [`PER_LANE`](crate::sweep::PER_LANE) stores a lane, over the padded array
/// — sixty-four workgroups at sixteen million cells, against the forty-nine thousand this whole
/// change is about.
///
/// # Errors
///
/// As [`settle`].
pub fn wipe_counts(subgroup: u32, counts: u32) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along)?;
    let nothing = kernel.module().constant_u32(0)?;

    for step in 0..PER_LANE {
        let at = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, at)?;
        kernel.store_at(counts, at, nothing)?;
    }
    kernel.finish()
}

/// One Margolus step over the quads the work list names, waking what moved.
///
/// The same rule as [`sand::settle`] — the four ordered swaps — over a different set of quads and
/// with the wake writes on the end. Kept apart from that one rather than folded into it, because
/// `tests/live.rs` runs both and compares, and a shared body would agree with itself.
///
/// # Errors
///
/// As [`sand::settle`].
pub fn settle(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => listed::<4>(subgroup, grid),
        8 => listed::<8>(subgroup, grid),
        16 => listed::<16>(subgroup, grid),
        32 => listed::<32>(subgroup, grid),
        64 => listed::<64>(subgroup, grid),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The listed settle, at a width the caller has matched to the device.
fn listed<const LANES: u32>(subgroup: u32, grid: Grid) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let slot = kernel.module().i_add(uint, base, lane)?;

    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let grand = kernel.load_at(GRAND, head)?;
    let second = kernel.module().constant_u32(1)?;
    let dump_base = kernel.load_at(PARAMS, second)?;

    // **The last workgroup reads past the end of the list**, because a workgroup count is a ceiling,
    // and what it reads there is a stale entry the gather did not overwrite. That quad is *not*
    // harmless to step: it may be one another invocation is legitimately stepping this frame, and
    // then the same four cells are written twice with different answers. Both its state writes and
    // its wake writes are therefore sent to a dump instead.
    let inside = {
        let mut lanes = kernel.lanes()?;
        let here = lanes.splat_id::<U32, LANES>(slot)?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let listed = lanes.greater_than(total, here)?;
        lanes.select(listed, one, none)?
    };

    // Which quad, and where its four cells are. A slot past the end names whatever is there, and
    // `low_bits` keeps it inside the grid however wild it is.
    let quad = kernel.load_at(DENSE, slot)?;
    let (places, column, row) = corners::<LANES>(&mut kernel, grid, quad, frame)?;

    let held = [
        kernel.load_at(STATE, places[0].id())?,
        kernel.load_at(STATE, places[1].id())?,
        kernel.load_at(STATE, places[2].id())?,
        kernel.load_at(STATE, places[3].id())?,
    ];

    let (settled, counted) = {
        let mut lanes = kernel.lanes()?;
        let a = lanes.splat_id::<U32, LANES>(held[0])?;
        let b = lanes.splat_id::<U32, LANES>(held[1])?;
        let c = lanes.splat_id::<U32, LANES>(held[2])?;
        let d = lanes.splat_id::<U32, LANES>(held[3])?;

        // **The same seven rules as `sand::settle`, in the same order**, because the whole claim of
        // `tests/live.rs` is that the two runs differ only in *which* quads are stepped. A rule that
        // lived in one of them and not the other would make the comparison meaningless.
        let a = sand::hold::<LANES>(&mut lanes, a, c)?;
        let b = sand::hold::<LANES>(&mut lanes, b, d)?;
        let [sa, sb, sc, sd] = sand::start::<LANES>(&mut lanes, [a, b, c, d])?;
        let (sa, sb) = sand::course::<LANES>(&mut lanes, sa, sb)?;
        let (sc, sd) = sand::course::<LANES>(&mut lanes, sc, sd)?;

        let (na, nc) = sand::fall::<LANES>(&mut lanes, sa, sc)?;
        let (nb, nd) = sand::fall::<LANES>(&mut lanes, sb, sd)?;
        let (na, nd) = sand::fall::<LANES>(&mut lanes, na, nd)?;
        let (nb, nc) = sand::fall::<LANES>(&mut lanes, nb, nc)?;

        // **Does it hold anything that can move**, not *did anything move*. Those are different
        // questions and the second one is wrong: the partition's origin shifts every frame, so a
        // quad with nowhere to go this frame can have somewhere next frame with the same contents.
        // Waking on movement lets a falling stream outrun its own wake front, which is what
        // `tests/live.rs` found at frame 75 — a grain present in the whole-grid run and missing
        // from the listed one.
        //
        // Content is sufficient and provable: a move needs sand, a move happens inside one quad,
        // and next phase that sand lies inside a quad within one of this one. Four comparisons
        // summed, because a `Predicate` is not a value this API combines.
        let mut count = lanes.splat_bits::<U32, LANES>(0)?;
        for (before, after) in [(a, na), (b, nb), (c, nc), (d, nd)] {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let same = lanes.equal(before, after)?;
            let differs = lanes.select(same, none, one)?;
            count = lanes.add(count, differs)?;
        }

        // What this quad holds, for the tally. Counted after the step rather than before, so the
        // numbers describe the world the viewer is about to be shown.
        let mut wet = lanes.splat_bits::<U32, LANES>(0)?;
        let mut dry = lanes.splat_bits::<U32, LANES>(0)?;
        for cell in [na, nb, nc, nd] {
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let water = lanes.splat_bits::<U32, LANES>(sand::WATER)?;
            let grain = lanes.splat_bits::<U32, LANES>(sand::SAND)?;
            let kind = sand::material::<LANES>(&mut lanes, cell)?;
            let is_wet = lanes.equal(kind, water)?;
            let is_dry = lanes.equal(kind, grain)?;
            let add_wet = lanes.select(is_wet, one, none)?;
            let add_dry = lanes.select(is_dry, one, none)?;
            wet = lanes.add(wet, add_wet)?;
            dry = lanes.add(dry, add_dry)?;
        }
        ([na, nb, nc, nd], [count, wet, dry])
    };

    // Where a slot past the end of the list puts its four cells: past the grid, four to an
    // invocation, and never read.
    let spare = {
        let mut lanes = kernel.lanes()?;
        let past =
            lanes.splat_bits::<U32, LANES>(u32::try_from(grid.cells()).unwrap_or(u32::MAX))?;
        let mine = lanes.splat_id::<U32, LANES>(slot)?;
        let four = lanes.splat_bits::<U32, LANES>(2)?;
        let along = lanes.shift_left(mine, four)?;
        lanes.add(past, along)?
    };

    for (corner, (place, value)) in places.iter().zip(&settled).enumerate() {
        let where_at = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let step = lanes.splat_bits::<U32, LANES>(u32::try_from(corner).unwrap_or(0))?;
            let elsewhere = lanes.add(spare, step)?;
            let listed = lanes.greater_than(inside, none)?;
            lanes.select(listed, *place, elsewhere)?
        };
        kernel.store_at(STATE, where_at.id(), value.id())?;
    }

    let where_at = Waking {
        subgroup,
        grid,
        column,
        row,
    };
    let holding = *counted.first().ok_or(LaneError::NoMapping {
        lanes: LANES,
        width: subgroup,
    })?;
    wake::<LANES>(&mut kernel, where_at, holding, inside, dump_base)?;

    let what = Tallying {
        counted,
        column,
        row,
        inside,
    };
    tally::<LANES>(&mut kernel, what)?;
    kernel.finish()
}

/// What [`tally`] needs, gathered so the call is not eight arguments long.
struct Tallying<const LANES: u32> {
    /// Cells changed, water cells and sand cells, per quad.
    counted: [Vector<U32, LANES>; 3],
    /// The quad's own column and row.
    column: Vector<U32, LANES>,
    /// The quad's row.
    row: Vector<U32, LANES>,
    /// One where this slot is a real list entry, nought where it is past the end.
    inside: Vector<U32, LANES>,
}

/// Add this workgroup's share of the frame's numbers into [`TALLY`].
///
/// # The one reduction in a crate about scans
///
/// Every other pass here compacts: it wants to know *which* quads, in what order, and a prefix sum
/// is the only thing that answers that. This wants the opposite — not which, but how many, and one
/// answer for the whole device. That is [`simdr::lanes::Lanes::reduce_sum`], which folds the strips
/// inside a lane and then the lanes across the subgroup, and hands the same total back to every one
/// of them.
///
/// # And why the total is then multiplied by nought in all but one lane
///
/// Because every lane holds it, and every lane is about to execute the atomic. An add is not
/// idempotent, so `width` lanes adding the same total would count the workgroup `width` times.
/// `decisions/DR-0003` refuses the branch that would let only one lane run the instruction, so the
/// instruction runs everywhere and the *value* is what carries the decision: the total in the first
/// lane, nought in the rest. Adding nought is free and lands nowhere.
///
/// The cost is one subgroup reduction and one atomic a workgroup for each of six numbers, on data
/// that was already in registers because the settle had just computed it. **The telemetry is not a
/// pass.**
fn tally<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    what: Tallying<LANES>,
) -> Result<(), LaneError> {
    use crate::layout::{COARSE, TALLY};

    let Tallying {
        counted,
        column,
        row,
        inside,
    } = what;
    let [moved, wet, dry] = counted;
    let first = kernel.local_index();

    let shares = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let one = lanes.splat_bits::<U32, LANES>(1)?;
        let listed = lanes.greater_than(inside, none)?;
        let keep =
            |lanes: &mut Lanes<'_>, value: Vector<U32, LANES>| lanes.select(listed, value, none);

        // A slot past the end of the list stepped a spare quad, and its numbers are not the world's.
        let moved = keep(&mut lanes, moved)?;
        let wet = keep(&mut lanes, wet)?;
        let dry = keep(&mut lanes, dry)?;

        // Coarse, because the sum has to fit in a word — see `layout::COARSE`.
        let by = lanes.splat_bits::<U32, LANES>(COARSE)?;
        let across = lanes.shift_right_logical(column, by)?;
        let down = lanes.shift_right_logical(row, by)?;
        let across = lanes.mul(across, moved)?;
        let down = lanes.mul(down, moved)?;

        // And whether the quad moved at all, which is what the two sums are averaged over.
        let stirred = lanes.greater_than(moved, none)?;
        let stirred = lanes.select(stirred, one, none)?;

        [moved, wet, dry, across, down, stirred]
    };

    for (word, share) in shares.into_iter().enumerate() {
        let total = {
            let mut lanes = kernel.lanes()?;
            lanes.reduce_sum::<U32, LANES>(share)?
        };
        let mine = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let seat = lanes.splat_id::<U32, LANES>(first)?;
            let sum = lanes.splat_id::<U32, LANES>(total)?;
            let leading = lanes.equal(seat, none)?;
            lanes.select(leading, sum, none)?
        };
        let at = kernel
            .module()
            .constant_u32(u32::try_from(word).unwrap_or(0))?;
        // The slot's old value is what makes this instruction an allocator elsewhere. Here it
        // is a counter and nobody wants the answer.
        let _ = kernel.atomic_add_at(TALLY, at, mine.id())?;
    }
    Ok(())
}

/// Where a quad's four cells are, and which quad it was.
type Corners<const LANES: u32> = (
    [Vector<U32, LANES>; 4],
    Vector<U32, LANES>,
    Vector<U32, LANES>,
);

/// The four cell addresses of a quad, at this frame's partition origin.
///
/// Also hands back the quad's own column and row, which the wake writes need.
fn corners<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    grid: Grid,
    quad: simdr::module::Id,
    frame: simdr::module::Id,
) -> Result<Corners<LANES>, LaneError> {
    let mut lanes = kernel.lanes()?;
    let which = lanes.splat_id::<U32, LANES>(quad)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;

    let phase = sand::low_bits::<LANES>(&mut lanes, now, 1)?;
    let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
    let down = lanes.shift_right_logical(which, by)?;
    let along = sand::low_bits::<LANES>(&mut lanes, which, grid.across().trailing_zeros())?;

    let one = lanes.splat_bits::<U32, LANES>(1)?;
    let doubled_x = lanes.shift_left(along, one)?;
    let doubled_y = lanes.shift_left(down, one)?;
    let column = lanes.add(doubled_x, phase)?;
    let row = lanes.add(doubled_y, phase)?;
    let right = lanes.add(column, one)?;
    let under = lanes.add(row, one)?;

    let places = [
        sand::at::<LANES>(&mut lanes, grid, column, row)?,
        sand::at::<LANES>(&mut lanes, grid, right, row)?,
        sand::at::<LANES>(&mut lanes, grid, column, under)?,
        sand::at::<LANES>(&mut lanes, grid, right, under)?,
    ];
    Ok((places, along, down))
}

/// Which quad is doing the waking, and on what device.
pub(crate) struct Waking<const LANES: u32> {
    /// The subgroup's width, which is also the workgroup's.
    pub subgroup: u32,
    /// How big the world is.
    pub grid: Grid,
    /// The quad's column and row.
    pub column: Vector<U32, LANES>,
    pub row: Vector<U32, LANES>,
}

/// Mark this quad and its eight neighbours, if `holding` is anything and this slot was in the list.
///
/// Nine unconditional writes of the number one. Where the quad holds nothing, or the slot was past
/// the end of the list, every one of them goes to this invocation's dump slot instead — and they all
/// go to the *same* one, so it is a cache line touched once rather than nine words moved.
pub(crate) fn wake<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    where_at: Waking<LANES>,
    holding: Vector<U32, LANES>,
    inside: Vector<U32, LANES>,
    dump_base: simdr::module::Id,
) -> Result<(), LaneError> {
    let Waking {
        subgroup,
        grid,
        column,
        row,
    } = where_at;
    let lane = kernel.local_index();
    let group = kernel.workgroup_index();

    for down in 0..3_u32 {
        for across in 0..3_u32 {
            let slot = {
                let mut lanes = kernel.lanes()?;
                let one = lanes.splat_bits::<U32, LANES>(1)?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let minus = lanes.splat_bits::<U32, LANES>(u32::MAX)?;

                // The neighbour, wrapped into the quad grid the way the cells wrap into theirs.
                let along = lanes.splat_bits::<U32, LANES>(across)?;
                let below = lanes.splat_bits::<U32, LANES>(down)?;
                let back = lanes.mul(one, minus)?;
                let near_x = lanes.add(column, along)?;
                let near_x = lanes.add(near_x, back)?;
                let near_y = lanes.add(row, below)?;
                let near_y = lanes.add(near_y, back)?;

                let x =
                    sand::low_bits::<LANES>(&mut lanes, near_x, grid.across().trailing_zeros())?;
                let y = sand::low_bits::<LANES>(&mut lanes, near_y, grid.down().trailing_zeros())?;
                let by = lanes.splat_bits::<U32, LANES>(grid.across().trailing_zeros())?;
                let stride = lanes.shift_left(y, by)?;
                let flag = lanes.add(stride, x)?;

                // Where this invocation's discarded writes go.
                let base = lanes.splat_id::<U32, LANES>(dump_base)?;
                let mine = lanes.splat_id::<U32, LANES>(group)?;
                let seat = lanes.splat_id::<U32, LANES>(lane)?;
                // The workgroup's own width, not the quad grid's — this is an invocation index.
                let width = lanes.splat_bits::<U32, LANES>(subgroup)?;
                let spread = lanes.mul(mine, width)?;
                let elsewhere = lanes.add(base, spread)?;
                let elsewhere = lanes.add(elsewhere, seat)?;

                let both = lanes.mul(holding, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, flag, elsewhere)?
            };
            // Two, not one — see `clear`. One frame is a window that closes on the phase where the
            // move it was waiting for becomes available.
            let mark = kernel.module().constant_u32(2)?;
            kernel.store_at(AWAKE, slot.id(), mark)?;

            // **And the block that quad is in**, with the same number and by the same rule, so the
            // coarse flag can never be colder than the fine one it stands for. A quad index divided
            // by a block's worth of quads, which is a shift because both are powers of two.
            let coarse = {
                let mut lanes = kernel.lanes()?;
                let none = lanes.splat_bits::<U32, LANES>(0)?;
                let per_block = crate::sweep::cells(subgroup);
                let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
                // **Past the padding, not past the last block.** The coarse sweep reads whole
                // tiles, so `HOT` is rounded up to `tiles × per_block` — and a dump slot inside
                // that would be a hot block no quad ever woke, listed every frame for ever.
                // `compact::Plan::cold` is the same number, arrived at from the other side.
                let blocks = grid.quads().div_ceil(per_block);
                let cold = lanes.splat_bits::<U32, LANES>(blocks.div_ceil(per_block) * per_block)?;

                let which = lanes.shift_right_logical(slot, by)?;
                let both = lanes.mul(holding, inside)?;
                let waking = lanes.greater_than(both, none)?;
                lanes.select(waking, which, cold)?
            };
            kernel.store_at(HOT, coarse.id(), mark)?;
        }
    }
    Ok(())
}

/// The wake flags, scanned into buckets and counts — the compaction's first tier.
///
/// [`crate::sweep`] without the simulation in front of it: the predicate is already in the tier's
/// flags, so this reads it, scans it in three tiers, and writes the indices that survive.
/// Everything after it — the two scans, the gather, the count — is `crate::compact` unchanged.
///
/// # One kernel, both hierarchies
///
/// Over [`Tier::FINE`] the flags are one word a quad and this writes the quad indices the settle
/// will step. Over [`Tier::COARSE`] they are one word a *block* and it writes the block indices
/// this pass will itself be dispatched over next frame. Nothing in the body knows which — a
/// countdown above nought is a live entry either way, and the index it writes is the index it read
/// at.
///
/// # Errors
///
/// As [`settle`].
pub fn mark(subgroup: u32, tier: Tier, over: Over) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => marked::<4>(subgroup, tier, over),
        8 => marked::<8>(subgroup, tier, over),
        16 => marked::<16>(subgroup, tier, over),
        32 => marked::<32>(subgroup, tier, over),
        64 => marked::<64>(subgroup, tier, over),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The mark, at a width the caller has matched to the device.
fn marked<const LANES: u32>(subgroup: u32, tier: Tier, over: Over) -> Result<Vec<u32>, LaneError> {
    use crate::sweep::{self, PER_LANE};

    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along_lane)?;

    let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
    let region = kernel.module().i_mul(uint, block, wide)?;
    let past = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let dump_start = kernel.module().i_add(uint, region, past)?;
    let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

    // Blocked, not striped: the two scan tiers add up to a prefix only when (lane, then step) is
    // index order. `tests/compact.rs` found that out the hard way.
    //
    // **The gate is gone and so is the zeroing that stood in front of it.** Every block this is
    // dispatched over is one the hot list named, so the vote would pass every time — and the zero
    // that used to be written here for a skipped block is now [`wipe_counts`]'s job, because a
    // block this never visits is a block this cannot write anything for.
    swept::<LANES>(
        &mut kernel,
        Sweeping {
            mine,
            region,
            dump_mine,
            block,
            tier,
        },
    )?;
    kernel.finish()
}

/// Where one workgroup's sweep reads and writes, gathered so the call is not five arguments long.
struct Sweeping {
    /// The first quad this lane owns.
    mine: simdr::module::Id,
    /// Where this block's bucket begins.
    region: simdr::module::Id,
    /// Where this lane's discarded bucket writes go.
    dump_mine: simdr::module::Id,
    /// Which block this is.
    block: simdr::module::Id,
    /// Which hierarchy's flags are being read and whose buckets are being written.
    tier: Tier,
}

/// The sweep itself, over the blocks the caller was dispatched for.
fn swept<const LANES: u32>(kernel: &mut Kernel<U32>, where_at: Sweeping) -> Result<(), LaneError> {
    use crate::sweep::PER_LANE;

    let Sweeping {
        mine,
        region,
        dump_mine,
        block,
        tier,
    } = where_at;
    let uint = kernel.index_type();

    let mut quads = Vec::with_capacity(PER_LANE as usize);
    let mut flags = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        flags.push(kernel.load_at(tier.flags, at)?);
        quads.push(at);
    }

    let mut serial = Vec::with_capacity(PER_LANE as usize);
    let carried = {
        let mut lanes = kernel.lanes()?;
        let mut running = lanes.splat_bits::<U32, LANES>(0)?;
        for &flag in &flags {
            // **One or nothing, not the countdown itself.** A scan of the counters would give
            // offsets two apart and a bucket full of holes.
            let held = lanes.splat_id::<U32, LANES>(flag)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let one = lanes.splat_bits::<U32, LANES>(1)?;
            let listed = lanes.greater_than(held, none)?;
            let counted = lanes.select(listed, one, none)?;
            serial.push(running);
            running = lanes.add(running, counted)?;
        }
        running
    };

    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(carried)?;
        let total = lanes.reduce_sum::<U32, LANES>(carried)?;
        (before, total)
    };
    kernel.store_at(tier.counts, block, total)?;

    for (index, quad) in quads.iter().enumerate() {
        let (Some(flag), Some(earlier)) = (flags.get(index), serial.get(index).copied()) else {
            continue;
        };
        let slot = {
            let mut lanes = kernel.lanes()?;
            let here = lanes.splat_id::<U32, LANES>(region)?;
            let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            // The counter is a countdown, so anything above nothing is awake.
            let awake = lanes.splat_id::<U32, LANES>(*flag)?;

            let within = lanes.add(before, earlier)?;
            let bucket = lanes.add(here, within)?;
            let listed = lanes.greater_than(awake, none)?;
            lanes.select(listed, bucket, elsewhere)?
        };
        kernel.store_at(tier.buckets, slot.id(), *quad)?;
    }

    Ok(())
}
