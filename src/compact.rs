//! The other two tiers: the blocks scanned against each other, and the buckets gathered.
//!
//! [`crate::sweep`] left every block with its own bucket and its own count. What is missing is the
//! third term of the offset — how many live cells every *earlier block* holds — and that cannot be
//! known until every block has run. So it is a second dispatch, and because there are more blocks
//! than one block can scan, a third.
//!
//! ```text
//!   sweep        blocks × per_block cells   →  counts[block]     and a bucket each
//!   scan_counts  tiles  × per_block counts  →  bases[block]      and totals[tile]
//!   scan_totals  1      × per_block totals  →  totals[tile]      exclusive, and the grand total
//!   gather       blocks × per_block slots   →  dense[]           the work list, contiguous
//! ```
//!
//! # When a work list is worth anything
//!
//! Not always, and the condition is a number rather than a judgement. A list makes a pass run over
//! fewer things; it does not make any one of them shorter. So it buys time where a pass is many
//! short independent steps and the device was full of them, and almost nothing where the cost is the
//! length of one chain of dependent steps. `decisions/DR-0009` has both sides: 44% of the work for
//! 47 to 49% of the time where an invocation is a few loads and a store, and 31 to 69% of the work
//! for 98 to 100% of it where an invocation is a carried loop over a thousand connections.
//!
//! The sand is the first case. A quad's step is short, it waits on nothing, and there are millions
//! of them, so cutting their number cuts the time — which is what `src/bin/scale.rs` measures and
//! what this module exists for.
//!
//! # Three passes and not four
//!
//! The obvious arrangement has a fourth: after `scan_totals` makes the tile bases absolute,
//! something has to add them back into `bases`. It is not here, because `gather` can read its own
//! tile's base directly — one load per block instead of a pass over every block. The offset it
//! writes at is
//!
//! ```text
//!   dense[ totals[block / per_block] + bases[block] + j ]
//! ```
//!
//! which is the same three-term sum as before, one tier up.
//!
//! # Why the second scan may run in place
//!
//! `scan_totals` reads and writes the same buffer. That is safe here and not in general: every
//! invocation reads only the entries it will itself write, and everything crossing between lanes
//! goes through a subgroup instruction over *registers*, never through the buffer. No lane reads a
//! word another lane has written.
//!
//! # The refusal
//!
//! Two levels reach `per_block²` blocks — 65 536 on a 32-lane part, which is 16.7 million cells, and
//! 1024 on a four-lane one. Past that a third level is needed and [`Plan::over`] says so by name
//! rather than scanning the first `per_block` tiles and silently dropping the rest.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

use crate::Groups;
pub use crate::layout::{BASES, BINDINGS, DENSE, GRAND, INDIRECT, TOTALS};
use crate::layout::{
    AWAKE, BUCKETS, COUNTS, HOT, HOT_BASES, HOT_BUCKETS, HOT_COUNTS, HOT_DENSE, HOT_GRAND,
    HOT_INDIRECT, HOT_TOTALS, PIXELS,
};
use crate::sand::View;
use crate::sweep::{self, PER_LANE};

/// The seven bindings one compaction hierarchy runs over.
///
/// **There are two of them, and they are the same code.** The fine tier compacts one word a quad
/// into the list the settle steps; the coarse tier compacts one word a *block* into the list that
/// decides how many workgroups the fine tier's own passes are dispatched over. Naming the bindings
/// rather than hard-coding them is the whole of what lets the second exist — see
/// [`crate::layout::HOT_BUCKETS`].
#[derive(Debug, Clone, Copy)]
pub struct Tier {
    /// One word an item: whether it wants looking at. A countdown, so anything above nought counts.
    pub flags: u32,
    /// One region a block: its compacted indices, then its dump.
    pub buckets: u32,
    /// One word a block: how many of its items are live.
    pub counts: u32,
    /// Where the exclusive prefix of each block lands.
    pub bases: u32,
    /// One word a tile: the sum of its blocks, then that made exclusive.
    pub totals: u32,
    /// The list itself, contiguous, with a dump past the end.
    pub dense: u32,
    /// One word: how long the list turned out to be.
    pub grand: u32,
    /// Three words Vulkan reads as a workgroup count.
    pub indirect: u32,
}

