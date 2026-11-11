use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

use crate::Groups;
use crate::layout::{
    AWAKE, BUCKETS, COUNTS, Dimension, HOT, HOT_BASES, HOT_BUCKETS, HOT_COUNTS, HOT_DENSE,
    HOT_GRAND, HOT_INDIRECT, HOT_TOTALS, PIXELS,
};
pub use crate::layout::{BASES, BINDINGS, DENSE, GRAND, INDIRECT, TOTALS};
use crate::sand::View;
use crate::sweep::{self, PER_LANE};

#[derive(Debug, Clone, Copy)]
pub struct Tier {
    pub flags: u32,
    pub buckets: u32,
    pub counts: u32,
    pub bases: u32,
    pub totals: u32,
    pub dense: u32,
    pub grand: u32,
    pub indirect: u32,
}

impl Tier {
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

#[must_use]
pub fn reaching(bindings: &[u32]) -> u32 {
    bindings
        .iter()
        .map(|binding| binding.saturating_add(1))
        .fold(BINDINGS, u32::max)
}

impl Tier {
    #[must_use]
    pub fn reach(&self) -> u32 {
        reaching(&[
            self.flags,
            self.buckets,
            self.counts,
            self.bases,
            self.totals,
            self.dense,
            self.grand,
            self.indirect,
        ])
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Over {
    Every,
    Listed(u32),
}

#[must_use]
pub fn over_reach(tier: Tier, over: Over) -> u32 {
    match over {
        Over::Every => tier.reach(),
        Over::Listed(binding) => tier.reach().max(binding.saturating_add(1)),
    }
}

pub(crate) fn owned(kernel: &mut Kernel<U32>, over: Over) -> Result<Id, LaneError> {
    let group = kernel.workgroup_index();
    match over {
        Over::Every => Ok(group),
        Over::Listed(binding) => kernel.load_at(binding, group),
    }
}

#[derive(Debug)]
pub enum TooBig {
    Blocks { blocks: u32, limit: u32 },
    Cells { cells: u64, limit: u64 },
}

impl std::fmt::Display for TooBig {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocks { blocks, limit } => write!(out, "{blocks} {limit}"),
            Self::Cells { cells, limit } => write!(
                out,
                "{cells} words of STATE pass the {limit} a u32 index reaches"
            ),
        }
    }
}

impl std::error::Error for TooBig {}

#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub subgroup: u32,
    pub per_block: u32,
    pub blocks: u32,
    pub tiles: u32,
    pub hot_tiles: u32,
    pub grid: usize,
    pub quads: usize,
    pub dimension: Dimension,
    window: u32,
    bricked: bool,
}

impl Plan {
    /// The most quads a scan two levels deep reaches for this subgroup, and
    /// so the most a flat plan holds.
    ///
    /// `over` refuses a flat plan once its blocks outrun `sweep::cells`
    /// squared, because its scan is two levels deep and reaches no further,
    /// and a block holds `sweep::cells` quads. So the grid such a plan can
    /// address is that number cubed: 16 777 216 quads at a subgroup of 32,
    /// and 32 768 at a subgroup of 4. A solid plan past it scans three levels
    /// deep ([`Plan::most_blocks`]). It is not a limit of the device.
    #[must_use]
    pub fn reach(subgroup: u32) -> u64 {
        u64::from(sweep::cells(subgroup)).pow(3)
    }

    /// The most blocks a plan of `dimension` scans: [`Plan::reach`] over a
    /// block's quads for a flat world, whose scan is two levels deep, and a
    /// block's quads squared for a solid one, whose scan takes a third level
    /// once its tiles outnumber a block's quads.
    #[must_use]
    pub fn most_blocks(subgroup: u32, dimension: Dimension) -> u64 {
        let per_block = u64::from(sweep::cells(subgroup));
        match dimension {
            Dimension::Two => per_block * per_block,
            Dimension::Three => per_block * per_block * per_block,
        }
    }

    /// The most words of `STATE` a plan holds: every one is addressed by a
    /// `u32` index.
    pub const MOST_WORDS: u64 = 1 << 32;

