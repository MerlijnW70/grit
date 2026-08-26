//! Which buffer is which, in one place.
//!
//! Every pass shares one descriptor set — `crate::resident` says why — so every kernel has to agree
//! about what binding three is. They were agreeing in two places, one in `crate::sand` and one in
//! `crate::compact`, which is the arrangement that holds right up until somebody adds a binding to
//! one of them.
//!
//! # The frame, as a chain of buffers
//!
//! ```text
//!   STATE     ──settle──▶ STATE, and AWAKE where something moved
//!   AWAKE     ──mark────▶ BUCKETS, COUNTS
//!   COUNTS    ──scan────▶ BASES, TOTALS
//!   TOTALS    ──scan────▶ TOTALS, GRAND
//!   BUCKETS   ──gather──▶ DENSE
//!   GRAND     ──prepare─▶ INDIRECT
//!   INDIRECT  ──────────▶ how many workgroups the *next* frame's settle runs
//! ```
//!
//! And the same chain again over one word a *block* rather than one a quad, which is what decides
//! how many workgroups the three passes above are dispatched over at all — see [`HOT_BUCKETS`].
//!
//! ```text
//!   HOT           ──hot_mark────▶ HOT_BUCKETS, HOT_COUNTS
//!   HOT_COUNTS    ──hot_scan────▶ HOT_BASES, HOT_TOTALS
//!   HOT_TOTALS    ──hot_scan────▶ HOT_TOTALS, HOT_GRAND
//!   HOT_BUCKETS   ──hot_gather──▶ HOT_DENSE
//!   HOT_GRAND     ──hot_prepare─▶ HOT_INDIRECT
//!   HOT_INDIRECT  ─────────────▶ how many workgroups decay, mark and gather run
//! ```

/// Words the host and the device share.
///
/// ```text
///   0  the frame number      the device increments it
///   1  the dump base         where a discarded write goes
///   2  the brush's column    the host writes from here on
///   3  the brush's row
///   4  the brush's reach, squared
///   5  what the brush pours
///   6  the window's column
///   7  the window's row
/// ```
pub const PARAMS: u32 = 0;
/// One word a cell: what material is there.
pub const STATE: u32 = 1;
/// One region a quad-block: its compacted list, then its dump.
pub const BUCKETS: u32 = 2;
/// One word a quad-block: how many of its quads are awake.
pub const COUNTS: u32 = 3;
/// Where the exclusive prefix of each quad-block lands.
pub const BASES: u32 = 4;
/// One word a tile: the sum of its blocks, then that made exclusive.
pub const TOTALS: u32 = 5;
/// The work list — quad indices, contiguous — and a dump past the end.
pub const DENSE: u32 = 6;
/// One word: how long the work list turned out to be.
pub const GRAND: u32 = 7;
/// Three words Vulkan reads as a workgroup count.
pub const INDIRECT: u32 = 8;
/// One word a quad: whether it needs looking at next frame. A dump past the end.
pub const AWAKE: u32 = 9;
/// One word a cell, as a colour the window can show.
pub const PIXELS: u32 = 10;

/// Six words the settle adds into as it goes: what moved, what of, and roughly where.
///
/// **The only number in this crate that comes back by reduction rather than by scan.** Everything
/// else here compacts — it wants to know *which* quads, in order. Telemetry wants the opposite: not
/// which, but how many, and one answer for the whole device. That is a subgroup reduction folded
/// into the pass that already has the data in registers, and one atomic a workgroup to combine.
///
/// ```text
///   0  cells that changed        what is actually moving
///   1  water cells               ... in the active set, not in the world
///   2  sand cells
///   3  sum of quad column x moved   } divided by word nought, the centre of the motion
///   4  sum of quad row    x moved   }
///   5  quads that moved at all
/// ```
///
/// Words three and four are in *coarse* quads — see [`COARSE`] — because a sum over every moving
/// cell in a sixteen-million-cell world has to fit in thirty-two bits.
pub const TALLY: u32 = 11;

