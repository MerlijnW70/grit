#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const ARGS: usize = 0;
const MARK: usize = 1;
const SIDE: usize = 2;
const SIZES: [usize; 3] = [8, 1, 1];
const SEVEN: u32 = 7;
const ROUNDS: usize = 4;

fn store(subgroup: u32, binding: u32, words: &[(u32, u32)]) -> Result<Vec<u32>, LaneError> {
    let bindings = u32::try_from(SIZES.len()).unwrap_or(3);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, bindings))?;
    for &(at, value) in words {
        let slot = kernel.module().constant_u32(at)?;
        let word = kernel.module().constant_u32(value)?;
        kernel.store_at(binding, slot, word)?;
    }
    kernel.finish()
}

fn args(subgroup: u32, at: u32, groups: [u32; 3]) -> Vec<u32> {
    let [x, y, z] = groups;
    store(subgroup, 0, &[(at, x), (at + 1, y), (at + 2, z)]).unwrap()
}

fn marker(subgroup: u32) -> Vec<u32> {
    store(subgroup, 1, &[(0, SEVEN)]).unwrap()
}

fn side(subgroup: u32) -> Vec<u32> {
    store(subgroup, 2, &[(0, 1)]).unwrap()
}

fn mark(host: &Host, modules: &[Vec<u32>], laid: &[u32], stages: &[&[Groups]], times: u32) -> u32 {
    let refs: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&SIZES, &refs).unwrap();
    held.write(ARGS, laid).unwrap();
    held.write(MARK, &[0]).unwrap();
    held.write(SIDE, &[0]).unwrap();
    if times > 1 {
        held.staged_frames(stages, times).unwrap();
    } else {
        held.staged(stages).unwrap();
    }
    held.read(MARK, 1).unwrap()[0]
}

const ZERO: [u32; 8] = [0; 8];

#[test]
fn an_indirect_read_sees_a_fixed_writer_in_the_stage_before() {
    let Some(host) = device("an_indirect_read_sees_a_fixed_writer_in_the_stage_before") else {
        return;
    };
    let width = host.width();
    for tag in [Groups::Fixed(1), Groups::Counting(1)] {
        for round in 0..ROUNDS {
            let modules = [args(width, 0, [1, 1, 1]), marker(width)];
            let first = [tag];
            let second = [Groups::From {
                binding: ARGS,
                at: 0,
            }];
            let got = mark(&host, &modules, &ZERO, &[&first, &second], 1);
            assert_eq!(got, SEVEN, "{} writer={tag:?} round {round}", host.name());
        }
    }
}

#[test]
fn an_indirect_read_two_stages_after_its_writer_sees_it() {
    let Some(host) = device("an_indirect_read_two_stages_after_its_writer_sees_it") else {
        return;
    };
    let width = host.width();
    for round in 0..ROUNDS {
        let modules = [args(width, 0, [1, 1, 1]), side(width), marker(width)];
        let first = [Groups::Fixed(1)];
        let second = [Groups::Fixed(1)];
        let third = [Groups::From {
            binding: ARGS,
            at: 0,
        }];
        let got = mark(&host, &modules, &ZERO, &[&first, &second, &third], 1);
        assert_eq!(got, SEVEN, "{} round {round}", host.name());
    }
}

#[test]
fn an_indirect_read_sees_a_writer_that_shares_its_stage_with_other_passes() {
    let Some(host) =
        device("an_indirect_read_sees_a_writer_that_shares_its_stage_with_other_passes")
    else {
        return;
    };
    let width = host.width();
    for round in 0..ROUNDS {
        let modules = [side(width), args(width, 0, [1, 1, 1]), marker(width)];
        let first = [Groups::Fixed(1), Groups::Fixed(1)];
        let second = [Groups::From {
            binding: ARGS,
            at: 0,
        }];
        let got = mark(&host, &modules, &ZERO, &[&first, &second], 1);
        assert_eq!(got, SEVEN, "{} round {round}", host.name());
    }
}

#[test]
fn a_counting_pass_over_no_groups_does_not_stand_in_for_the_barrier() {
    let Some(host) = device("a_counting_pass_over_no_groups_does_not_stand_in_for_the_barrier")
    else {
        return;
    };
    let width = host.width();
    for round in 0..ROUNDS {
        let modules = [side(width), args(width, 0, [1, 1, 1]), marker(width)];
        let first = [Groups::Counting(0), Groups::Fixed(1)];
        let second = [Groups::From {
            binding: ARGS,
            at: 0,
        }];
        let got = mark(&host, &modules, &ZERO, &[&first, &second], 1);
        assert_eq!(got, SEVEN, "{} round {round}", host.name());
    }
}

