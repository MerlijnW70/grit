#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::ffi::{OsStr, OsString};
use std::process::Command;

use ash::vk;
use grit::{Error, Host};

const MISSING: &str = "grit-no-such-driver-manifest.json";

#[derive(Clone, Copy)]
enum Set<'a> {
    To(&'a OsStr),
    Gone,
}

struct Child {
    passed: bool,
    said: String,
}

fn child(probe: &str, settings: &[(&str, Set<'_>)]) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        probe,
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]);
    command.env_remove("GRIT_DEVICE");
    command.env_remove("GRIT_REQUIRE_DEVICE");
    for (name, set) in settings {
        match set {
            Set::To(value) => command.env(name, value),
            Set::Gone => command.env_remove(name),
        };
    }
    let out = command.output().unwrap();
    let mut said = String::from_utf8_lossy(&out.stdout).into_owned();
    said.push_str(&String::from_utf8_lossy(&out.stderr));
    Child {
        passed: out.status.success(),
        said,
    }
}

fn missing() -> OsString {
    std::env::temp_dir().join(MISSING).into_os_string()
}

fn without_vulkan<'a>(path: &'a OsStr, more: &[(&'a str, Set<'a>)]) -> Vec<(&'a str, Set<'a>)> {
    let mut settings = vec![
        ("VK_ICD_FILENAMES", Set::To(path)),
        ("VK_DRIVER_FILES", Set::To(path)),
        ("VK_ADD_DRIVER_FILES", Set::Gone),
    ];
    settings.extend_from_slice(more);
    settings
}

fn here(label: &str) -> bool {
    common::device(label).is_some()
}

#[test]
#[ignore]
fn probe_through_the_helper() {
    let Some(host) = common::device("probe_through_the_helper") else {
        return;
    };
    eprintln!("RAN ON {}", host.name());
}

#[test]
#[ignore]
fn probe_bare() {
    match Host::open() {
        Ok(Some(host)) => eprintln!("OPENED {}", host.name()),
        Ok(None) => eprintln!("OPENED NOTHING"),
        Err(why) => eprintln!("OPENED ERROR {why:?}"),
    }
}

#[test]
fn a_card_named_that_is_not_here_fails_the_test() {
    if !here("a_card_named_that_is_not_here_fails_the_test") {
        return;
    }
    let wanted = OsStr::new("zzz-no-card-is-called-this");
    let ran = child(
        "probe_through_the_helper",
        &[("GRIT_DEVICE", Set::To(wanted))],
    );
    assert!(!ran.passed, "{}", ran.said);
    assert!(
        ran.said.contains("zzz-no-card-is-called-this"),
        "{}",
        ran.said
    );
    assert!(!ran.said.contains("RAN ON"), "{}", ran.said);
}

#[test]
fn a_card_named_on_a_machine_without_vulkan_fails_the_test() {
    if !here("a_card_named_on_a_machine_without_vulkan_fails_the_test") {
        return;
    }
    let path = missing();
    let ran = child(
        "probe_through_the_helper",
        &without_vulkan(&path, &[("GRIT_DEVICE", Set::To(OsStr::new("4080")))]),
    );
    assert!(!ran.passed, "{}", ran.said);
    assert!(!ran.said.contains("RAN ON"), "{}", ran.said);
}

#[test]
fn a_required_device_fails_the_test_where_it_would_have_skipped() {
    if !here("a_required_device_fails_the_test_where_it_would_have_skipped") {
        return;
    }
    let path = missing();
    let ran = child(
        "probe_through_the_helper",
        &without_vulkan(&path, &[("GRIT_REQUIRE_DEVICE", Set::To(OsStr::new("1")))]),
    );
    assert!(!ran.passed, "{}", ran.said);
    assert!(!ran.said.contains("RAN ON"), "{}", ran.said);
}

#[test]
fn a_machine_without_vulkan_and_nothing_named_still_skips() {
    if !here("a_machine_without_vulkan_and_nothing_named_still_skips") {
        return;
    }
    let path = missing();
    let bare = child("probe_bare", &without_vulkan(&path, &[]));
    assert!(bare.passed, "{}", bare.said);
    assert!(bare.said.contains("OPENED NOTHING"), "{}", bare.said);

    let ran = child("probe_through_the_helper", &without_vulkan(&path, &[]));
    assert!(ran.passed, "{}", ran.said);
    assert!(
        ran.said.contains("SKIPPED probe_through_the_helper"),
        "{}",
        ran.said
    );
}

#[test]
fn a_require_flag_that_is_not_zero_or_one_fails_rather_than_being_ignored() {
    if !here("a_require_flag_that_is_not_zero_or_one_fails_rather_than_being_ignored") {
        return;
    }
    let path = missing();
    for value in ["yes", "true", "2", " 1"] {
        let ran = child(
            "probe_through_the_helper",
            &without_vulkan(
                &path,
                &[("GRIT_REQUIRE_DEVICE", Set::To(OsStr::new(value)))],
            ),
        );
        assert!(!ran.passed, "{value:?} {}", ran.said);
        assert!(
            ran.said.contains("GRIT_REQUIRE_DEVICE"),
            "{value:?} {}",
            ran.said
        );
    }
}

