#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// What the library holds only to show a picture in a window: the names of
/// the surface and swapchain extensions it asks for, and the calls it makes
/// to make a surface, a swapchain and to present to one.
const WINDOW_CODE: [&str; 6] = [
    "VK_KHR_surface",
    "VK_KHR_win32_surface",
    "VK_KHR_swapchain",
    "create_win32_surface",
    "acquire_next_image",
    "queue_present",
];

fn library(features: &[&str], into: &Path) -> Vec<u8> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(cargo);
    build
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["build", "--lib", "--locked"])
        .args(features)
        .env("CARGO_TARGET_DIR", into)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env("CARGO_PROFILE_DEV_DEBUG", "0");
    let status = build.status().unwrap();
    assert!(status.success(), "cargo build --lib {features:?}: {status}");
    let rlib = into.join("debug").join("libgrit.rlib");
    std::fs::read(&rlib).unwrap_or_else(|why| panic!("{}: {why}", rlib.display()))
}

fn holds(bytes: &[u8], word: &str) -> bool {
    bytes
        .windows(word.len())
        .any(|window| window == word.as_bytes())
}

#[test]
fn the_library_holds_no_surface_or_swapchain_code_without_the_window_feature() {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("headless");
    let with = library(&[], &scratch.join("window"));
    let without = library(&["--no-default-features"], &scratch.join("bare"));
    let mut wrong = Vec::new();
    for word in WINDOW_CODE {
        println!(
            "{word}: default library {}, headless library {}",
            if holds(&with, word) {
                "holds it"
            } else {
                "does not"
            },
            if holds(&without, word) {
                "holds it"
            } else {
                "does not"
            }
        );
        if !holds(&with, word) {
            wrong.push(format!(
                "the default library does not hold {word}: the check cannot see it"
            ));
        }
        if holds(&without, word) {
            wrong.push(format!("the headless library holds {word}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