    pub fn over(subgroup: u32, blocks: u32) -> Result<Self, TooBig> {
        Self::over_in(subgroup, blocks, Dimension::Two)
    }

    pub fn over_in(subgroup: u32, blocks: u32, dimension: Dimension) -> Result<Self, TooBig> {
        let held = Self::laid(subgroup, blocks, dimension)?;
        let words = held.state_words() as u64;
        if words > Self::MOST_WORDS {
            return Err(TooBig::Cells {
                cells: words,
                limit: Self::MOST_WORDS,
            });
        }
        Ok(held)
    }

    pub fn bricked(subgroup: u32, blocks: u32) -> Result<Self, TooBig> {
        let mut held = Self::laid(subgroup, blocks, Dimension::Three)?;
        held.bricked = true;
        Ok(held)
    }

    #[must_use]
    pub const fn is_bricked(&self) -> bool {
        self.bricked
    }

    fn laid(subgroup: u32, blocks: u32, dimension: Dimension) -> Result<Self, TooBig> {
        let per_block = sweep::cells(subgroup);
        let most = Self::most_blocks(subgroup, dimension);
        if u64::from(blocks) > most {
            let limit = u32::try_from(most).unwrap_or(u32::MAX);
            return Err(TooBig::Blocks { blocks, limit });
        }
        let tiles = blocks.div_ceil(per_block);
        let held = Self {
            subgroup,
            per_block,
            blocks,
            tiles,
            hot_tiles: tiles.div_ceil(per_block),
            quads: per_block as usize * blocks as usize,
            grid: per_block as usize * blocks as usize * dimension.corners(),
            dimension,
            window: crate::layout::WINDOW_BITS,
            bricked: false,
        };
        Ok(held)
    }

    /// This plan with `STATE` split into windows of `2^bits` words once it
    /// holds `2^(bits + 1)` words or more, in place of
    /// [`crate::layout::WINDOW_BITS`]. Only tests build it, to split a small
    /// world.
    #[cfg(feature = "faults")]
    #[must_use]
    pub const fn split_at(mut self, bits: u32) -> Self {
        self.window = bits;
        self
    }

    /// The scan's depth: 1 for one tile, 2 while the tiles fit one block's
    /// quads, and 3 past that.
    #[must_use]
    pub const fn levels(&self) -> u32 {
        if self.tiles == 1 {
            1
        } else if self.tiles <= self.per_block {
            2
        } else {
            3
        }
    }

    /// The groups of tiles the third level scans, each a block's quads of
    /// tiles.
    #[must_use]
    pub const fn supers(&self) -> u32 {
        self.tiles.div_ceil(self.per_block)
    }

    /// Where in `TOTALS` the totals of the third level start: past the
    /// tiles' totals, padded to whole groups of tiles.
    #[must_use]
    pub const fn upper(&self) -> u32 {
        self.supers() * self.per_block
    }

    /// The words of `STATE`: the grid, and for a flat world the spare its
    /// listed settle writes the lanes past the list into.
    #[must_use]
    pub const fn state_words(&self) -> usize {
        match self.dimension {
            Dimension::Two => self.grid + self.quads * self.dimension.corners(),
            Dimension::Three => self.grid,
        }
    }

    /// How `STATE` lies over bindings.
    #[must_use]
    pub const fn windows(&self) -> crate::layout::Windows {
        crate::layout::Windows::at(self.state_words(), self.window)
    }

    /// [`Plan::sizes`] laid over the bindings a resident holds: `STATE` as
    /// its first window and the other windows after the engine's bindings.
    #[must_use]
    pub fn bound(&self) -> Vec<usize> {
        self.windows().laid(&self.sizes())
    }

    #[must_use]
    pub const fn padded(&self) -> u32 {
        self.tiles * self.per_block
    }

