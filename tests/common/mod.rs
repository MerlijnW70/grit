#![allow(clippy::panic)]

use std::ffi::OsString;
use std::fmt::Display;

use grit::{Error, Host};

pub enum Verdict<T> {
    Run(T),
    Skip(String),
    Fail(String),
}

pub struct Asked {
    pub named: Option<String>,
    pub required: Result<bool, String>,
}

impl Asked {
    pub fn from(named: Option<OsString>, required: Option<OsString>) -> Self {
        let named = named
            .map(|held| held.to_string_lossy().into_owned())
            .filter(|held| !held.is_empty());
        let required = match required.as_ref().map(|held| held.to_str()) {
            None | Some(Some("" | "0")) => Ok(false),
            Some(Some("1")) => Ok(true),
            Some(_) => Err(format!(
                "GRIT_REQUIRE_DEVICE is {:?}; it must be 1, 0 or unset",
                required.unwrap_or_default()
            )),
        };
        Self { named, required }
    }

    fn here() -> Self {
        Self::from(
            std::env::var_os("GRIT_DEVICE"),
            std::env::var_os("GRIT_REQUIRE_DEVICE"),
        )
    }
}

pub fn verdict<T>(opened: Result<Option<T>, Error>, asked: &Asked, label: &str) -> Verdict<T> {
    let required = match &asked.required {
        Ok(required) => *required,
        Err(why) => return Verdict::Fail(format!("{label}: {why}")),
    };
    match opened {
        Ok(Some(host)) => Verdict::Run(host),
        Ok(None) => match &asked.named {
            Some(named) => Verdict::Fail(format!(
                "{label}: GRIT_DEVICE names {named:?}, and this machine has no Vulkan driver"
            )),
            None if required => Verdict::Fail(format!(
                "{label}: GRIT_REQUIRE_DEVICE is 1, and this machine has no Vulkan driver"
            )),
            None => Verdict::Skip(label.to_owned()),
        },
        Err(why) => Verdict::Fail(format!("{label}: {why}")),
    }
}

pub fn skip(why: impl Display) {
    let why = why.to_string();
    let asked = Asked::here();
    match asked.required {
        Ok(false) => {}
        Ok(true) => panic!("GRIT_REQUIRE_DEVICE is 1, and this test would have skipped: {why}"),
        Err(wrong) => panic!("{wrong}; this test would have skipped: {why}"),
    }
    if why.is_empty() {
        eprintln!("SKIPPED");
    } else {
        eprintln!("SKIPPED {why}");
    }
}

pub fn device(label: &str) -> Option<Host> {
    match verdict(Host::open(), &Asked::here(), label) {
        Verdict::Run(host) => Some(host),
        Verdict::Skip(why) => {
            skip(why);
            None
        }
        Verdict::Fail(why) => panic!("{why}"),
    }
}
