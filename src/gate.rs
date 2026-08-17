//! One branch, taken by a whole workgroup or by none of it.
//!
//! # Why a compaction still needs a coarse level
//!
//! The work list makes the *settle* proportional to what is moving. It does nothing for the passes
//! that produce it. At sixteen million cells four of them still touched every quad there is:
//!
//! ```text
//!   decay   every wake counter, one closer to nothing
//!   mark    every counter read again, and compacted into a bucket
//!   gather  every bucket slot read, and written to the list or to a dump
//! ```
//!
//! Together that is four million invocations doing eight loads and eight stores each, against a
//! settle of seven hundred. The claim this repository exists to make — that the work does not grow
//! with the world — was only ever true of the middle of the loop.
//!
//! # What is being skipped, and what says it is safe
//!
//! A block is [`crate::sweep::cells`] quads. Two questions decide whether one can be passed over,
//! and each pass asks whichever it can answer soonest:
//!
//! * [`crate::layout::HOT`] — is anything in this block awake? Written by the same code that writes
//!   [`crate::layout::AWAKE`], with the same countdown and the same nine-neighbour dilation, so a
//!   block reading nought provably has no awake quad in it.
//! * [`crate::layout::COUNTS`] — how many quads did this block contribute to the list? `gather` has
//!   already loaded it before it needs to decide, so its gate costs nothing at all.
//!
//! # And why a branch is allowed here when `select` is the rule everywhere else
//!
//! `decisions/DR-0003` refuses a branch whose condition differs between lanes, because a subgroup
//! instruction inside one is defined over whichever lanes happen to be executing. The condition here
//! is a single word loaded at the *workgroup's* own index, so every lane has the same answer by
//! construction — and [`simdr::lanes::Lanes::any_uniform`] is how a program says so in a form the
//! branch will accept. The vote costs one instruction and turns a forbidden branch into a legal one.
//!
//! Opened and closed by hand rather than through `Lanes::if_uniform`, because the body wants
//! `Kernel::load_at` and `Kernel::store_at` and that helper hands out only the lane builder.
//! `runner/src/kernels/control.rs` in `simdr` keeps one in longhand for the same reason.

use simdr::kernel::Kernel;
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;
use simdr::spec::SelectionControl;

/// Run `body` only where `value` is above nothing.
///
/// `value` must be uniform across the subgroup — one word loaded at the workgroup's own index, not
/// at a lane's. A per-lane value here would pass the vote and mean something else.
///
/// # Errors
///
/// [`LaneError::Build`] if an instruction cannot be emitted, or whatever `body` returns.
pub fn if_any<const LANES: u32, F>(
    kernel: &mut Kernel<U32>,
    value: Id,
    body: F,
) -> Result<(), LaneError>
where
    F: FnOnce(&mut Kernel<U32>) -> Result<(), LaneError>,
{
    let taken = {
        let mut lanes = kernel.lanes()?;
        let none = lanes.splat_bits::<U32, LANES>(0)?;
        let now = lanes.splat_id::<U32, LANES>(value)?;
        let live = lanes.greater_than(now, none)?;
        lanes.any_uniform(live)?
    };

    let then_block = kernel.module().alloc_id()?;
    let merge_block = kernel.module().alloc_id()?;

    // The merge is declared before the branch: a consumer has to know where the arms rejoin before
    // it has walked either of them.
    kernel
        .module()
        .selection_merge(merge_block, SelectionControl::None)?;
    kernel
        .module()
        .branch_conditional(taken.id(), then_block, merge_block)?;
    kernel.module().label_at(then_block)?;
    body(kernel)?;
    kernel.module().branch(merge_block)?;
    kernel.module().label_at(merge_block)?;
    Ok(())
}
