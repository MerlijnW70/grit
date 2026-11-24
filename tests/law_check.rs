#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
#[path = "law_parallel/pinned.rs"]
#[allow(dead_code)]
mod pinned;
#[path = "law_check/pins.rs"]
mod pins;
#[path = "law_check/served.rs"]
mod served;

use common::device;

use grit::serve3::Server;

const RECORDED: [(&str, &str); 4] = [
    (
        "life-session",
        "f28a61b82d17b0f8a63425dc34878db3dbb86519632aa11798239188a0472e15",
    ),
    (
        "heat-session",
        "d8f687f4ac7be461a23e976400a44a12a29388ab715b26ffb7b7527aadaa5004",
    ),
    (
        "tools-list",
        "9d309031d26ffda6713ec450689e19d524dc65d906ba601a285165182865ce03",
    ),
    (
        "describe",
        "c2835eba8090e60f715751173efed70c06b1524a592cf318089b42dfdb9c868b",
    ),
];

#[test]
fn c4_base_answers_are_the_bytes_recorded_on_base() {
    let Some(host) = device("law_check c4") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let got = [
        ("life-session", pins::life_session(&mut server)),
        ("heat-session", pins::heat_session(&mut server)),
        ("tools-list", pins::listing(&mut Server::new(None))),
        ("describe", pins::described(&mut Server::new(None))),
    ];
    for (name, hash) in &got {
        println!("RECORD {name} {hash}");
    }
    for ((name, hash), (pinned, wanted)) in got.iter().zip(RECORDED) {
        assert_eq!(*name, pinned);
        assert_eq!(hash, wanted, "{name}");
    }
}