impl Tier {
    /// One word a quad, compacted into the list the settle steps.
    pub const FINE: Self = Self {
        flags: AWAKE,
        buckets: BUCKETS,
        counts: COUNTS,
        bases: BASES,
        totals: TOTALS,
        dense: DENSE,
        grand: GRAND,
        indirect: INDIRECT,
    };

    /// One word a block, compacted into the list the passes above are dispatched over.
    pub const COARSE: Self = Self {
        flags: HOT,
        buckets: HOT_BUCKETS,
        counts: HOT_COUNTS,
        bases: HOT_BASES,
        totals: HOT_TOTALS,
        dense: HOT_DENSE,
        grand: HOT_GRAND,
        indirect: HOT_INDIRECT,
    };
}

/// Which blocks a pass runs over, and therefore where a workgroup learns which one it owns.
///
/// # The whole of the saving, in one enum
///
/// [`Over::Every`] is a workgroup a block whether the block holds anything or not — which is what
/// `decay`, `mark` and `gather` did, and what cost 3 × 16 384 dispatches at sixteen million cells.
/// [`Over::Listed`] is a workgroup per *listed* block, over a list the device compacted and a count
/// the host never learns. The kernel is otherwise identical: only the first line differs.
#[derive(Debug, Clone, Copy)]
pub enum Over {
    /// One workgroup a block, indexed by the workgroup's own number.
    Every,
    /// One workgroup a listed block, indexed by the word this binding holds at that number.
    Listed(u32),
}

/// Which block this workgroup owns.
///
/// # Errors
///
/// As the module builder.
pub(crate) fn owned(kernel: &mut Kernel<U32>, over: Over) -> Result<Id, LaneError> {
    let group = kernel.workgroup_index();
    match over {
        Over::Every => Ok(group),
        // **No bound to check.** `prepare` divides the count by one for a list dispatched a
        // workgroup an entry, so the ceiling is exact and there is no last workgroup reading past
        // the end — which is the one thing the settle has to guard against and this does not.
        Over::Listed(binding) => kernel.load_at(binding, group),
    }
}

/// What a hierarchy of this shape can and cannot hold.
#[derive(Debug)]
pub enum TooBig {
    /// More blocks than two levels of scan reach.
    Blocks {
        /// What was asked for.
        blocks: u32,
        /// The most two levels reach.
        limit: u32,
    },
}

impl std::fmt::Display for TooBig {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocks { blocks, limit } => write!(
                out,
                "{blocks} blocks needs a third level of scan; two reach {limit}"
            ),
        }
    }
}

impl std::error::Error for TooBig {}

/// Every size the four passes need, derived from one number.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    /// The device's subgroup width, which is also the workgroup's.
    pub subgroup: u32,
    /// Cells in one block, and entries one scan pass covers.
    pub per_block: u32,
    /// How many blocks of cells there are.
    pub blocks: u32,
    /// How many blocks of *blocks* — one per `scan_counts` workgroup, and one per coarse workgroup.
    ///
    /// **One number for both hierarchies.** A tile is `per_block` blocks, which is exactly what one
    /// workgroup of the coarse `mark` and `gather` sweeps. They are the same division of the same
    /// array rather than two that happen to agree.
    pub tiles: u32,
    /// How many workgroups the coarse `scan_counts` runs. Always one, and provably so.
    ///
    /// [`Plan::over`] refuses past `per_block²` blocks, so there are at most `per_block` tiles, and
    /// one workgroup scans `per_block` entries. Carried as a field anyway, so the arithmetic is in
    /// one place rather than in a comment beside a literal.
    pub hot_tiles: u32,
    /// Cells altogether — four to a quad.
    pub grid: usize,
    /// Quads altogether, which is what the compaction runs over.
    pub quads: usize,
}

