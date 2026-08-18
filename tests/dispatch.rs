//! What a dispatch of no workgroups does, which is nothing.
//!
//! # Why this is worth a test
//!
//! Because it used to do something. `Groups::Fixed(count)` was dispatched as `count.max(1)`, so a
//! caller asking for no work got one workgroup of it. Nothing in this repository ever asked for
//! none, which is exactly why it went unnoticed — and a caller holding several passes and wanting
//! one of them is the obvious use, since a pass is recorded whether it is wanted or not.
//!
//! A rounded-up dispatch is not a slow dispatch. It runs a kernel over buffers the caller may have
//! had no reason to size for it.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

/// What the kernel writes.
const MARK: u32 = 0x5EED_BEEF;

/// A kernel that writes one word to binding 0 and does nothing else.
fn mark(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let head = kernel.module().constant_u32(0)?;
    let value = kernel.module().constant_u32(MARK)?;
    kernel.store_at(0, head, value)?;
    kernel.finish()
}

/// A kernel that adds one to the word at binding 0 — it reads what a pass before it wrote.
fn bump(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 1))?;
    let uint = kernel.index_type();
    let head = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(0, head)?;
    let next = kernel.module().i_add(uint, held, one)?;
    kernel.store_at(0, head, next)?;
    kernel.finish()
}

/// Open a device, or say why not and hand back nothing.
fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}: no Vulkan device");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label}: {why}");
            None
        }
    }
}

#[test]
fn a_pass_dispatched_over_no_workgroups_does_not_run() {
    let Some(host) = device("a_pass_dispatched_over_no_workgroups_does_not_run") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
    let mut held = host.resident(&[4], &[&module]).expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    held.frame(&[Groups::Fixed(0)]).expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![0],
        "a pass told to run over nothing ran anyway"
    );

    // And one workgroup does what the kernel says, so the test is about the count and not about a
    // kernel that never worked.
    held.frame(&[Groups::Fixed(1)]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK]);
}

#[test]
fn one_pass_of_several_can_be_run_while_the_others_are_skipped() {
    // The use the clamp made impossible. Three passes over one buffer, each writing a different
    // word; only the middle one is dispatched.
    let Some(host) = device("one_pass_of_several_can_be_run_while_the_others_are_skipped") else {
        return;
    };
    let module = mark(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[4], &[&module, &module, &module])
        .expect("a device buffer");
    held.write(0, &[0, 0, 0, 0]).expect("a clear");

    held.frame(&[Groups::Fixed(0), Groups::Fixed(1), Groups::Fixed(0)])
        .expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![MARK]);

    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[Groups::Fixed(0), Groups::Fixed(0), Groups::Fixed(0)])
        .expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![0],
        "no pass ran, so nothing was written"
    );
}

#[test]
fn the_allocation_ceiling_is_a_buffer_this_device_will_actually_hand_out() {
    // Not guessable, and not a round fraction of anything: on this machine's two parts it is
    // 3.997 GiB and 2.00 GiB. A caller with more to hold than this holds it in pieces, so the
    // number has to come from the device rather than from a constant somebody typed.
    let Some(host) = device("the_allocation_ceiling_is_a_buffer_this_device_will_hand_out") else {
        return;
    };
    let ceiling = host.ceiling();
    assert!(ceiling > 0, "a device that can allocate nothing");
    assert!(
        ceiling <= u64::from(u32::MAX),
        "a binding cannot address more than a u32 of bytes, and this says {ceiling}"
    );

    // And it is a buffer the device really will give: one word under it, asked for and freed.
    let words = usize::try_from(ceiling / 4).expect("a count");
    match host.resident(&[words], &[]) {
        Ok(held) => drop(held),
        Err(why) => panic!("the device refused its own ceiling of {ceiling}: {why}"),
    }
}

#[test]
fn a_skipped_pass_does_not_take_the_ordering_with_it() {
    // The failure a cheaper command buffer could introduce. A pass told to run nothing is not
    // recorded at all, so neither is the barrier that would have followed it — and if the barriers
    // were counted off the pass list rather than off what actually runs, the two passes either side
    // of a skipped one would go unordered. Both read the word the other wrote, so an unordered pair
    // reads a stale one and the answer comes back short.
    let Some(host) = device("a_skipped_pass_does_not_take_the_ordering_with_it") else {
        return;
    };
    let module = bump(host.width()).expect("a kernel");
    let mut held = host
        .resident(&[4], &[&module, &module, &module, &module, &module])
        .expect("a device buffer");

    // Every pass runs: five bumps of one workgroup, one lane of which stores, so the word is five.
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[Groups::Fixed(1); 5]).expect("a frame");
    assert_eq!(held.read(0, 1).expect("a read"), vec![5]);

    // Now with three of them skipped, and the two that run are no longer neighbours in the list.
    held.write(0, &[0, 0, 0, 0]).expect("a clear");
    held.frame(&[
        Groups::Fixed(1),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(0),
        Groups::Fixed(1),
    ])
    .expect("a frame");
    assert_eq!(
        held.read(0, 1).expect("a read"),
        vec![2],
        "the second pass did not see what the first wrote"
    );
}
