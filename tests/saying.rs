#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use ash::vk;
use grit::Error;
use grit::compact::Plan;

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

#[test]
fn a_vulkan_code_becomes_the_error_that_carries_it() {
    let from = Error::from(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY);
    match from {
        Error::Vulkan(code) => assert_eq!(code, vk::Result::ERROR_OUT_OF_DEVICE_MEMORY),
        other => panic!("a Vulkan code became {other:?}"),
    }
    says(&from, &["OUT_OF_DEVICE_MEMORY"]);
}

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
