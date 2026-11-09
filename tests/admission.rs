#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use ash::vk;

use grit::host::{Limit, Limits, OPERATIONS};
use grit::{Error, Groups};

fn radeon() -> Limits {
    Limits {
        storage_range: 4_294_967_295,
        allocation: 2_147_483_648,
        groups: 4_294_967_295,
        width: 64,
        stages: vk::ShaderStageFlags::COMPUTE,
        operations: OPERATIONS,
        api: vk::API_VERSION_1_3,
    }
}

fn small() -> Limits {
    Limits {
        storage_range: 1 << 20,
        allocation: 1 << 22,
        groups: 65_535,
        ..radeon()
    }
}

fn too_large(made: &Result<(), Error>, binding: usize, words: usize, limit: Limit, allowed: u64) {
    assert!(
        matches!(
            made,
            Err(Error::TooLarge { binding: at, words: held, limit: named, allowed: most })
                if *at == binding && *held == words && *named == limit && *most == allowed
        ),
        "{made:?}"
    );
}

#[test]
fn a_binding_exactly_at_the_range_is_taken_and_one_word_more_is_not() {
    let limits = small();
    let most = (1 << 20) / 4;
    assert!(limits.admits(&[most], None).is_ok());
    too_large(
        &limits.admits(&[most + 1], None),
        0,
        most + 1,
        Limit::StorageBufferRange,
        1 << 20,
    );
}

#[test]
fn a_range_that_is_not_a_whole_number_of_words_still_holds_the_words_below_it() {
    let limits = radeon();
    let most = 4_294_967_295 / 4;
    let tighter = Limits {
        allocation: u64::MAX,
        ..limits
    };
    assert!(tighter.admits(&[most], None).is_ok());
    too_large(
        &tighter.admits(&[most + 1], None),
        0,
        most + 1,
        Limit::StorageBufferRange,
        4_294_967_295,
    );
}

#[test]
fn a_binding_inside_the_range_and_past_one_allocation_is_refused_by_the_allocation() {
    let limits = Limits {
        storage_range: 1 << 22,
        allocation: 1 << 20,
        ..small()
    };
    let words = (1 << 20) / 4 + 1;
    too_large(
        &limits.admits(&[1, 2, words], None),
        2,
        words,
        Limit::MemoryAllocationSize,
        1 << 20,
    );
    assert!(limits.admits(&[1, 2, (1 << 20) / 4], None).is_ok());
}

#[test]
fn a_sparse_binding_is_held_to_the_range_and_not_to_one_allocation() {
    let limits = Limits {
        storage_range: 1 << 22,
        allocation: 1 << 20,
        ..small()
    };
    let past_one = (1 << 20) / 4 + 1;
    assert!(limits.admits(&[4, past_one], Some(1)).is_ok());
    too_large(
        &limits.admits(&[4, past_one], Some(0)),
        1,
        past_one,
        Limit::MemoryAllocationSize,
        1 << 20,
    );
    let past_range = (1 << 22) / 4 + 1;
    too_large(
        &limits.admits(&[4, past_range], Some(1)),
        1,
        past_range,
        Limit::StorageBufferRange,
        1 << 22,
    );
}

#[test]
fn the_widest_radeon_plan_is_refused_by_the_storage_range() {
    let words = 1_073_741_824;
    too_large(
        &radeon().admits(&[8, words, 3], None),
        1,
        words,
        Limit::StorageBufferRange,
        4_294_967_295,
    );
}

#[test]
fn an_empty_binding_is_counted_as_the_one_word_it_is_given() {
    let limits = Limits {
        storage_range: 3,
        allocation: 3,
        ..small()
    };
    too_large(
        &limits.admits(&[0], None),
        0,
        0,
        Limit::StorageBufferRange,
        3,
    );
    let room = Limits {
        storage_range: 4,
        allocation: 4,
        ..small()
    };
    assert!(room.admits(&[0, 1], None).is_ok());
    assert!(room.admits(&[], None).is_ok());
}

