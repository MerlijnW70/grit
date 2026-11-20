#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

const QUEUE_CALLS: [&str; 4] = [
    "queue_submit(",
    "queue_bind_sparse(",
    "queue_present(",
    "wait_for_fences(",
];

fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn every_queue_submit_and_fence_wait_goes_through_the_host() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let host = root.join("src").join("host.rs");
    let mut found = Vec::new();
    sources(&root.join("src"), &mut found);
    assert!(found.len() > 10, "{found:?}");
    let mut bypassing = Vec::new();
    for path in found {
        let text = std::fs::read_to_string(&path).unwrap();
        for (at, line) in text.lines().enumerate() {
            for call in QUEUE_CALLS {
                if line.contains(call) && path != host {
                    bypassing.push(format!("{}:{}: {}", path.display(), at + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        bypassing.is_empty(),
        "queue work outside Host::submit, Host::bind_sparse, Host::present and Host::wait:\n{}",
        bypassing.join("\n")
    );
    let text = std::fs::read_to_string(&host).unwrap();
    for call in QUEUE_CALLS {
        assert_eq!(
            text.matches(call).count(),
            1,
            "{call} appears once in the host, in its one path"
        );
    }
}
