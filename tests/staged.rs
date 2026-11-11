#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::layout::{BINDINGS, STATE};
use grit::live;
use grit::resident::STAGING;
use grit::{Error, Groups};

#[test]
fn a_binding_larger_than_the_staging_cap_is_written_and_read_in_pieces() {
    let Some(host) = device("a_binding_larger_than_the_staging_cap_is_written_and_read_in_pieces")
    else {
        return;
    };
    let cap = usize::try_from(STAGING / 4).unwrap();
    let range = usize::try_from(host.limits().storage_range / 4).unwrap();
    let words = (cap * 2 + 3).min(range);
    assert!(
        words > cap + 2,
        "{} holds no more than one piece",
        host.name()
    );
    let tick = live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[STATE as usize] = words;
    let mut held = host.resident(&sizes, &modules).unwrap();
    let staged = held.staging();
    assert_eq!(staged.len(), 3);
    assert!(staged.iter().all(|&room| room == cap), "{staged:?}");

    let pattern: Vec<u32> = (0..words as u32)
        .map(|at| at.wrapping_mul(0x9E37_79B1))
        .collect();
    held.write(STATE as usize, &pattern).unwrap();
    assert_eq!(held.read(STATE as usize, words).unwrap(), pattern);
    held.write_at(STATE as usize, cap - 1, &[7, 8, 9]).unwrap();
    let back = held.read(STATE as usize, cap + 2).unwrap();
    assert_eq!(&back[cap - 1..cap + 2], &[7, 8, 9]);
    assert!(held.staging().iter().all(|&room| room == cap));

    let stages: Vec<&[Groups]> = vec![&[Groups::Fixed(0)]];
    let (_, read) = held.staged_reading(&stages, STATE as usize, words).unwrap();
    assert_eq!(read[..cap - 1], pattern[..cap - 1]);
    assert_eq!(read[cap + 2..], pattern[cap + 2..]);
    let grown = held.staging();
    assert_eq!(grown[0], cap);
    assert!(grown[1..].contains(&words), "{grown:?}");
    println!("staging after a frame that read {words} words: {grown:?}");

    assert!(matches!(
        held.read(STATE as usize, words + 1),
        Err(Error::Overrun { .. })
    ));
}

/// A read takes one copy a piece and no more, and a frame that keeps
/// reading what its slot holds neither grows the slot nor records again:
/// the per-frame path does no host work the uncapped one did not.
#[test]
fn a_read_copies_once_a_piece_and_a_repeated_frame_reuses_its_recording() {
    let Some(host) = device("a_read_copies_once_a_piece_and_a_repeated_frame_reuses_its_recording")
    else {
        return;
    };
    let cap = usize::try_from(STAGING / 4).unwrap();
    let range = usize::try_from(host.limits().storage_range / 4).unwrap();
    let words = (cap + 1).min(range);
    let tick = live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[STATE as usize] = words;
    let mut held = host.resident(&sizes, &modules).unwrap();
    let pattern: Vec<u32> = (0..words as u32).collect();
    held.write(STATE as usize, &pattern).unwrap();

    let copies = |held: &grit::Resident<'_>| held.copying().1;
    let before = copies(&held);
    assert_eq!(held.read(STATE as usize, 1000).unwrap(), pattern[..1000]);
    assert_eq!(copies(&held) - before, 1);
    let before = copies(&held);
    assert_eq!(held.read(STATE as usize, words).unwrap(), pattern);
    assert_eq!(copies(&held) - before, words.div_ceil(cap) as u64);

    let stages: Vec<&[Groups]> = vec![&[Groups::Fixed(0)]];
    let _ = held.staged_reading(&stages, STATE as usize, 1000).unwrap();
    let (staging, reused) = (held.staging(), held.reused());
    for _ in 0..8 {
        let (_, read) = held.staged_reading(&stages, STATE as usize, 1000).unwrap();
        assert_eq!(read, pattern[..1000]);
    }
    assert_eq!(held.staging(), staging);
    assert_eq!(held.reused() - reused, 8);
}
