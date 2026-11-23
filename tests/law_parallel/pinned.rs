pub const LONG: u64 = 600_000;

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

#[derive(Clone, Debug)]
pub struct Counted {
    pub cube: bool,
    pub outer: bool,
    pub places: Vec<u32>,
    pub born: u32,
    pub kept: u32,
}

impl Counted {
    pub fn stencil(&self) -> &'static str {
        if self.cube { "3x3x3" } else { "3x3" }
    }

    pub const fn centre(&self) -> u32 {
        if self.cube { 13 } else { 4 }
    }
}

pub fn offset(place: u32) -> [i64; 3] {
    [
        i64::from(place % 3) - 1,
        i64::from(place / 3 % 3) - 1,
        i64::from(place / 9) - 1,
    ]
}

pub fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

pub fn cells(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

pub fn neighbour(size: [u32; 3], at: [u32; 3], step: [i64; 3], wrapped: bool) -> Option<usize> {
    let mut spot = [0_u32; 3];
    for axis in 0..3 {
        let moved = i64::from(at[axis]) + step[axis];
        let side = i64::from(size[axis]);
        if !wrapped && !(0..side).contains(&moved) {
            return None;
        }
        spot[axis] = u32::try_from(moved.rem_euclid(side)).unwrap();
    }
    Some(index(size, spot))
}

pub fn counted_step(rule: &Counted, size: [u32; 3], words: &[u32], wrapped: bool) -> Vec<u32> {
    let mut out = vec![0_u32; words.len()];
    let centre = rule.centre();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let at = [x, y, z];
                let mut lit = 0_u32;
                for &place in &rule.places {
                    if rule.outer && place == centre {
                        continue;
                    }
                    let mut step = offset(place);
                    if !rule.cube {
                        step[2] = 0;
                    }
                    if let Some(spot) = neighbour(size, at, step, wrapped) {
                        lit += words[spot] & 1;
                    }
                }
                let here = index(size, at);
                let table = if rule.outer && words[here] & 1 == 1 {
                    rule.kept
                } else {
                    rule.born
                };
                out[here] = (table >> lit) & 1;
            }
        }
    }
    out
}

pub fn soup(rng: &mut Rng, count: usize, permille: u64) -> Vec<u32> {
    (0..count)
        .map(|_| u32::from(rng.below(1000) < permille))
        .collect()
}

pub fn listed(held: &[u32]) -> String {
    let shown: Vec<String> = held.iter().map(u32::to_string).collect();
    format!("[{}]", shown.join(","))
}

pub fn inline(size: [u32; 3], input: &[u32], output: &[u32]) -> String {
    format!(
        "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
        size[0],
        size[1],
        size[2],
        listed(input),
        listed(output)
    )
}

pub fn examples(
    rule: &Counted,
    size: [u32; 3],
    densities: &[u64],
    seed: u64,
    wrapped: bool,
    (steps, flips): (u32, u64),
) -> String {
    let mut rng = Rng(seed);
    let shown: Vec<String> = densities
        .iter()
        .map(|&permille| {
            let mut input = soup(&mut rng, cells(size), permille);
            let mut output = input.clone();
            for _ in 0..steps {
                output = counted_step(rule, size, &output, wrapped);
            }
            for word in input.iter_mut().chain(output.iter_mut()) {
                if rng.below(1000) < flips {
                    *word ^= 1;
                }
            }
            inline(size, &input, &output)
        })
        .collect();
    format!("[{}]", shown.join(","))
}

pub fn request(rule: &Counted, border: &str, examples: &str, more: &str) -> String {
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{}\",\"examples\":{examples},\"budget_ms\":{LONG}{more}}}",
        rule.stencil(),
        if rule.outer { "outer" } else { "totalistic" }
    )
}

pub fn moore(cube: bool) -> Vec<u32> {
    (0..if cube { 27 } else { 9 }).collect()
}

pub fn life() -> Counted {
    Counted {
        cube: false,
        outer: true,
        places: moore(false),
        born: 1 << 3,
        kept: (1 << 2) | (1 << 3),
    }
}

pub fn bays() -> Counted {
    Counted {
        cube: true,
        outer: true,
        places: moore(true),
        born: 1 << 6,
        kept: (1 << 5) | (1 << 6) | (1 << 7),
    }
}

pub fn majority() -> Counted {
    Counted {
        cube: false,
        outer: false,
        places: moore(false),
        born: 0b11_1110_0000,
        kept: 0,
    }
}

pub fn cube_faces() -> Counted {
    Counted {
        cube: true,
        outer: false,
        places: vec![4, 10, 12, 13, 14, 16, 22],
        born: 0b1111_0000,
        kept: 0,
    }
}

pub fn requests() -> Vec<(&'static str, String)> {
    let square = [32, 32, 1];
    let cube = [16, 16, 16];
    vec![
        (
            "outer-life",
            request(
                &life(),
                "wrapped",
                &examples(&life(), square, &[250, 350, 450, 600], 0xB1, true, (1, 0)),
                "",
            ),
        ),
        (
            "totalistic-majority-zero",
            request(
                &majority(),
                "zero",
                &examples(&majority(), square, &[300, 500, 700], 0xB2, false, (1, 0)),
                "",
            ),
        ),
        (
            "totalistic-faces-3d",
            request(
                &cube_faces(),
                "wrapped",
                &examples(&cube_faces(), cube, &[300, 500], 0xB3, true, (1, 0)),
                ",\"places\":\"von_neumann\"",
            ),
        ),
        (
            "outer-life-steps-4",
            request(
                &life(),
                "wrapped",
                &examples(
                    &life(),
                    square,
                    &[350, 350, 350, 350, 350, 350, 350, 350],
                    0xB4,
                    true,
                    (4, 0),
                ),
                ",\"steps\":4",
            ),
        ),
        (
            "outer-life-vote",
            request(
                &life(),
                "wrapped",
                &examples(&life(), square, &[300, 400, 500, 600], 0xB5, true, (1, 20)),
                ",\"noise\":\"vote\",\"min_support\":3",
            ),
        ),
    ]
}