impl Plan {
    /// The sizes for `blocks` blocks on a `subgroup`-wide device.
    ///
    /// # Errors
    ///
    /// [`TooBig::Blocks`] when two levels of scan do not reach.
    pub fn over(subgroup: u32, blocks: u32) -> Result<Self, TooBig> {
        let per_block = sweep::cells(subgroup);
        let limit = per_block.saturating_mul(per_block);
        if blocks > limit {
            return Err(TooBig::Blocks { blocks, limit });
        }
        let tiles = blocks.div_ceil(per_block);
        Ok(Self {
            subgroup,
            per_block,
            blocks,
            tiles,
            hot_tiles: tiles.div_ceil(per_block),
            quads: per_block as usize * blocks as usize,
            grid: per_block as usize * blocks as usize * 4,
        })
    }

    /// How many blocks the coarse hierarchy actually sweeps: the blocks, padded to whole tiles.
    ///
    /// A coarse workgroup covers a tile, so a partial one would read past the end of
    /// [`crate::layout::HOT`]. The padding is zeros, which is a block nothing woke.
    #[must_use]
    pub const fn padded(&self) -> u32 {
        self.tiles * self.per_block
    }

    /// The words each binding needs, in binding order — what `Host::resident` is handed.
    ///
    /// `counts` and `bases` are padded to a whole number of tiles, because a scan pass covers a
    /// tile and a partial one would read past the end. The padding is zeros, which a sum ignores.
    #[must_use]
    pub fn sizes(&self) -> [usize; BINDINGS as usize] {
        let padded = self.tiles as usize * self.per_block as usize;
        let hot_padded = self.hot_tiles as usize * self.per_block as usize;
        let spare = self.blocks as usize * self.subgroup as usize;
        [
            8,
            // The grid, and four spare cells an invocation for the settle's discarded quads.
            self.grid + self.quads * 4,
            self.blocks as usize * sweep::stride(self.subgroup) as usize,
            padded,
            padded,
            self.per_block as usize,
            self.quads + spare,
            1,
            3,
            // **The dump has to fit the widest dispatch that writes into it**, which is the
            // settle: one invocation a quad. It used to be the brush, at one a cell — four times
            // as many — and sizing it by quads then was four times short. A storage write past the
            // end is undefined rather than refused; it landed in the next binding and looked like a
            // wake set that had missed something. The brush is a box now, so quads is the bound
            // again, and at sixteen million cells that is sixty-seven megabytes back.
            self.quads + self.quads,
            // **One word, until somebody says what they are looking at.** A picture is the size of
            // the window, not of the world, and a plan is never told which — so the default is the
            // smallest legal allocation and [`Plan::showing`] is how a viewer asks for a real one.
            // A run that never paints then carries no picture at all, which at sixteen million
            // cells is sixty-seven megabytes it was allocating and never reading.
            1,
            crate::layout::TALLY_WORDS,
            // One a block, padded to whole tiles because the coarse sweep covers a tile at a time,
            // and one more for everything that was not a block — `layout::HOT` says why a single
            // slot is enough for all of it.
            self.padded() as usize + 1,
            // The coarse hierarchy, which is the fine one over one word a block instead of one a
            // quad. Every size below is the line above it with `blocks` where that had `quads` and
            // `tiles` where that had `blocks`.
            self.tiles as usize * sweep::stride(self.subgroup) as usize,
            hot_padded,
            hot_padded,
            self.per_block as usize,
            // Every block could be hot, and the dump has to fit the widest dispatch that writes
            // into it — which is the coarse gather, one invocation a slot over `tiles` workgroups.
            self.padded() as usize + self.tiles as usize * self.subgroup as usize,
            1,
            3,
        ]
    }

    /// Where the coarse flags' dump is: one word past the **padding**, not past the last block.
    ///
    /// A slot inside the padding would be a hot block that no quad ever woke, and the coarse sweep
    /// reads the padding — see [`crate::layout::HOT`].
    #[must_use]
    pub const fn cold(&self) -> u32 {
        self.padded()
    }

    /// Where the coarse gather's dump begins in [`crate::layout::HOT_DENSE`].
    #[must_use]
    pub const fn hot_dump(&self) -> u32 {
        self.padded()
    }