#[test]
fn a_count_of_words_whose_bytes_overflow_is_refused_and_does_not_wrap() {
    let limits = Limits {
        storage_range: u64::MAX,
        allocation: u64::MAX,
        ..small()
    };
    let last = (1 << 62) - 1;
    assert!(limits.admits(&[last], None).is_ok());
    for wrapping in [1 << 62, usize::MAX / 2 + 1, usize::MAX] {
        let refused = limits.admits(&[4, wrapping], None);
        assert!(
            matches!(refused, Err(Error::NoMemory { bytes: u64::MAX })),
            "{wrapping}: {refused:?}"
        );
    }
    too_large(
        &small().admits(&[last], None),
        0,
        last,
        Limit::StorageBufferRange,
        1 << 20,
    );
}

#[test]
fn the_first_binding_past_the_ceiling_is_the_one_named() {
    let limits = small();
    let over = (1 << 20) / 4 + 1;
    too_large(
        &limits.admits(&[4, over, over + 7], None),
        1,
        over,
        Limit::StorageBufferRange,
        1 << 20,
    );
}

#[test]
fn a_dispatch_at_the_limit_is_taken_and_one_group_more_is_not() {
    let limits = small();
    let at = [Groups::Fixed(65_535)];
    assert!(limits.dispatchable(&[&at]).is_ok());

    let first = [Groups::Fixed(1), Groups::Fixed(0)];
    let second = [Groups::Fixed(65_536)];
    let refused = limits.dispatchable(&[&first, &second]);
    assert!(
        matches!(
            refused,
            Err(Error::TooManyGroups {
                pass: 2,
                groups: 65_536,
                allowed: 65_535
            })
        ),
        "{refused:?}"
    );
}

#[test]
fn a_counting_dispatch_is_held_to_the_same_limit() {
    let limits = small();
    let refused = limits.dispatchable(&[&[Groups::Counting(65_536)]]);
    assert!(
        matches!(
            refused,
            Err(Error::TooManyGroups {
                pass: 0,
                groups: 65_536,
                allowed: 65_535
            })
        ),
        "{refused:?}"
    );
    assert!(limits.dispatchable(&[&[Groups::Counting(65_535)]]).is_ok());
}

#[test]
fn an_indirect_dispatch_is_not_counted_on_the_host() {
    let limits = Limits {
        groups: 1,
        ..small()
    };
    let indirect = [Groups::From { binding: 0, at: 0 }];
    assert!(limits.dispatchable(&[&indirect]).is_ok());
    assert!(limits.dispatchable(&[]).is_ok());
}

#[test]
fn every_refusal_says_which_limit_and_which_value_in_a_sentence() {
    let range = Error::TooLarge {
        binding: 1,
        words: 1_073_741_824,
        limit: Limit::StorageBufferRange,
        allowed: 4_294_967_295,
    }
    .to_string();
    assert!(range.contains("binding 1"), "{range}");
    assert!(range.contains("1073741824 words"), "{range}");
    assert!(range.contains("maxStorageBufferRange"), "{range}");
    assert!(range.contains("4294967295 bytes"), "{range}");

    let allocation = Error::TooLarge {
        binding: 0,
        words: 9,
        limit: Limit::MemoryAllocationSize,
        allowed: 32,
    }
    .to_string();
    assert!(
        allocation.contains("maxMemoryAllocationSize"),
        "{allocation}"
    );

    let groups = Error::TooManyGroups {
        pass: 3,
        groups: 65_536,
        allowed: 65_535,
    }
    .to_string();
    assert!(groups.contains("pass 3"), "{groups}");
    assert!(groups.contains("65536"), "{groups}");
    assert!(groups.contains("maxComputeWorkGroupCount[0]"), "{groups}");
    assert!(groups.contains("65535"), "{groups}");

    for sentence in [range, allocation, groups] {
        assert!(sentence.split_whitespace().count() > 8, "{sentence}");
    }
    assert_eq!(
        Limit::WorkGroupCount.to_string(),
        "maxComputeWorkGroupCount[0]"
    );
}
