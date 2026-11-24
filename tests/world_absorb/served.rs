#![allow(dead_code)]

use std::path::{Path, PathBuf};

use grit::serve3::Server;
use grit::wire::{Json, written};

use super::pinned::{Counted, LONG, examples, request};

pub fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

pub fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    let shown = written(got);
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &shown[..shown.len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

pub fn ok(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), false)
}

pub fn no(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), true)
}

pub fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

pub fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

pub fn quoted(text: &str) -> String {
    written(&Json::Text(text.to_owned()))
}

pub fn hex(sum: &[u8; 32]) -> String {
    sum.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn words_hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    hex(&grit::branch::sha256(&bytes))
}

pub fn learned(server: &mut Server<'_>, rule: &Counted, parallel: bool, tables: &str) -> String {
    let (size, densities): ([u32; 3], &[u64]) = if rule.cube {
        ([16, 16, 16], &[120, 180, 220, 260, 320, 400, 930])
    } else {
        ([32, 32, 1], &[250, 300, 350, 400, 450, 500, 600, 700, 930])
    };
    let seed = u64::from(rule.born ^ rule.kept.rotate_left(7));
    let shown = examples(rule, size, densities, seed, true, (1, 0));
    let more = if parallel { ",\"parallel\":true" } else { "" };
    let got = ok(
        server,
        "grit_law_learn",
        &request(rule, "wrapped", &shown, more),
    );
    assert_eq!(written(got.get("tables").unwrap()), tables);
    text_at(&got, "law")
}

pub const LIFE_TABLES: &str = "[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]";

pub const BAYS_TABLES: &str = "[[[0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],[0,0,0,0,0,1,1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]]]";

pub fn cells_text(cells: &[([u32; 3], u32)]) -> String {
    let listed: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", listed.join(","))
}

pub fn write(server: &mut Server<'_>, world: i64, plane: u32, cells: &[([u32; 3], u32)]) -> Json {
    let mut done = Json::Null;
    for piece in cells.chunks(4096) {
        done = ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":{},\"budget_ms\":{LONG}}}",
                cells_text(piece)
            ),
        );
    }
    done
}

pub fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
    ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    )
}

pub fn plane_words(server: &mut Server<'_>, world: i64, size: [u32; 3], plane: u32) -> Vec<u32> {
    let per = (4096 / (size[0] * size[1])).max(1);
    let mut words = Vec::new();
    let mut z = 0;
    while z < size[2] {
        let top = (z + per).min(size[2]) - 1;
        let got = ok(
            server,
            "grit_world_read",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"from\":[0,0,{z}],\"to\":[{},{},{top}],\"budget_ms\":{LONG}}}",
                size[0] - 1,
                size[1] - 1
            ),
        );
        for cell in got.get("cells").and_then(Json::list).unwrap() {
            words.push(u32::try_from(cell.int().unwrap()).unwrap());
        }
        z = top + 1;
    }
    words
}

pub fn every_plane(server: &mut Server<'_>, world: i64, size: [u32; 3], planes: u32) -> Vec<u32> {
    let mut words = Vec::new();
    for plane in 0..planes {
        words.extend(plane_words(server, world, size, plane));
    }
    words
}

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "grit-border-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn dir(&self) -> PathBuf {
        self.0.join("worlds")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn files(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, into);
        } else if path.file_name().and_then(|name| name.to_str()) != Some("lock") {
            into.push(path);
        }
    }
}

pub fn journal_hash(dir: &Path) -> String {
    let mut names = Vec::new();
    files(dir, &mut names);
    names.sort();
    let mut joined = Vec::new();
    for path in names {
        let name = path
            .strip_prefix(dir)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        joined.extend_from_slice(name.as_bytes());
        joined.push(0);
        joined.extend_from_slice(&std::fs::read(&path).unwrap());
        joined.push(0);
    }
    hex(&grit::branch::sha256(&joined))
}

pub const STEADY: [&str; 10] = [
    "world", "from", "name", "written", "frame", "frames", "tally", "current", "plane", "absorb",
];

pub fn steady(answer: &Json, sparse: bool) -> String {
    match answer {
        Json::Map(pairs) if sparse => written(&Json::Map(
            pairs
                .iter()
                .filter(|(key, _)| STEADY.contains(&key.as_str()))
                .cloned()
                .collect(),
        )),
        other => written(other),
    }
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, most: u64) -> u64 {
        self.next() % most
    }
}

pub fn soup_cells(
    seed: u64,
    from: [u32; 3],
    sides: [u32; 3],
    permille: u64,
    word: impl Fn(&mut Rng) -> u32,
) -> Vec<([u32; 3], u32)> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    for z in 0..sides[2] {
        for y in 0..sides[1] {
            for x in 0..sides[0] {
                if rng.below(1000) < permille {
                    let held = word(&mut rng);
                    if held != 0 {
                        out.push(([from[0] + x, from[1] + y, from[2] + z], held));
                    }
                }
            }
        }
    }
    out
}

pub fn glider_2d(at: [u32; 3], flip: [bool; 2]) -> Vec<[u32; 3]> {
    let flat = [[1_u32, 0], [2, 1], [0, 2], [1, 2], [2, 2]];
    flat.iter()
        .map(|&[a, b]| {
            let a = if flip[0] { 2 - a } else { a };
            let b = if flip[1] { 2 - b } else { b };
            [at[0] + a, at[1] + b, at[2]]
        })
        .collect()
}

pub fn glider_3d(at: [u32; 3], flip: [bool; 2]) -> Vec<[u32; 3]> {
    let mut out = Vec::new();
    for cell in glider_2d([at[0], at[1], at[2]], flip) {
        out.push(cell);
        out.push([cell[0], cell[1], cell[2] + 1]);
    }
    out
}

pub fn set_total(words: &[u32]) -> usize {
    words.iter().filter(|word| **word != 0).count()
}

pub fn glider_along(at: [u32; 3], axes: [usize; 3], flip: [bool; 2]) -> Vec<[u32; 3]> {
    let flat = [[1_u32, 0], [2, 1], [0, 2], [1, 2], [2, 2]];
    let mut out = Vec::new();
    for [a, b] in flat {
        let a = if flip[0] { 2 - a } else { a };
        let b = if flip[1] { 2 - b } else { b };
        for c in [0, 1] {
            let mut cell = at;
            cell[axes[0]] += a;
            cell[axes[1]] += b;
            cell[axes[2]] += c;
            out.push(cell);
        }
    }
    out
}