    /// What the three passes driven by the hot-block list ask for: the count `hot_prepare` wrote.
    ///
    /// **The host never learns this one either.** It is how many blocks turned out to have
    /// something in them, which is the number `decay`, `mark` and `gather` used to be dispatched
    /// over the whole world to avoid asking for.
    #[must_use]
    pub const fn hot_listed() -> Groups {
        Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }
    }

    /// The same sizes, with room for a picture of `view`.
    #[must_use]
    pub fn showing(&self, view: View) -> [usize; BINDINGS as usize] {
        let mut sizes = self.sizes();
        if let Some(picture) = sizes.get_mut(PIXELS as usize) {
            *picture = view.pixels();
        }
        sizes
    }

    /// Where `gather`'s dump begins in [`DENSE`], which is past every slot the work list can use.
    #[must_use]
    pub fn dump(&self) -> u32 {
        u32::try_from(self.quads).unwrap_or(u32::MAX)
    }

    /// One workgroup count per pass, in the order the passes run.
    #[must_use]
    pub fn groups(&self) -> [Groups; 5] {
        [
            Groups::Fixed(self.blocks),
            Groups::Fixed(self.tiles),
            Groups::Fixed(1),
            Groups::Fixed(self.blocks),
            Groups::Fixed(1),
        ]
    }

    /// What a pass driven by the work list asks for: the count `prepare` wrote.
    ///
    /// **The host never learns what it is.** Everything in [`Plan::groups`] is sized here; this one
    /// is read out of a buffer at dispatch time, which is the whole reason `prepare` exists.
    #[must_use]
    pub const fn listed() -> Groups {
        Groups::From {
            binding: INDIRECT as usize,
            at: 0,
        }
    }
}

/// A block-local exclusive scan of `from` into `into`, with each block's total into `aggregate`.
///
/// The same three tiers as [`crate::sweep`], without the simulation in front of them: a serial scan
/// in registers, one `Lanes::prefix_sum_exclusive` across the lanes, and one `Lanes::reduce_sum` for
/// what the block came to.
///
/// # Errors
///
/// [`LaneError::NoMapping`] for a width with no arm, otherwise as the module builder.
pub fn scan(subgroup: u32, from: u32, into: u32, aggregate: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => scanned::<4>(subgroup, from, into, aggregate),
        8 => scanned::<8>(subgroup, from, into, aggregate),
        16 => scanned::<16>(subgroup, from, into, aggregate),
        32 => scanned::<32>(subgroup, from, into, aggregate),
        64 => scanned::<64>(subgroup, from, into, aggregate),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The scan, at a width the caller has matched to the device.
fn scanned<const LANES: u32>(
    subgroup: u32,
    from: u32,
    into: u32,
    aggregate: u32,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along_lane)?;

    // Every entry this invocation owns, read before anything is written — which is what makes an
    // in-place scan safe: no lane reads a word another lane will write.
    let mut addresses = Vec::with_capacity(PER_LANE as usize);
    let mut loaded = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        loaded.push(kernel.load_at(from, at)?);
        addresses.push(at);
    }

    // The serial tier, in registers.
    let mut serial = Vec::with_capacity(PER_LANE as usize);
    let carried = {
        let mut lanes = kernel.lanes()?;
        let mut running = lanes.splat_bits::<U32, LANES>(0)?;
        for &value in &loaded {
            let held = lanes.splat_id::<U32, LANES>(value)?;
            serial.push(running);
            running = lanes.add(running, held)?;
        }
        running
    };

    // The lane tier, and what the block came to.
    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(carried)?;
        let total = lanes.reduce_sum::<U32, LANES>(carried)?;
        (before, total)
    };
    kernel.store_at(aggregate, block, total)?;

    // And the exclusive prefix of every entry, which is the two tiers added.
    for (index, at) in addresses.iter().enumerate() {
        let Some(earlier) = serial.get(index).copied() else {
            continue;
        };
        let sum = {
            let mut lanes = kernel.lanes()?;
            lanes.add(before, earlier)?
        };
        kernel.store_at(into, *at, sum.id())?;
    }

    kernel.finish()
}