#[test]
fn a_require_flag_of_zero_or_empty_is_the_same_as_none() {
    if !here("a_require_flag_of_zero_or_empty_is_the_same_as_none") {
        return;
    }
    let path = missing();
    for value in ["0", ""] {
        let ran = child(
            "probe_through_the_helper",
            &without_vulkan(
                &path,
                &[("GRIT_REQUIRE_DEVICE", Set::To(OsStr::new(value)))],
            ),
        );
        assert!(ran.passed, "{value:?} {}", ran.said);
        assert!(
            ran.said.contains("SKIPPED probe_through_the_helper"),
            "{value:?} {}",
            ran.said
        );
    }
}

#[cfg(windows)]
fn unreadable() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[0x0034, 0xD800, 0x0030])
}

#[cfg(unix)]
fn unreadable() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![b'4', 0xFF, b'0'])
}

#[test]
fn a_card_named_in_bytes_that_are_not_text_is_not_read_as_no_name() {
    if !here("a_card_named_in_bytes_that_are_not_text_is_not_read_as_no_name") {
        return;
    }
    let name = unreadable();
    let ran = child(
        "probe_through_the_helper",
        &[("GRIT_DEVICE", Set::To(name.as_os_str()))],
    );
    assert!(!ran.passed, "{}", ran.said);
    assert!(!ran.said.contains("RAN ON"), "{}", ran.said);
}

#[test]
fn the_card_that_is_here_runs_under_any_case_and_when_required() {
    let Some(host) = common::device("the_card_that_is_here_runs") else {
        return;
    };
    let name = host.name().to_owned();
    drop(host);
    for wanted in [name.to_uppercase(), name.to_lowercase(), name.clone()] {
        let ran = child(
            "probe_through_the_helper",
            &[
                ("GRIT_DEVICE", Set::To(OsStr::new(&wanted))),
                ("GRIT_REQUIRE_DEVICE", Set::To(OsStr::new("1"))),
            ],
        );
        assert!(ran.passed, "{wanted:?} {}", ran.said);
        assert!(ran.said.contains(&format!("RAN ON {name}")), "{}", ran.said);
    }
}

fn sentence(said: &str) -> bool {
    let words = said
        .split(|one: char| !one.is_ascii_alphabetic())
        .filter(|word| word.len() >= 2)
        .count();
    words >= 5
}

#[test]
fn a_device_that_is_not_here_is_named_with_the_devices_that_are() {
    let said = Error::NoSuchDevice {
        wanted: "shimmer".to_owned(),
        present: vec![
            "NVIDIA GeForce RTX 4080".to_owned(),
            "AMD Radeon".to_owned(),
        ],
    }
    .to_string();
    assert!(sentence(&said), "{said}");
    assert!(said.contains("shimmer"), "{said}");
    assert!(said.contains("device"), "{said}");
    assert!(said.contains("NVIDIA GeForce RTX 4080"), "{said}");
    assert!(said.contains("AMD Radeon"), "{said}");
}

#[test]
fn every_error_is_a_sentence_that_names_what_failed() {
    let every = [
        (
            Error::Vulkan(vk::Result::ERROR_DEVICE_LOST),
            vec!["DEVICE_LOST", "Vulkan"],
        ),
        (Error::NoComputeDevice, vec!["compute", "device"]),
        (Error::NoMemory { bytes: 4096 }, vec!["4096", "memory"]),
        (
            Error::NoSuchBinding {
                index: 37,
                held: 12,
            },
            vec!["37", "12", "binding"],
        ),
        (
            Error::Overrun {
                index: 41,
                needed: 5000,
                held: 64,
            },
            vec!["41", "5000", "64"],
        ),
        (
            Error::Staged {
                needed: 300,
                staging: 16,
            },
            vec!["300", "16", "staging"],
        ),
        (Error::NoSparse, vec!["sparse"]),
        (Error::StillInFlight, vec!["flight"]),
        (
            Error::NoSuchTicket {
                ticket: 4242,
                held: Some(4141),
            },
            vec!["4242", "4141", "ticket"],
        ),
        (
            Error::NoSuchTicket {
                ticket: 4242,
                held: None,
            },
            vec!["4242", "ticket"],
        ),
        (
            Error::WrongOutputs {
                asked: 23,
                wanted: 57,
            },
            vec!["23", "57", "outputs"],
        ),
        (
            Error::Wedged {
                after: std::time::Duration::from_millis(250),
            },
            vec!["250", "device"],
        ),
        (
            Error::WrongPasses {
                given: 77,
                held: 91,
            },
            vec!["77", "91", "passes"],
        ),
        (
            Error::Strayed {
                at: 9,
                was: 3,
                now: 5,
            },
            vec!["cell", "9", "3", "5"],
        ),
        (
            Error::Lane(simdr::lanes::LaneError::BadWidth { width: 3 }),
            vec!["kernel", "3"],
        ),
    ];
    for (error, parts) in &every {
        let said = error.to_string();
        assert!(sentence(&said), "{error:?} says {said:?}");
        assert!(!said.contains("None"), "{error:?} says {said:?}");
        for part in parts {
            assert!(said.contains(part), "{error:?} says {said:?}, not {part:?}");
        }
    }
}
