use std::io::{BufRead, Write};

use grit::Host;
use grit::serve::heard;
use grit::wire::written;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = Host::open()?;
    let mut stdin = std::io::stdin().lock();
    let mut out = std::io::stdout();
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if stdin.read_until(b'\n', &mut bytes)? == 0 {
            return Ok(());
        }
        let line = String::from_utf8_lossy(&bytes);
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = heard(host.as_ref(), &line) {
            writeln!(out, "{}", written(&answer))?;
            out.flush()?;
        }
    }
}