#[test]
fn an_indirect_read_of_arguments_the_host_wrote_before_the_frame_sees_them() {
    let Some(host) =
        device("an_indirect_read_of_arguments_the_host_wrote_before_the_frame_sees_them")
    else {
        return;
    };
    let width = host.width();
    let laid = [1, 1, 1, 0, 0, 0, 0, 0];
    let from = [Groups::From {
        binding: ARGS,
        at: 0,
    }];
    for round in 0..ROUNDS {
        let alone = [marker(width)];
        assert_eq!(
            mark(&host, &alone, &laid, &[&from], 1),
            SEVEN,
            "{} first stage, round {round}",
            host.name()
        );

        let after = [side(width), marker(width)];
        let first = [Groups::Fixed(1)];
        assert_eq!(
            mark(&host, &after, &laid, &[&first, &from], 1),
            SEVEN,
            "{} after a stage that writes elsewhere, round {round}",
            host.name()
        );
    }
}

#[test]
fn an_indirect_read_sees_a_fixed_writer_that_zeroes_what_the_host_wrote() {
    let Some(host) = device("an_indirect_read_sees_a_fixed_writer_that_zeroes_what_the_host_wrote")
    else {
        return;
    };
    let width = host.width();
    let laid = [1, 1, 1, 0, 0, 0, 0, 0];
    for round in 0..ROUNDS {
        let modules = [args(width, 0, [0, 1, 1]), marker(width)];
        let first = [Groups::Fixed(1)];
        let second = [Groups::From {
            binding: ARGS,
            at: 0,
        }];
        let got = mark(&host, &modules, &laid, &[&first, &second], 1);
        assert_eq!(got, 0, "{} round {round}", host.name());
    }
}

#[test]
fn an_indirect_read_at_an_offset_sees_a_fixed_writer() {
    let Some(host) = device("an_indirect_read_at_an_offset_sees_a_fixed_writer") else {
        return;
    };
    let width = host.width();
    for round in 0..ROUNDS {
        let modules = [args(width, 5, [1, 1, 1]), marker(width)];
        let first = [Groups::Fixed(1)];
        let second = [Groups::From {
            binding: ARGS,
            at: 5,
        }];
        let got = mark(&host, &modules, &ZERO, &[&first, &second], 1);
        assert_eq!(got, SEVEN, "{} round {round}", host.name());
    }
}

#[test]
fn a_repeated_frame_reads_arguments_its_last_stage_wrote_in_the_repeat_before() {
    let Some(host) =
        device("a_repeated_frame_reads_arguments_its_last_stage_wrote_in_the_repeat_before")
    else {
        return;
    };
    let width = host.width();
    for round in 0..ROUNDS {
        let modules = [marker(width), args(width, 0, [1, 1, 1])];
        let first = [Groups::From {
            binding: ARGS,
            at: 0,
        }];
        let second = [Groups::Fixed(1)];
        let got = mark(&host, &modules, &ZERO, &[&first, &second], 2);
        assert_eq!(got, SEVEN, "{} round {round}", host.name());
    }
}

#[test]
fn a_reused_recording_still_orders_the_indirect_read_after_a_fixed_writer() {
    let Some(host) =
        device("a_reused_recording_still_orders_the_indirect_read_after_a_fixed_writer")
    else {
        return;
    };
    let width = host.width();
    let modules = [args(width, 0, [1, 1, 1]), marker(width)];
    let refs: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&SIZES, &refs).unwrap();
    let from = [Groups::From {
        binding: ARGS,
        at: 0,
    }];
    let tags = [
        Groups::Counting(1),
        Groups::Fixed(1),
        Groups::Fixed(1),
        Groups::Fixed(1),
    ];
    for (round, tag) in tags.into_iter().enumerate() {
        held.write(ARGS, &ZERO).unwrap();
        held.write(MARK, &[0]).unwrap();
        let first = [tag];
        held.staged(&[&first, &from]).unwrap();
        assert_eq!(
            held.read(MARK, 1).unwrap()[0],
            SEVEN,
            "{} writer={tag:?} round {round}",
            host.name()
        );
    }
    assert!(
        held.reused() >= 2,
        "{} reused {}",
        host.name(),
        held.reused()
    );
}