/// One word a *block*: whether anything in it is awake. Padded to a whole number of tiles, and a
/// single dump slot past that.
///
/// # The coarse level, and why the fine one was not enough
///
/// The work list makes the settle proportional to what is moving. It does nothing for the two
/// passes that produce it: [`crate::live::decay`] touches every wake counter there is, and
/// [`crate::live::mark`] reads every one of them again to compact it. At a quarter of a million
/// cells that is invisible; at sixteen million it was most of the frame, and the claim this
/// repository exists to make — that the work does not grow with the world — was only true of the
/// middle of the loop.
///
/// So the same trick, one level up. A block is [`crate::sweep::cells`] quads, and this says whether
/// any of them is awake. It is written by exactly the code that writes [`AWAKE`], with the same
/// countdown and the same nine-neighbour dilation, so the two cannot fall out of step: **a block
/// reading nought has no awake quad in it**, and the two passes above skip it without looking.
///
/// Skipping is a real branch rather than a `select`, which `decisions/DR-0003` permits precisely
/// here: the condition is one word loaded at the workgroup's own index, so every lane of the
/// subgroup has the same answer, and [`simdr::lanes::Lanes::any_uniform`] is how that is said.
///
/// # One dump slot, not one an invocation
///
/// [`AWAKE`]'s discarded writes go to a dump the size of the widest dispatch. This one needs a
/// single word, because every write here is the constant two — identical writes to one address need
/// no ordering, which is the same argument that lets nine invocations wake the same quad.
///
/// It sits past the **padding** rather than past the last block. The pass that compacts this reads
/// a whole tile at a time, so the array is rounded up to `tiles × per_block` the way [`COUNTS`] is,
/// and a dump slot inside that padding would be a hot block that no quad ever woke.
pub const HOT: u32 = 12;

/// One region a tile: the hot blocks it holds, compacted, then its dump.
///
/// # The same three tiers, one level up
///
/// [`HOT`] made the passes that produce the work list *skip* a cold block. It did not stop them
/// being **dispatched** for one: `decay`, `mark` and `gather` still ran a workgroup per block, and
/// at sixteen million cells that is 3 × 16 384 workgroups doing one load and a vote before they
/// exit. `tests/coarse.rs` measures that arrangement at 19.08–19.11 µs on the device at 1024
/// blocks and 37.37–37.50 at 4096, and there is nothing else in that loop whose size is the world's.
///
/// So the compaction is applied to itself. These seven bindings are the same hierarchy as
/// [`BUCKETS`] through [`INDIRECT`], over a different input: not one word a quad, but one word a
/// **block**, which is [`HOT`]. What comes out is a list of the blocks that have something in them,
/// and the three passes are dispatched over *that* instead of over every block there is.
///
/// ```text
///   hot_mark     tiles × per_block blocks  →  hot_counts[tile]  and a bucket each
///   hot_scan ×2                               the tiles against each other
///   hot_gather   tiles × per_block slots   →  hot_dense[], the hot-block list
///   hot_prepare  1                         →  how many workgroups decay, mark and gather run
/// ```
///
/// # Why this level needs no third tier of its own, ever
///
/// Because [`crate::compact::Plan::over`] already refuses past `per_block²` blocks. A tile is
/// `per_block` blocks, so there are at most `per_block` tiles — and one workgroup scans
/// `per_block` entries. The level above this one is a single dispatch by construction, and there
/// is no size at which a fourth would be wanted.
///
/// # A tile and a band are the same partition
///
/// The fine hierarchy's *tile* is `per_block` blocks, and so is the unit one coarse workgroup
/// sweeps. They are not two numbers that happen to agree — they are the same division of the same
/// array, which is why [`crate::compact::Plan`] carries one field for both.
pub const HOT_BUCKETS: u32 = 13;
/// One word a tile: how many of its blocks are hot. See [`HOT_BUCKETS`].
pub const HOT_COUNTS: u32 = 14;
/// Where the exclusive prefix of each tile lands. See [`HOT_BUCKETS`].
pub const HOT_BASES: u32 = 15;
/// One word per tile-of-tiles, which is one — made exclusive in place. See [`HOT_BUCKETS`].
pub const HOT_TOTALS: u32 = 16;
/// The hot-block list — block indices, contiguous — and a dump past the end.
pub const HOT_DENSE: u32 = 17;
/// One word: how many blocks turned out to be hot. See [`HOT_BUCKETS`].
pub const HOT_GRAND: u32 = 18;
/// Three words Vulkan reads as the workgroup count for `decay`, `mark` and `gather`.
pub const HOT_INDIRECT: u32 = 19;

/// How many bindings every pass declares.
pub const BINDINGS: u32 = 20;

/// How many words [`TALLY`] holds.
pub const TALLY_WORDS: usize = 6;

/// How many bits the centre-of-motion sums drop off a quad coordinate before adding it up.
///
/// A sum of `column x moved` over the whole grid would be `2047 x 4 x 4194304`, which is thirty-five
/// bits and therefore wrong rather than large. Eight quads to the unit brings the worst case to
/// about a tenth of what a word holds, and costs the answer sixteen cells of resolution — which,
/// in a window a thousand cells across, is not visible.
pub const COARSE: u32 = 3;

/// Where in [`PARAMS`] the host's half begins.
pub const BRUSH: usize = 2;
/// How many words a brush is: where, how wide, and what of.
pub const BRUSH_WORDS: usize = 4;

/// Where in [`PARAMS`] the window's top-left corner sits: two words, column then row.
///
/// The device never decides where to look, so this is the host's like the brush is — and panning a
/// window over sixteen million cells costs two words a frame rather than a different picture.
pub const LOOKING: usize = 6;
