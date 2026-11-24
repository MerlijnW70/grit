#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[cfg(feature = "faults")]
#[path = "world_absorb/pins.rs"]
mod pins;
#[path = "world_absorb/served.rs"]
mod served;

use common::device;

#[cfg(feature = "faults")]
const RECORDED: [(&str, &str, &str); 4] = [
    (
        "life-dense-64x64x2",
        "b5eaaddf420215cd621e3f75e89592a7845cf1ac34feec16ec618a8a4c44ed5b",
        "d6d78d014cfcaa8fffef101f9b116afcbe01901f8b0d564f5120f836be4f1850",
    ),
    (
        "life-parallel-dense-64x64x2",
        "4d069fc0fa43ac964f70ded55415b5d15dc1ef8a27920b2e23852ae4621b2903",
        "d7a924f0edd971c3b7ee32c18da2d4d0a8face9e5d5b6cf2e89ad0de95a6898f",
    ),
    (
        "bays-sparse-64",
        "403cc1825b414ff0f7dcbf71932d7e3c6b8cfc62d5fd26572eaa4b50332c49d9",
        "0fe62bc0072dd3331892beab9039eca429b59965b176eb63eb0dc8e0d71f8043",
    ),
    (
        "bays-parallel-dense-32",
        "1454c4175daff3b4a0ce4b12f4d56214b23601973313ce5b45bbdf2359d260d3",
        "143b36487f8d6f393074cbbd2b39723cd0ae8e5c4acbe782681d1809ec257f41",
    ),
];

#[cfg(feature = "faults")]
const RECORDED_WIDTH: u32 = 32;

#[cfg(feature = "faults")]
#[test]
fn r5_worlds_without_absorb_answer_and_journal_as_recorded_on_base() {
    let Some(host) = device("world_absorb r5") else {
        return;
    };
    let mut got = Vec::new();
    for case in &pins::CASES {
        let (answers, journal) = pins::trace(&host, case);
        println!("RECORD {} {answers} {journal}", case.name);
        got.push((case.name, answers, journal));
    }
    for (((name, answers, journal), (pinned_name, want_answers, want_journal)), case) in
        got.iter().zip(RECORDED).zip(&pins::CASES)
    {
        assert_eq!(*name, pinned_name);
        assert_eq!(answers, want_answers, "{name}: answers");
        if !case.sparse || host.width() == RECORDED_WIDTH {
            assert_eq!(journal, want_journal, "{name}: journal");
        }
    }
}
