#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::layout::BINDINGS;
use grit::{Error, Host};

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
    assert!(matches!(out, Err(Error::Staged { .. })));

    let after = held.read(0, ORDINARY).unwrap();
    let intact = after.iter().all(|word| *word == 7);
    eprintln!("{intact}");

    assert!(intact);
}
