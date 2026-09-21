use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

use crate::Groups;
use crate::layout::{
    AWAKE, BUCKETS, COUNTS, HOT, HOT_BASES, HOT_BUCKETS, HOT_COUNTS, HOT_DENSE, HOT_GRAND,
    HOT_INDIRECT, HOT_TOTALS, PIXELS,
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

#[derive(Debug, Clone, Copy)]
pub enum Over {
    Every,
    Listed(u32),
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
}

impl std::fmt::Display for TooBig {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocks { blocks, limit } => write!(out, "{blocks} {limit}"),
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
}

impl Plan {
    /// The most quads one plan for this subgroup can hold.
    ///
    /// `over` refuses once the blocks outrun `sweep::cells` squared, because
    /// the scan over them is two levels deep and reaches no further, and a
    /// block holds `sweep::cells` quads. So the grid a plan can address is
    /// that number cubed: 16 777 216 quads at a subgroup of 32, and 32 768 at
    /// a subgroup of 4. It is not a limit of the device.
    #[must_use]
    pub fn reach(subgroup: u32) -> u64 {
        u64::from(sweep::cells(subgroup)).pow(3)
    }

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

    #[must_use]
    pub const fn padded(&self) -> u32 {
        self.tiles * self.per_block
    }

    #[must_use]
    pub fn sizes(&self) -> [usize; BINDINGS as usize] {
        let padded = self.tiles as usize * self.per_block as usize;
        let hot_padded = self.hot_tiles as usize * self.per_block as usize;
        let spare = self.blocks as usize * self.subgroup as usize;
        [
            8,
            self.grid + self.quads * 4,
            self.blocks as usize * sweep::stride(self.subgroup) as usize,
            padded,
            padded,
            self.per_block as usize,
            self.quads + spare,
            1,
            3,
            self.quads + self.quads,
            1,
            crate::layout::TALLY_WORDS,
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
    match subgroup {
        4 => scanned::<4>(subgroup, from, into, aggregate, None),
        8 => scanned::<8>(subgroup, from, into, aggregate, None),
        16 => scanned::<16>(subgroup, from, into, aggregate, None),
        32 => scanned::<32>(subgroup, from, into, aggregate, None),
        64 => scanned::<64>(subgroup, from, into, aggregate, None),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

pub fn whole(subgroup: u32, from: u32, into: u32, tier: Tier) -> Result<Vec<u32>, LaneError> {
    let held = Some((tier.totals, tier.grand));
    match subgroup {
        4 => scanned::<4>(subgroup, from, into, tier.totals, held),
        8 => scanned::<8>(subgroup, from, into, tier.totals, held),
        16 => scanned::<16>(subgroup, from, into, tier.totals, held),
        32 => scanned::<32>(subgroup, from, into, tier.totals, held),
        64 => scanned::<64>(subgroup, from, into, tier.totals, held),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn scanned<const LANES: u32>(
    subgroup: u32,
    from: u32,
    into: u32,
    aggregate: u32,
    alone: Option<(u32, u32)>,
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
        None => kernel.store_at(aggregate, block, total)?,
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

    let tile = {
        let mut lanes = kernel.lanes()?;
        let which = lanes.splat_id::<U32, LANES>(block)?;
        let by = lanes.splat_bits::<U32, LANES>(per_block.trailing_zeros())?;
        lanes.shift_right_logical(which, by)?
    };

    let tile_base = kernel.load_at(tier.totals, tile.id())?;
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
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
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
