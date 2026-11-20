use std::io::Write;
use std::path::PathBuf;

use grit::Host;
use grit::serve3::Options;
use grit::serve3::exec::Exec;
use grit::serve3::transport::Lines;
use grit::wire::written;

fn named(flag: &str, variable: &str) -> Option<String> {
    let mut given = std::env::args().skip(1);
    while let Some(one) = given.next() {
        if one == flag {
            return given.next();
        }
        if let Some(value) = one.strip_prefix(&format!("{flag}=")) {
            return Some(value.to_owned());
        }
    }
    std::env::var(variable)
        .ok()
        .filter(|value| !value.is_empty())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let memory = match named("--memory", "GRIT_MEMORY_BYTES") {
        Some(bytes) => Some(bytes.parse::<u64>().map_err(|why| {
            format!("--memory takes a count of bytes, and {bytes:?} is not one: {why}")
        })?),
        None => None,
    };
    let options = Options {
        dir: named("--dir", "GRIT_DIR").map(PathBuf::from),
        memory,
    };
    let dir = options.dir.clone();
    let opener = move || -> Result<Option<Host>, grit::host::Error> {
        let host = Host::open()?;
        if let Some(host) = host.as_ref() {
            match grit::calibrate::taken(host, dir.as_deref()) {
                Ok(taken) => {
                    if let Some(why) = taken.refused {
                        eprintln!("grit-mcp: {why}");
                    }
                }
                Err(why) => eprintln!("grit-mcp: the device was not calibrated: {why}"),
            }
        }
        Ok(host)
    };
    let (exec, answers) = Exec::spawn(Box::new(opener), &options)?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut out = std::io::stdout();
        for answer in answers {
            writeln!(out, "{}", written(&answer))?;
            out.flush()?;
        }
        Ok(())
    });
    let mut lines = Lines::new(std::io::stdin().lock());
    while let Some(line) = lines.next_line()? {
        exec.heard(line);
    }
    exec.finish();
    writer
        .join()
        .map_err(|_| "the writer thread failed")?
        .map_err(Into::into)
}
