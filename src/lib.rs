//! A sparse-grid stream compaction engine, built on `simdr`.
//!
//! # What this repository is for
//!
//! `simdr` emits SPIR-V and declares no dependencies. Its `runner` drives that SPIR-V on a device
//! and is shaped, everywhere, by measurement: upload, dispatch, download, and a number for what
//! that cost. `simdr`'s `decisions/DR-0008` priced the round trip at about a hundred microseconds
//! before any arithmetic happens, and every entry point `runner` offers puts that price where a
//! caller can see it. That figure is a round trip that *moves data*, which is what those entry
//! points do; `tests/floor.rs` puts a submit and a fence with nothing in it at 41.1–43.9 µs.
//!
//! A simulation is the other shape entirely. State that never crosses the bus, several kernels a
//! frame over the same buffers, and only a picture coming back. That primitive does not exist in
//! `runner` and does not belong there — so it is here, in [`host`] and [`resident`], and it is the
//! only Vulkan this crate owns.
//!
//! Nothing in `simdr` changes for this to exist. It arrives as a path dependency and does what it is
//! for.

#![deny(missing_docs)]

/// The front page, pulled in so that `cargo test` compiles the snippet in it.
///
/// `cfg(doctest)` rather than `doc`: this adds no public surface and no second copy of the README on
/// the crate's own page. What it adds is that a snippet which stops compiling stops the build —
/// which is the same arrangement `simdr` uses for its `USING.md`, and the same failure this
/// repository has been finding all week in prose that nothing ran.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;

pub mod compact;
pub mod gate;
pub mod host;
pub mod layout;
pub mod live;
pub mod netlist;
pub mod resident;
pub mod sand;
pub mod sweep;

pub use host::{Error, Host};
pub use resident::{Groups, Resident, Run, Ticket};
