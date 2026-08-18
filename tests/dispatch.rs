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