/// Move every block's bucket into the dense work list, at the place the two scans decided.
///
/// # The reads past the count are deliberate
///
/// A block's bucket holds `counts[block]` live cells and `per_block` slots. Every lane reads all
/// [`PER_LANE`] of its slots and writes all of them — the ones past the count carry whatever was
/// there and are written to a dump nothing reads, because `decisions/DR-0003` refuses the branch
/// that would skip them. The dump is one slot a lane, so those writes are one cache line rather
/// than one word each.
///
/// # Errors
///
/// As [`scan`].
pub fn gather(subgroup: u32, tier: Tier, over: Over, dump: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => gathered::<4>(subgroup, tier, over, dump),
        8 => gathered::<8>(subgroup, tier, over, dump),
        16 => gathered::<16>(subgroup, tier, over, dump),
        32 => gathered::<32>(subgroup, tier, over, dump),
        64 => gathered::<64>(subgroup, tier, over, dump),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The gather, at a width the caller has matched to the device.
fn gathered<const LANES: u32>(
    subgroup: u32,
    tier: Tier,
    over: Over,
    dump: u32,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let per_block = sweep::cells(subgroup);

    // Which tile this block is in. **A shift, because there is no division** — and it is exact
    // rather than lucky: a block is a power-of-two subgroup times `PER_LANE`, which is also one.
    let tile = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
        lanes.shift_right_logical(which, by)?
    };

    let tile_base = kernel.load_at(tier.totals, tile.id())?;
    let block_base = kernel.load_at(tier.bases, block)?;
    let live = kernel.load_at(tier.counts, block)?;

    // Where this block's bucket starts, and where its dump slot is.
    let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
    let region = kernel.module().i_mul(uint, block, wide)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let source = kernel.module().i_add(uint, region, along_lane)?;

    // **A constant, where this used to be a word of `PARAMS`.** There are two hierarchies now and
    // their dumps are in different buffers at different offsets, so the one shared parameter word
    // could not name both — and the module is built for a plan already, so a load bought nothing.
    let dump_base = kernel.module().constant_u32(dump)?;
    let dump_span = kernel.module().constant_u32(subgroup)?;
    let dump_block = kernel.module().i_mul(uint, block, dump_span)?;
    let dump_start = kernel.module().i_add(uint, dump_base, dump_block)?;
    let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

    // **The gate, on a number this pass had already read.** A block that contributed nothing to the
    // list has nothing here anyone wants, and without this every block reads eight bucket slots and
    // writes eight list slots whether it holds anything or not — four million of each at sixteen
    // million cells, against a work list of seven hundred. It was the last pass in the loop whose
    // cost was the size of the world. See `crate::gate`.
    crate::gate::if_any::<LANES, _>(&mut kernel, live, |kernel| {
        let where_at = Gathering {
            source,
            along_lane,
            dump_mine,
            live,
            tile_base,
            block_base,
            tier,
        };
        taking::<LANES>(kernel, where_at)
    })?;
    kernel.finish()
}

/// Where one block's gather reads and writes, gathered so the call is not six arguments long.
struct Gathering {
    /// Where this lane's run of bucket slots begins.
    source: Id,
    /// How far into the block this lane's run starts.
    along_lane: Id,
    /// Where this lane's discarded writes go.
    dump_mine: Id,
    /// How many of this block's slots carry anything.
    live: Id,
    /// Where this tile's share of the list begins.
    tile_base: Id,
    /// And this block's, within the tile.
    block_base: Id,
    /// Which hierarchy's buckets are being read and whose list is being written.
    tier: Tier,
}

/// The gather itself, run only for blocks that contributed something.
fn taking<const LANES: u32>(
    kernel: &mut Kernel<U32>,
    where_at: Gathering,
) -> Result<(), LaneError> {
    let Gathering {
        source,
        along_lane,
        dump_mine,
        live,
        tile_base,
        block_base,
        tier,
    } = where_at;
    let uint = kernel.index_type();

    // Read every slot this lane owns, then write every one of them.
    let mut taken = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, source, along)?;
        taken.push(kernel.load_at(tier.buckets, at)?);
    }

    for (step, value) in taken.iter().enumerate() {
        let slot = {
            let mut lanes = kernel.lanes()?;
            let position = lanes.splat_id::<U32, LANES>(along_lane)?;
            let along = lanes.splat_bits::<U32, LANES>(u32::try_from(step).unwrap_or(0))?;
            let within = lanes.add(position, along)?;

            let count = lanes.splat_id::<U32, LANES>(live)?;
            let tile_at = lanes.splat_id::<U32, LANES>(tile_base)?;
            let block_at = lanes.splat_id::<U32, LANES>(block_base)?;
            let elsewhere = lanes.splat_id::<U32, LANES>(dump_mine)?;

            // The third and fourth tiers: this tile's base, plus this block's within the tile.
            let base = lanes.add(tile_at, block_at)?;
            let there = lanes.add(base, within)?;

            // A slot past the count carries nothing anyone wants, and goes where nothing looks.
            let inside = lanes.greater_than(count, within)?;
            lanes.select(inside, there, elsewhere)?
        };
        kernel.store_at(tier.dense, slot.id(), *value)?;
    }

    Ok(())
}

