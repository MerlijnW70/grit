use std::io::{BufRead, Write};

use grit::Host;
use grit::serve::handle;
use grit::wire::{read, written};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = Host::open()?;
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = read(&line) else {
            continue;
        };
        if let Some(answer) = handle(host.as_ref(), &request) {
            writeln!(out, "{}", written(&answer))?;
            out.flush()?;
        }
    }
    Ok(())
}
