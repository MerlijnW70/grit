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

/// One word a *block*: whether anything in it is awake. A single dump slot past the end.
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
pub const HOT: u32 = 12;

/// How many bindings every pass declares.
pub const BINDINGS: u32 = 13;

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
