#![allow(dead_code)]

use grit::serve3::Server;
use grit::wire::{Json, written};

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

pub fn code_of(refusal: &Json) -> (&str, &str) {
    (
        refusal.get("refused").and_then(Json::text).unwrap_or(""),
        refusal.get("field").and_then(Json::text).unwrap_or(""),
    )
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

pub fn bytes_hash(bytes: &[u8]) -> String {
    hex(&grit::branch::sha256(bytes))
}

pub fn words_hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    bytes_hash(&bytes)
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

    pub fn word(&mut self) -> u32 {
        self.next() as u32
    }
}

pub fn cells_text(cells: &[([u32; 3], u32)]) -> String {
    let shown: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", shown.join(","))
}

pub fn listed(held: &[u32]) -> String {
    let shown: Vec<String> = held.iter().map(u32::to_string).collect();
    format!("[{}]", shown.join(","))
}

const SIXES: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encoded(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at < bytes.len() {
        let left = bytes.len() - at;
        let a = bytes[at];
        let b = if left > 1 { bytes[at + 1] } else { 0 };
        let c = if left > 2 { bytes[at + 2] } else { 0 };
        out.push(SIXES[(a >> 2) as usize] as char);
        out.push(SIXES[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if left > 1 {
            SIXES[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if left > 2 {
            SIXES[(c & 63) as usize] as char
        } else {
            '='
        });
        at += 3;
    }
    out
}

pub fn decoded(text: &str) -> Vec<u8> {
    let value = |byte: u8| -> u32 {
        SIXES
            .iter()
            .position(|one| *one == byte)
            .unwrap_or_else(|| panic!("{byte} is not base64")) as u32
    };
    let bytes = text.as_bytes();
    assert_eq!(
        bytes.len() % 4,
        0,
        "base64 text of {} characters",
        bytes.len()
    );
    let mut out = Vec::new();
    for group in bytes.chunks(4) {
        let pads = group.iter().filter(|one| **one == b'=').count();
        let mut joined = 0_u32;
        for byte in group {
            joined = (joined << 6) | if *byte == b'=' { 0 } else { value(*byte) };
        }
        let three = [(joined >> 16) as u8, (joined >> 8) as u8, joined as u8];
        out.extend_from_slice(&three[..3 - pads]);
    }
    out
}

pub fn packed(format: &str, words: &[u32]) -> Vec<u8> {
    let held: Vec<u32> = if format == "bits" {
        bits_of(words)
    } else {
        words.to_vec()
    };
    held.iter().flat_map(|word| word.to_le_bytes()).collect()
}

pub fn bits_of(words: &[u32]) -> Vec<u32> {
    let mut held = vec![0_u32; words.len().div_ceil(32)];
    for (at, word) in words.iter().enumerate() {
        assert!(*word <= 1, "cell {at} holds {word}, not a bit");
        held[at / 32] |= word << (at % 32);
    }
    held
}

pub fn unpacked(format: &str, bytes: &[u8], cells: usize) -> Vec<u32> {
    let words: Vec<u32> = bytes
        .chunks(4)
        .map(|four| u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
        .collect();
    if format == "bits" {
        assert_eq!(
            words.len(),
            cells.div_ceil(32),
            "a bits page of {cells} cells"
        );
        (0..cells)
            .map(|at| (words[at / 32] >> (at % 32)) & 1)
            .collect()
    } else {
        assert_eq!(words.len(), cells, "a words page of {cells} cells");
        words
    }
}

pub fn box_count(from: [u32; 3], to: [u32; 3]) -> usize {
    (0..3)
        .map(|axis| (to[axis] - from[axis] + 1) as usize)
        .product()
}

pub fn read_page(
    server: &mut Server<'_>,
    world: i64,
    plane: u32,
    (from, to): ([u32; 3], [u32; 3]),
    format: &str,
) -> Vec<u32> {
    let got = ok(
        server,
        "grit_world_read",
        &format!(
            "{{\"world\":{world},\"plane\":{plane},\"form\":\"page\",\"from\":[{},{},{}],\"to\":[{},{},{}],\"format\":\"{format}\"}}",
            from[0], from[1], from[2], to[0], to[1], to[2]
        ),
    );
    let cells = box_count(from, to);
    assert_eq!(int_at(&got, "cells"), cells as i64);
    let words = unpacked(format, &decoded(&text_at(&got, "data")), cells);
    let count = if format == "bits" {
        cells.div_ceil(32)
    } else {
        cells
    };
    assert_eq!(int_at(&got, "words"), count as i64);
    words
}

pub fn dense_box(
    server: &mut Server<'_>,
    world: i64,
    plane: u32,
    from: [u32; 3],
    to: [u32; 3],
) -> Vec<u32> {
    let wide = to[0] - from[0] + 1;
    let rows = (4096 / wide).max(1);
    let mut words = Vec::new();
    for z in from[2]..=to[2] {
        let mut y = from[1];
        while y <= to[1] {
            let top = (y + rows - 1).min(to[1]);
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"from\":[{},{y},{z}],\"to\":[{},{top},{z}]}}",
                    from[0], to[0]
                ),
            );
            for cell in got.get("cells").and_then(Json::list).unwrap() {
                words.push(u32::try_from(cell.int().unwrap()).unwrap());
            }
            y = top + 1;
        }
    }
    words
}