/// What the work list should be: every block's bucket, in block order, end to end.
///
/// **The order is part of the answer.** A compaction that produced the right *set* in the wrong
/// order would pass a check that sorted first, and would mean the scan's tiers had been added in
/// some other arrangement than the one the kernel claims. So this concatenates rather than collects.
#[must_use]
pub fn dense_reference(plan: &Plan, counts: &[u32], buckets: &[u32]) -> (Vec<u32>, u32) {
    let wide = sweep::stride(plan.subgroup) as usize;
    let mut out = Vec::new();
    let mut running = 0_u32;

    for block in 0..plan.blocks as usize {
        let live = counts.get(block).copied().unwrap_or(0) as usize;
        let from = block * wide;
        if let Some(slice) = buckets.get(from..from + live) {
            out.extend_from_slice(slice);
        }
        running = running.wrapping_add(counts.get(block).copied().unwrap_or(0));
    }

    (out, running)
}

/// Turn the grand total into a workgroup count, in the buffer the next dispatch reads.
///
/// **The pass that makes the saving real.** Without it the host must read [`GRAND`] back to know how
/// many workgroups to ask for — a submit and a fence to learn a number the device already had, and
/// `tests/floor.rs` prices that at **41 µs** on this machine — more than the whole frame that
/// produced the number. With this pass the count never leaves.
///
/// `per_group` is how many entries the pass that reads this count covers with one workgroup, and it is
/// a parameter because the passes disagree. A scan covers [`PER_LANE`] entries a lane; a settle over
/// the work list covers one. Dividing by the wrong one is a dispatch that silently runs over part of
/// the list — which is exactly what `tests/live.rs` caught, as sand that fell in one run and not the
/// other.
///
/// **One is a legal value and the coarse tier uses it.** A pass dispatched a workgroup per listed
/// block divides by one, and the ceiling is then exact — which is what lets [`Over::Listed`] skip
/// the bound check the settle needs.
///
/// The rounding up is a shift and an add, because there is no division and none is needed: both
/// counts are powers of two.
///
/// Every invocation of the single workgroup writes the same three words to the same three slots,
/// which is the case `Kernel::store_at` documents as safe.
///
/// # Errors
///
/// As [`scan`].
pub fn prepare(subgroup: u32, per_group: u32, tier: Tier) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => prepared::<4>(subgroup, per_group, tier),
        8 => prepared::<8>(subgroup, per_group, tier),
        16 => prepared::<16>(subgroup, per_group, tier),
        32 => prepared::<32>(subgroup, per_group, tier),
        64 => prepared::<64>(subgroup, per_group, tier),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// The count, at a width the caller has matched to the device.
fn prepared<const LANES: u32>(
    subgroup: u32,
    per_group: u32,
    tier: Tier,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let per_block = per_group.max(1);

    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(tier.grand, head)?;

    let groups = {
        let mut lanes = kernel.lanes()?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let short = lanes.splat_bits::<U32, LANES>(per_block - 1)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;

        // `(grand + per_group - 1) >> log2(per_group)`: a ceiling division, spelled the only way an
        // engine without division can spell one.
        let rounded = lanes.add(total, short)?;
        lanes.shift_right_logical(rounded, by)?
    };

    let one = kernel.module().constant_u32(1)?;
    let two = kernel.module().constant_u32(2)?;
    kernel.store_at(tier.indirect, head, groups.id())?;
    kernel.store_at(tier.indirect, one, one)?;
    kernel.store_at(tier.indirect, two, one)?;
    kernel.finish()
}
