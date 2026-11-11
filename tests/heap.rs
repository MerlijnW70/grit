#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ash::vk;

use grit::Error;
use grit::host::{Limit, Limits, OPERATIONS};

fn limits(range: u64, allocation: u64) -> Limits {
    Limits {
        storage_range: range,
        allocation,
        groups: u32::MAX,
        width: 32,
        stages: vk::ShaderStageFlags::COMPUTE,
        operations: OPERATIONS,
        api: vk::API_VERSION_1_3,
    }
}

#[test]
fn bindings_that_fill_the_heap_exactly_are_taken_and_one_word_more_is_not() {
    let held = limits(1 << 20, 1 << 20);
    let sizes = [1 << 18, 1 << 18, 1 << 17];
    let heap = (1 << 20) * 2 + (1 << 19);
    assert!(held.admits_on(&sizes, None, heap).is_ok());
    let refused = held.admits_on(&sizes, None, heap - 4);
    assert!(
        matches!(refused, Err(Error::TooMuchMemory { bytes, heap: most }) if bytes == heap && most == heap - 4),
        "{refused:?}"
    );
}

#[test]
fn an_empty_binding_counts_as_its_one_word_against_the_heap() {
    let held = limits(1 << 20, 1 << 20);
    assert!(held.admits_on(&[0, 0], None, 8).is_ok());
    assert!(matches!(
        held.admits_on(&[0, 0], None, 7),
        Err(Error::TooMuchMemory { bytes: 8, heap: 7 })
    ));
}

#[test]
fn a_binding_past_the_range_is_named_before_the_heap_is_counted() {
    let held = limits(1 << 20, 1 << 22);
    let refused = held.admits_on(&[4, (1 << 18) + 1], None, 1);
    assert!(
        matches!(
            refused,
            Err(Error::TooLarge {
                binding: 1,
                limit: Limit::StorageBufferRange,
                ..
            })
        ),
        "{refused:?}"
    );
}

#[test]
fn bytes_that_overflow_while_summed_are_refused_and_do_not_wrap() {
    let held = limits(u64::MAX, u64::MAX);
    let half = 1_usize << 61;
    let refused = held.admits_on(&[half, half, half], None, u64::MAX);
    assert!(
        matches!(refused, Err(Error::NoMemory { bytes: u64::MAX })),
        "{refused:?}"
    );
}

#[test]
fn the_heap_refusal_is_a_sentence_that_names_both_numbers() {
    let said = Error::TooMuchMemory {
        bytes: 13_175_408_748,
        heap: 8_589_934_592,
    }
    .to_string();
    assert!(said.contains("13175408748"), "{said}");
    assert!(said.contains("8589934592"), "{said}");
    assert!(said.contains("memory"), "{said}");
}
