#![allow(dead_code)]

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

pub fn count(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

pub fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

pub fn point(size: [u32; 3], at: usize) -> [u32; 3] {
    let wide = size[0] as usize;
    let deep = size[1] as usize;
    [
        (at % wide) as u32,
        (at / wide % deep) as u32,
        (at / (wide * deep)) as u32,
    ]
}

pub fn listed(held: &[u32]) -> String {
    let shown: Vec<String> = held.iter().map(u32::to_string).collect();
    format!("[{}]", shown.join(","))
}

pub fn cells_text(cells: &[([u32; 3], u32)]) -> String {
    let shown: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    format!("[{}]", shown.join(","))
}

pub fn set_of(size: [u32; 3], words: &[u32]) -> Vec<([u32; 3], u32)> {
    words
        .iter()
        .enumerate()
        .filter(|(_, word)| **word != 0)
        .map(|(at, word)| (point(size, at), *word))
        .collect()
}

pub fn learned(server: &mut Server<'_>, rule: &Counted, parallel: bool) -> String {
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
    text_at(&got, "law")
}

pub const FRAC: u32 = 16;

pub fn fixed(value: f64, frac: u32) -> u32 {
    ((value * f64::from(1_u32 << frac)).round() as i64 as i32).cast_unsigned()
}

pub fn heat_step(
    size: [u32; 3],
    words: &[u32],
    weights: &[([i64; 3], f64)],
    frac: u32,
) -> Vec<u32> {
    let scale = f64::from(1_u32 << frac);
    (0..count(size))
        .map(|at| {
            let here = point(size, at);
            let mut sum = 0.0;
            for (by, weight) in weights {
                let mut spot = [0_u32; 3];
                for axis in 0..3 {
                    spot[axis] =
                        (i64::from(here[axis]) + by[axis]).rem_euclid(i64::from(size[axis])) as u32;
                }
                sum += weight * f64::from(words[index(size, spot)].cast_signed()) / scale;
            }
            fixed(sum, frac)
        })
        .collect()
}

pub const HEAT: [([i64; 3], f64); 5] = [
    ([0, 0, 0], 0.4),
    ([1, 0, 0], 0.15),
    ([-1, 0, 0], 0.15),
    ([0, 1, 0], 0.15),
    ([0, -1, 0], 0.15),
];

pub fn heat_field(rng: &mut Rng, size: [u32; 3], frac: u32) -> Vec<u32> {
    (0..count(size))
        .map(|_| fixed(rng.below(1 << 20) as f64 / f64::from(1_u32 << 20), frac))
        .collect()
}

pub fn heat_law(server: &mut Server<'_>) -> String {
    let size = [16, 16, 1];
    let mut rng = Rng(77);
    let shown: Vec<String> = (0..4)
        .map(|_| {
            let input = heat_field(&mut rng, size, FRAC);
            let output = heat_step(size, &input, &HEAT, FRAC);
            format!(
                "{{\"size\":[16,16,1],\"input\":[{}],\"output\":[{}]}}",
                listed(&input),
                listed(&output)
            )
        })
        .collect();
    let got = ok(
        server,
        "grit_law_learn",
        &format!(
            "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":{FRAC},\"examples\":[{}],\"budget_ms\":{LONG}}}",
            shown.join(",")
        ),
    );
    text_at(&got, "law")
}

pub fn write(server: &mut Server<'_>, world: i64, plane: u32, cells: &[([u32; 3], u32)]) {
    for piece in cells.chunks(4096) {
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":{},\"budget_ms\":{LONG}}}",
                cells_text(piece)
            ),
        );
    }
}

pub fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
    ok(
        server,
        "grit_world_step",
        &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":{LONG}}}"),
    )
}

pub fn box_words(
    server: &mut Server<'_>,
    world: i64,
    plane: u32,
    from: [u32; 3],
    sides: [u32; 3],
) -> Vec<u32> {
    let mut words = vec![0_u32; count(sides)];
    let rows = (4096 / sides[0]).max(1);
    for z in 0..sides[2] {
        let mut y = 0;
        while y < sides[1] {
            let top = (y + rows).min(sides[1]) - 1;
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"from\":[{},{},{}],\"to\":[{},{},{}],\"budget_ms\":{LONG}}}",
                    from[0],
                    from[1] + y,
                    from[2] + z,
                    from[0] + sides[0] - 1,
                    from[1] + top,
                    from[2] + z
                ),
            );
            let listed = got.get("cells").and_then(Json::list).unwrap();
            for (at, cell) in listed.iter().enumerate() {
                let x = at as u32 % sides[0];
                let row = y + at as u32 / sides[0];
                words[index(sides, [x, row, z])] = u32::try_from(cell.int().unwrap()).unwrap();
            }
            y = top + 1;
        }
    }
    words
}