    #[must_use]
    pub fn sizes(&self) -> [usize; BINDINGS as usize] {
        let padded = self.tiles as usize * self.per_block as usize;
        let hot_padded = self.hot_tiles as usize * self.per_block as usize;
        let spare = self.blocks as usize * self.subgroup as usize;
        let totals = if self.levels() > 2 {
            self.upper() as usize + self.per_block as usize
        } else {
            self.per_block as usize
        };
        [
            self.dimension.params_words(),
            self.state_words(),
            self.blocks as usize * sweep::stride(self.subgroup) as usize,
            padded,
            padded,
            totals,
            self.quads + spare,
            1,
            3,
            // AWAKE is the marks and then a scratch slot for every lane that
            // did not wake anything, which `live::wake` writes at
            // `dump_base + group * subgroup + lane`. The scratch has to hold
            // the widest pass that writes into it, and the brush is that one:
            // it covers a fixed box of `sand::BOX` seats along each axis however
            // small the grid is, so a grid with fewer quads than seats would be
            // written past its end.
            self.quads + self.quads.max(self.dimension.seats()),
            1,
            self.dimension.tally_words(),
            self.padded() as usize + 1,
            self.tiles as usize * sweep::stride(self.subgroup) as usize,
            hot_padded,
            hot_padded,
            self.per_block as usize,
            self.padded() as usize + self.tiles as usize * self.subgroup as usize,
            1,
            3,
        ]
    }

    #[must_use]
    pub const fn cold(&self) -> u32 {
        self.padded()
    }

    #[must_use]
    pub const fn hot_dump(&self) -> u32 {
        self.padded()
    }

    #[must_use]
    pub const fn hot_listed() -> Groups {
        Groups::From {
            binding: HOT_INDIRECT as usize,
            at: 0,
        }
    }

    #[must_use]
    pub fn showing(&self, view: View) -> [usize; BINDINGS as usize] {
        let mut sizes = self.sizes();
        if let Some(picture) = sizes.get_mut(PIXELS as usize) {
            *picture = view.pixels();
        }
        sizes
    }

    #[must_use]
    pub fn dump(&self) -> u32 {
        u32::try_from(self.quads).unwrap_or(u32::MAX)
    }

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

    #[must_use]
    pub const fn listed() -> Groups {
        Groups::From {
            binding: INDIRECT as usize,
            at: 0,
        }
    }
}

pub fn scan(subgroup: u32, from: u32, into: u32, aggregate: u32) -> Result<Vec<u32>, LaneError> {
    scan_at(subgroup, from, into, aggregate, Along::START)
}

/// Where a scan starts in its source and its target, and where its
/// aggregates start: the third level scans the tiles' totals into
/// `TOTALS` past them, and then scans those in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Along {
    pub from: u32,
    pub aggregate: u32,
}

impl Along {
    pub const START: Self = Self {
        from: 0,
        aggregate: 0,
    };
}

