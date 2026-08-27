//! Every refusal this crate can hand back, rendered.
//!
//! # Why a test that only formats things
//!
//! Because a coverage run said no test had ever formatted one. `Error` and `compact::TooBig` are
//! reached on failure paths — a device that will not open, a plan too large, a binding written past
//! its end — and none of those happen in a suite that passes. So every `Display` arm in the crate
//! was code that had never run, which `decisions/DR-0002` is about.
//!
//! **The check is that each message carries the numbers its variant holds**, not that it is worded
//! any particular way. An arm that formats the wrong field, or drops one, is the bug this catches;
//! rewording is not. `Error::NoLoader` is the one variant absent here, because `ash::LoadingError`
//! cannot be built without a loader that failed.

// A test may panic — that is how it reports.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use ash::vk;
use grit::Error;
use grit::compact::Plan;

/// Every number and name `what` holds has to appear in what it says.
#[track_caller]
fn says(what: &Error, parts: &[&str]) {
    let said = what.to_string();
    assert!(!said.is_empty(), "{what:?} rendered as nothing at all");
    for part in parts {
        assert!(
            said.contains(part),
            "{what:?} holds {part} and says {said:?}, so the field is not in the message"
        );
    }
}

#[test]
fn every_error_says_the_numbers_it_holds() {
    says(&Error::NoComputeDevice, &[]);
    says(&Error::StillInFlight, &[]);
    says(
        &Error::Vulkan(vk::Result::ERROR_DEVICE_LOST),
        &["DEVICE_LOST"],
    );
    says(
        &Error::NoSuchDevice {
            wanted: "shimmer".to_owned(),
            present: vec!["one".to_owned(), "two".to_owned()],
        },
        &["shimmer", "one", "two"],
    );
    says(&Error::NoMemory { bytes: 4096 }, &["4096"]);
    says(
        &Error::NoSuchBinding {
            index: 37,
            held: 12,
        },
        &["37", "12"],
    );
    says(
        &Error::Overrun {
            index: 41,
            needed: 5000,
            held: 64,
        },
        &["41", "5000", "64"],
    );
    says(
        &Error::WrongPasses {
            given: 77,
            held: 91,
        },
        &["77", "91"],
    );
    says(
        &Error::Wedged {
            after: Duration::from_millis(250),
        },
        &["250"],
    );
    says(
        &Error::WrongOutputs {
            asked: 23,
            wanted: 57,
        },
        &["23", "57"],
    );
    says(
        &Error::NoSuchTicket {
            ticket: 4242,
            held: Some(4141),
        },
        &["4242", "4141"],
    );

    // **The one arm that is not a substitution.** A ticket with nothing outstanding says so in
    // words, and an arm that reached for `{held:?}` instead would put `None` in front of a caller.
    let nothing = Error::NoSuchTicket {
        ticket: 4242,
        held: None,
    };
    says(&nothing, &["4242"]);
    let said = nothing.to_string();
    assert!(
        !said.contains("None"),
        "a ticket with nothing outstanding says {said:?}"
    );
}

/// A Vulkan code turns into the variant that holds it, and says the same thing.
#[test]
fn a_vulkan_code_becomes_the_error_that_carries_it() {
    let from = Error::from(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY);
    match from {
        Error::Vulkan(code) => assert_eq!(code, vk::Result::ERROR_OUT_OF_DEVICE_MEMORY),
        other => panic!("a Vulkan code became {other:?}"),
    }
    says(&from, &["OUT_OF_DEVICE_MEMORY"]);
}

/// A plan past what two levels of scan reach is refused **by name**.
///
/// `src/bin/scale.rs` prints this at a caller, and until now nothing had ever seen it: the sizes
/// that file runs are all inside the limit.
#[test]
fn a_plan_too_large_says_how_large_and_what_the_limit_is() {
    let width = 32_u32;
    let per_block = grit::sweep::cells(width);
    let over = per_block * per_block + 1;
    let Err(too_big) = Plan::over(width, over) else {
        panic!("{over} blocks at subgroup {width} was not refused");
    };
    let said = too_big.to_string();
    assert!(
        said.contains(&over.to_string()),
        "the refusal does not say how many blocks were asked for: {said:?}"
    );
    assert!(
        said.contains(&(per_block * per_block).to_string()),
        "the refusal does not say what two levels reach: {said:?}"
    );
}
