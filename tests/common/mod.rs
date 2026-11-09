use std::fmt::Display;

use grit::Host;

pub fn skip(why: impl Display) {
    let why = why.to_string();
    if why.is_empty() {
        eprintln!("SKIPPED");
    } else {
        eprintln!("SKIPPED {why}");
    }
}

pub fn device(label: &str) -> Option<Host> {
    match Host::open() {
        Ok(Some(host)) => Some(host),
        Ok(None) => {
            skip(label);
            None
        }
        Err(why) => {
            skip(format_args!("{label} {why}"));
            None
        }
    }
}