/// [`scan`] with each group's run starting `along.from` words into `from`
/// and `into`, and its aggregate stored `along.aggregate` words into
/// `aggregate`.
pub fn scan_at(
    subgroup: u32,
    from: u32,
    into: u32,
    aggregate: u32,
    along: Along,
) -> Result<Vec<u32>, LaneError> {
    let at = Scanning {
        from,
        into,
        aggregate,
        alone: None,
        along,
    };
    match subgroup {
        4 => scanned::<4>(subgroup, at),
        8 => scanned::<8>(subgroup, at),
        16 => scanned::<16>(subgroup, at),
        32 => scanned::<32>(subgroup, at),
        64 => scanned::<64>(subgroup, at),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

pub fn whole(subgroup: u32, from: u32, into: u32, tier: Tier) -> Result<Vec<u32>, LaneError> {
    let at = Scanning {
        from,
        into,
        aggregate: tier.totals,
        alone: Some((tier.totals, tier.grand)),
        along: Along::START,
    };
    match subgroup {
        4 => scanned::<4>(subgroup, at),
        8 => scanned::<8>(subgroup, at),
        16 => scanned::<16>(subgroup, at),
        32 => scanned::<32>(subgroup, at),
        64 => scanned::<64>(subgroup, at),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

struct Scanning {
    from: u32,
    into: u32,
    aggregate: u32,
    alone: Option<(u32, u32)>,
    along: Along,
}

fn scanned<const LANES: u32>(subgroup: u32, at: Scanning) -> Result<Vec<u32>, LaneError> {
    let Scanning {
        from,
        into,
        aggregate,
        alone,
        along,
    } = at;
    let mut held = vec![from, into, aggregate];
    if let Some((totals, grand)) = alone {
        held.extend([totals, grand]);
    }
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, reaching(&held)))?;
    let uint = kernel.index_type();

    let block = kernel.workgroup_index();
    let lane = kernel.local_index();
    let span = kernel.module().constant_u32(sweep::cells(subgroup))?;
    let start = kernel.module().i_mul(uint, block, span)?;
    let start = if along.from == 0 {
        start
    } else {
        let offset = kernel.module().constant_u32(along.from)?;
        kernel.module().i_add(uint, start, offset)?
    };
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let mine = kernel.module().i_add(uint, start, along_lane)?;

    let mut addresses = Vec::with_capacity(PER_LANE as usize);
    let mut loaded = Vec::with_capacity(PER_LANE as usize);
    for step in 0..PER_LANE {
        let along = kernel.module().constant_u32(step)?;
        let at = kernel.module().i_add(uint, mine, along)?;
        loaded.push(kernel.load_at(from, at)?);
        addresses.push(at);
    }

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

    let (before, total) = {
        let mut lanes = kernel.lanes()?;
        let before = lanes.prefix_sum_exclusive::<U32, LANES>(carried)?;
        let total = lanes.reduce_sum::<U32, LANES>(carried)?;
        (before, total)
    };
    match alone {
        None if along.aggregate == 0 => kernel.store_at(aggregate, block, total)?,
        None => {
            let offset = kernel.module().constant_u32(along.aggregate)?;
            let at = kernel.module().i_add(uint, block, offset)?;
            kernel.store_at(aggregate, at, total)?;
        }
        Some((totals, grand)) => {
            let nothing = kernel.module().constant_u32(0)?;
            kernel.store_at(totals, block, nothing)?;
            kernel.store_at(grand, block, total)?;
        }
    }

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

pub fn gather(subgroup: u32, tier: Tier, over: Over, dump: u32) -> Result<Vec<u32>, LaneError> {
    gather_over(subgroup, tier, over, dump, None)
}

/// [`gather`] behind a scan three levels deep: each block's base also takes
/// the total of the group of tiles it lies in, which the third level keeps
/// `upper` words into the tier's totals.
pub fn gather_over(
    subgroup: u32,
    tier: Tier,
    over: Over,
    dump: u32,
    upper: Option<u32>,
) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => gathered::<4>(subgroup, tier, over, (dump, upper)),
        8 => gathered::<8>(subgroup, tier, over, (dump, upper)),
        16 => gathered::<16>(subgroup, tier, over, (dump, upper)),
        32 => gathered::<32>(subgroup, tier, over, (dump, upper)),
        64 => gathered::<64>(subgroup, tier, over, (dump, upper)),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn gathered<const LANES: u32>(
    subgroup: u32,
    tier: Tier,
    over: Over,
    (dump, upper): (u32, Option<u32>),
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, over_reach(tier, over)))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let per_block = sweep::cells(subgroup);

    let tile = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
        lanes.shift_right_logical(which, by)?
    };

    let tile_base = kernel.load_at(tier.totals, tile.id())?;
    let tile_base = match upper {
        None => tile_base,
        Some(upper) => {
            let group = {
                let mut lanes = kernel.lanes()?;
                let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
                let past = lanes.splat_bits::<U32, LANES>(upper)?;
                let group = lanes.shift_right_logical(tile, by)?;
                lanes.add(group, past)?
            };
            let group_base = kernel.load_at(tier.totals, group.id())?;
            kernel.module().i_add(uint, tile_base, group_base)?
        }
    };
    let block_base = kernel.load_at(tier.bases, block)?;
    let live = kernel.load_at(tier.counts, block)?;

    let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
    let region = kernel.module().i_mul(uint, block, wide)?;
    let run = kernel.module().constant_u32(PER_LANE)?;
    let along_lane = kernel.module().i_mul(uint, lane, run)?;
    let source = kernel.module().i_add(uint, region, along_lane)?;

    let dump_base = kernel.module().constant_u32(dump)?;
    let dump_span = kernel.module().constant_u32(subgroup)?;
    let dump_block = kernel.module().i_mul(uint, block, dump_span)?;
    let dump_start = kernel.module().i_add(uint, dump_base, dump_block)?;
    let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;

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

#[derive(Debug, Clone, Copy)]
pub struct Held {
    pub table: u32,
    pub entry: u32,
    pub reach: u32,
    pub dump: u32,
    pub upper: Option<u32>,
}

pub fn gather_held(
    subgroup: u32,
    tier: Tier,
    over: Over,
    held: Held,
) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => gathered_held::<4>(subgroup, tier, over, held),
        8 => gathered_held::<8>(subgroup, tier, over, held),
        16 => gathered_held::<16>(subgroup, tier, over, held),
        32 => gathered_held::<32>(subgroup, tier, over, held),
        64 => gathered_held::<64>(subgroup, tier, over, held),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn gathered_held<const LANES: u32>(
    subgroup: u32,
    tier: Tier,
    over: Over,
    held: Held,
) -> Result<Vec<u32>, LaneError> {
    let reach = over_reach(tier, over).max(held.reach);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, reach))?;
    let uint = kernel.index_type();

    let block = owned(&mut kernel, over)?;
    let lane = kernel.local_index();
    let per_block = sweep::cells(subgroup);

    let tile = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
        lanes.shift_right_logical(which, by)?
    };

    let tile_base = kernel.load_at(tier.totals, tile.id())?;
    let tile_base = match held.upper {
        None => tile_base,
        Some(upper) => {
            let group = {
                let mut lanes = kernel.lanes()?;
                let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
                let past = lanes.splat_bits::<U32, LANES>(upper)?;
                let group = lanes.shift_right_logical(tile, by)?;
                lanes.add(group, past)?
            };
            let group_base = kernel.load_at(tier.totals, group.id())?;
            kernel.module().i_add(uint, tile_base, group_base)?
        }
    };
    let block_base = kernel.load_at(tier.bases, block)?;
    let live = kernel.load_at(tier.counts, block)?;

    crate::gate::if_any::<LANES, _>(&mut kernel, live, |kernel| {
        let uint = kernel.index_type();
        let width = kernel.module().constant_u32(held.entry)?;
        let at = kernel.module().i_mul(uint, block, width)?;
        let slot = kernel.load_at(held.table, at)?;
        let wide = kernel.module().constant_u32(sweep::stride(subgroup))?;
        let region = kernel.module().i_mul(uint, slot, wide)?;
        let run = kernel.module().constant_u32(PER_LANE)?;
        let along_lane = kernel.module().i_mul(uint, lane, run)?;
        let source = kernel.module().i_add(uint, region, along_lane)?;

        let dump_base = kernel.module().constant_u32(held.dump)?;
        let dump_span = kernel.module().constant_u32(subgroup)?;
        let dump_block = kernel.module().i_mul(uint, slot, dump_span)?;
        let dump_start = kernel.module().i_add(uint, dump_base, dump_block)?;
        let dump_mine = kernel.module().i_add(uint, dump_start, lane)?;
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

struct Gathering {
    source: Id,
    along_lane: Id,
    dump_mine: Id,
    live: Id,
    tile_base: Id,
    block_base: Id,
    tier: Tier,
}

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

            let base = lanes.add(tile_at, block_at)?;
            let there = lanes.add(base, within)?;

            let inside = lanes.greater_than(count, within)?;
            lanes.select(inside, there, elsewhere)?
        };
        kernel.store_at(tier.dense, slot.id(), *value)?;
    }

    Ok(())
}

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

fn prepared<const LANES: u32>(
    subgroup: u32,
    per_group: u32,
    tier: Tier,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, tier.reach()))?;
    let per_block = per_group.max(1);

    let head = kernel.module().constant_u32(0)?;
    let grand = kernel.load_at(tier.grand, head)?;

    let groups = {
        let mut lanes = kernel.lanes()?;
        let total = lanes.splat_id::<U32, LANES>(grand)?;
        let short = lanes.splat_bits::<U32, LANES>(per_block - 1)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;

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
