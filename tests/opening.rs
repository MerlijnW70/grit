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
        said_in_words(error, parts);
    }
}

#[track_caller]
fn said_in_words(error: &Error, parts: &[&str]) {
    let said = error.to_string();
    assert!(sentence(&said), "{error:?} says {said:?}");
    assert!(!said.contains("None"), "{error:?} says {said:?}");
    for part in parts {
        assert!(said.contains(part), "{error:?} says {said:?}, not {part:?}");
    }
}

#[test]
fn a_failure_to_open_says_which_step_failed_and_with_what_code() {
    said_in_words(
        &Error::NoInstance(vk::Result::ERROR_LAYER_NOT_PRESENT),
        &["instance", "ERROR_LAYER_NOT_PRESENT"],
    );
    said_in_words(
        &Error::NoDeviceList(vk::Result::ERROR_INITIALIZATION_FAILED),
        &["devices", "ERROR_INITIALIZATION_FAILED"],
    );
    said_in_words(
        &Error::NoDevice {
            name: "NVIDIA GeForce RTX 4080".to_owned(),
            code: vk::Result::ERROR_DEVICE_LOST,
        },
        &["NVIDIA GeForce RTX 4080", "ERROR_DEVICE_LOST", "device"],
    );
    said_in_words(
        &Error::NoSuchDevice {
            wanted: "4080".to_owned(),
            present: Vec::new(),
        },
        &["4080", "no compute device"],
    );
    let Err(why) = (unsafe { ash::Entry::load_from("grit-no-such-vulkan-loader") }) else {
        panic!("a library that does not exist was loaded");
    };
    said_in_words(&Error::NoLoader(why), &["loader"]);
}

fn asked(named: Option<&str>, required: Option<&str>) -> common::Asked {
    common::Asked::from(named.map(OsString::from), required.map(OsString::from))
}

fn said<T>(verdict: common::Verdict<T>) -> (&'static str, String) {
    match verdict {
        common::Verdict::Run(_) => ("run", String::new()),
        common::Verdict::Skip(why) => ("skip", why),
        common::Verdict::Fail(why) => ("fail", why),
    }
}

#[test]
fn the_helper_runs_skips_and_fails_on_what_it_was_asked_for() {
    let none = || Ok::<Option<u8>, Error>(None);
    let some = || Ok::<Option<u8>, Error>(Some(1));
    let missing = || {
        Err::<Option<u8>, Error>(Error::NoSuchDevice {
            wanted: "Radon".to_owned(),
            present: vec!["AMD Radeon(TM) Graphics".to_owned()],
        })
    };
    let broken =
        || Err::<Option<u8>, Error>(Error::NoInstance(vk::Result::ERROR_LAYER_NOT_PRESENT));

    for required in [None, Some("0"), Some(""), Some("1")] {
        for named in [None, Some(""), Some("4080")] {
            let asking = asked(named, required);
            assert_eq!(said(common::verdict(some(), &asking, "it")).0, "run");
            let (missing_is, missing_why) = said(common::verdict(missing(), &asking, "it"));
            assert_eq!(missing_is, "fail");
            assert!(
                missing_why.contains("Radon") && missing_why.contains("AMD Radeon(TM) Graphics")
            );
            assert_eq!(said(common::verdict(broken(), &asking, "it")).0, "fail");
            let wants_one = required == Some("1") || named == Some("4080");
            let (nothing_is, nothing_why) = said(common::verdict(none(), &asking, "it"));
            assert_eq!(
                nothing_is,
                if wants_one { "fail" } else { "skip" },
                "{named:?} {required:?} {nothing_why}"
            );
        }
    }

    for wrong in ["yes", "true", "2", " 1", "1 "] {
        let asking = asked(None, Some(wrong));
        assert!(asking.required.is_err(), "{wrong:?}");
        assert_eq!(said(common::verdict(some(), &asking, "it")).0, "fail");
        assert_eq!(said(common::verdict(none(), &asking, "it")).0, "fail");
    }
    assert_eq!(asked(Some(""), None).named, None);
    assert_eq!(asked(Some("4080"), None).named.as_deref(), Some("4080"));
}

#[test]
#[ignore]
fn probe_a_skip_inside_a_test() {
    common::skip("probe_a_skip_inside_a_test after the device was found");
}

#[test]
fn a_skip_inside_a_test_fails_when_a_device_is_required() {
    if !here("a_skip_inside_a_test_fails_when_a_device_is_required") {
        return;
    }
    let quiet = child("probe_a_skip_inside_a_test", &[]);
    assert!(quiet.passed, "{}", quiet.said);
    assert!(
        quiet.said.contains("SKIPPED probe_a_skip_inside_a_test"),
        "{}",
        quiet.said
    );

    let required = child(
        "probe_a_skip_inside_a_test",
        &[("GRIT_REQUIRE_DEVICE", Set::To(OsStr::new("1")))],
    );
    assert!(!required.passed, "{}", required.said);
    assert!(
        required.said.contains("would have skipped"),
        "{}",
        required.said
    );
}
