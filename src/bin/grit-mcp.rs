use std::io::Write;

use grit::Host;
use grit::serve3::Server;
use grit::serve3::transport::{Line, Lines, not_utf8, too_long};
use grit::wire::written;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = Host::open()?;
    let mut server = Server::new(host.as_ref());
    let mut lines = Lines::new(std::io::stdin().lock());
    let mut out = std::io::stdout();
    while let Some(line) = lines.next_line()? {
        let answer = match line {
            Line::Long { bytes } => Some(too_long(bytes)),
            Line::Held(bytes) => {
                let held = bytes.len();
                match String::from_utf8(bytes) {
                    Ok(line) => server.heard(&line),
                    Err(why) => Some(not_utf8(held, why.utf8_error().valid_up_to())),
                }
            }
        };
        if let Some(answer) = answer {
            writeln!(out, "{}", written(&answer))?;
            out.flush()?;
        }
    }
    Ok(())
}
