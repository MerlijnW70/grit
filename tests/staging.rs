#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::BINDINGS;
use grit::{Error, Host, Run};

fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            eprintln!("SKIPPED {label}");
            None
        }
        Err(why) => {
            eprintln!("SKIPPED {label} {why}");
            None
        }
    }
}

#[test]
fn a_write_to_the_sparse_binding_is_bounded_by_the_staging_buffer() {
    let Some(host) = device("staging") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED");
        return;
    }

    const ORDINARY: usize = 1024;
    const VAST: usize = 256 * 1024 * 1024;

    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = ORDINARY;
    sizes[1] = VAST;

    let tick = grit::live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let held = host.resident_vast(&sizes, &modules, 1);
    let mut held = match held {
        Ok(held) => held,
        Err(why) => {
            eprintln!("SKIPPED {why}");
            return;
        }
    };

    eprintln!("{ORDINARY}");
    eprintln!("{VAST}");

    held.write(0, &vec![7_u32; ORDINARY]).unwrap();

    let over = ORDINARY + 1;
    let out = held.write(1, &vec![9_u32; over]);
    match &out {
        Ok(()) => eprintln!("{over} {over} {ORDINARY}"),
        Err(Error::Staged { needed, staging }) => {
            eprintln!("{needed} {staging}");
        }
        Err(why) => eprintln!("{why}"),
    }
    assert!(matches!(
        out,
        Err(Error::Staged { needed, staging }) if needed == over && staging == ORDINARY
    ));

    let out = held.write_at(1, 0, &vec![9_u32; over]);
    assert!(matches!(
        out,
        Err(Error::Staged { needed, staging }) if needed == over && staging == ORDINARY
    ));

    let words = vec![9_u32; over];
    let runs = [Run {
        index: 1,
        at: 0,
        words: &words,
    }];
    let out = held.write_many(&runs);
    assert!(matches!(
        out,
        Err(Error::Staged { needed, staging }) if needed == over && staging == ORDINARY
    ));

    let after = held.read(0, ORDINARY).unwrap();
    let intact = after.iter().all(|word| *word == 7);
    eprintln!("{intact}");

    assert!(intact);
}

#[test]
fn a_write_that_ends_on_the_last_word_fits_and_one_past_it_does_not() {
    let Some(host) = device("a_write_that_ends_on_the_last_word_fits") else {
        return;
    };
    let tick = grit::live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = 4;
    let mut held = host.resident(&sizes, &modules).unwrap();

    held.write(0, &[0, 0, 0, 0]).unwrap();
    held.write_at(0, 2, &[FIRST, SECOND]).unwrap();
    assert_eq!(held.read(0, 4).unwrap(), vec![0, 0, FIRST, SECOND]);

    assert!(matches!(
        held.write_at(0, 3, &[FIRST, SECOND]),
        Err(Error::Overrun {
            index: 0,
            needed: 5,
            held: 4
        })
    ));
    assert!(matches!(
        held.read(0, 5),
        Err(Error::Overrun {
            index: 0,
            needed: 5,
            held: 4
        })
    ));
    assert_eq!(held.read(0, 4).unwrap(), vec![0, 0, FIRST, SECOND]);
}

#[test]
fn a_run_that_reaches_past_the_end_is_refused_by_where_it_starts() {
    let Some(host) = device("a_run_that_reaches_past_the_end_is_refused") else {
        return;
    };
    let tick = grit::live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = 4;
    let mut held = host.resident(&sizes, &modules).unwrap();
    held.write(0, &[0, 0, 0, 0]).unwrap();

    let words = [FIRST, SECOND];
    let over = [Run {
        index: 0,
        at: 3,
        words: &words,
    }];
    assert!(matches!(
        held.write_many(&over),
        Err(Error::Overrun {
            index: 0,
            needed: 5,
            held: 4
        })
    ));
    assert_eq!(held.read(0, 4).unwrap(), vec![0, 0, 0, 0]);

    let fitting = [Run {
        index: 0,
        at: 2,
        words: &words,
    }];
    held.write_many(&fitting).unwrap();
    assert_eq!(held.read(0, 4).unwrap(), vec![0, 0, FIRST, SECOND]);
}

const FIRST: u32 = 0x00C0_FFEE;
const SECOND: u32 = 0x00BA_DA55;

#[test]
fn a_write_too_wide_for_its_binding_says_how_wide_the_binding_is() {
    let Some(host) = device("a_write_too_wide_for_its_binding_says_how_wide") else {
        return;
    };
    let tick = grit::live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = 4;
    let mut held = host.resident(&sizes, &modules).unwrap();

    assert!(matches!(
        held.write(0, &[1, 2, 3, 4, 5]),
        Err(Error::Overrun {
            index: 0,
            needed: 5,
            held: 4
        })
    ));
    held.write(0, &[1, 2, 3, 4]).unwrap();
    assert_eq!(held.read(0, 4).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn a_write_at_filling_the_staging_buffer_to_the_word_is_taken() {
    let Some(host) = device("a_write_at_filling_the_staging_buffer_to_the_word") else {
        return;
    };
    if !host.sparse() {
        eprintln!("SKIPPED {}", host.name());
        return;
    }

    const ORDINARY: usize = 1024;
    const VAST: usize = 256 * 1024 * 1024;

    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[0] = ORDINARY;
    sizes[1] = VAST;

    let tick = grit::live::tick(host.width()).unwrap();
    let modules: Vec<&[u32]> = vec![&tick];
    let mut held = match host.resident_vast(&sizes, &modules, 1) {
        Ok(held) => held,
        Err(why) => {
            eprintln!("SKIPPED {why}");
            return;
        }
    };

    let page = held.page(1).unwrap();
    held.bind(1, 0, page).unwrap();
    held.write_at(1, 0, &vec![3_u32; ORDINARY]).unwrap();
    assert!(matches!(
        held.write_at(1, 0, &vec![3_u32; ORDINARY + 1]),
        Err(Error::Staged {
            needed: _,
            staging: ORDINARY
        })
    ));
}
