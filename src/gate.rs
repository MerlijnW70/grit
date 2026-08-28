use simdr::kernel::Kernel;
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;
use simdr::spec::SelectionControl;

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
