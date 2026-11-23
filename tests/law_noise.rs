#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::serve3::Server;
use grit::stencil::{self, Stencil, Tally};
use grit::wire::{Json, written};

const LONG: u64 = 600_000;

const CHUNK: usize = 4096;

const TRAIN: u64 = 20_261_123;

const LIFE_TABLES: &str = "[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]";

const NOISY_SHA: &str = "39a5dd5fa3cede2f43774c722083294eb4ed08ff8cd2106583fdc00037d3b617";

const CLEAN_SHA: &str = "4bff623e041b7aab388f61b89319bcb585499c8f54b5a5e238da53bc2a8e73c2";

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, most: u64) -> u32 {
        u32::try_from(self.next() % most).unwrap()
    }
}

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        &written(got)[..written(got).len().min(4000)]
    );
    held.get("structuredContent").cloned().unwrap()
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn words(held: &Json) -> Vec<u32> {
    held.list()
        .unwrap()
        .iter()
        .map(|one| u32::try_from(one.int().unwrap()).unwrap())
        .collect()
}

fn listed(held: &[u32]) -> String {
    format!(
        "[{}]",
        held.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn shown_of(held: &Json, key: &str) -> String {
    written(
        held.get(key)
            .unwrap_or_else(|| panic!("no {key} in {}", written(held))),
    )
}

struct Mcp<'host> {
    server: Server<'host>,
}

impl<'host> Mcp<'host> {
    fn new(host: Option<&'host Host>) -> Self {
        Self {
            server: Server::new(host),
        }
    }

    fn raw(&mut self, name: &str, arguments: &str) -> Json {
        self.server.heard(&line(name, arguments)).unwrap()
    }

    fn ok(&mut self, name: &str, arguments: &str) -> Json {
        result(&self.raw(name, arguments), false)
    }

    fn no(&mut self, name: &str, arguments: &str) -> Json {
        result(&self.raw(name, arguments), true)
    }

    fn world(&mut self, law: &str, size: [u32; 3]) -> u32 {
        let got = self.ok(
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{}}}",
                size[0],
                size[1],
                size[2],
                written(&Json::Text(law.to_owned()))
            ),
        );
        u32::try_from(int_at(&got, "world")).unwrap()
    }

    fn write(&mut self, world: u32, cells: &[([u32; 3], u32)]) {
        for piece in cells.chunks(CHUNK) {
            let shown: Vec<String> = piece
                .iter()
                .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
                .collect();
            let got = self.ok(
                "grit_world_write",
                &format!(
                    "{{\"world\":{world},\"plane\":0,\"cells\":[{}],\"budget_ms\":{LONG}}}",
                    shown.join(",")
                ),
            );
            assert_eq!(int_at(&got, "written"), piece.len() as i64);
        }
    }

    fn read(&mut self, world: u32, size: [u32; 3]) -> Vec<u32> {
        let mut out = Vec::new();
        let rows = (CHUNK as u32 / size[0]).max(1);
        for z in 0..size[2] {
            let mut y = 0;
            while y < size[1] {
                let last = (y + rows).min(size[1]) - 1;
                let got = self.ok(
                    "grit_world_read",
                    &format!(
                        "{{\"world\":{world},\"plane\":1,\"from\":[0,{y},{z}],\"to\":[{},{last},{z}]}}",
                        size[0] - 1
                    ),
                );
                out.extend(words(got.get("cells").unwrap()));
                y = last + 1;
            }
        }
        out
    }

    fn ran(&mut self, law: &str, size: [u32; 3], input: &[u32]) -> Vec<u32> {
        let world = self.world(law, size);
        let mut cells = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let word = input[stencil::index(size, [x, y, z])];
                    if word != 0 {
                        cells.push(([x, y, z], word));
                    }
                }
            }
        }
        self.write(world, &cells);
        self.ok(
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":1,\"budget_ms\":{LONG}}}"),
        );
        let got = self.read(world, size);
        self.ok("grit_world_close", &format!("{{\"world\":{world}}}"));
        got
    }
}

fn random_bits(rng: &mut Rng, cells: usize, permille: u64) -> Vec<u32> {
    (0..cells)
        .map(|_| u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn flipped(rng: &mut Rng, held: &[u32], permille: u64) -> Vec<u32> {
    held.iter()
        .map(|one| one ^ u32::from(u64::from(rng.below(1000)) < permille))
        .collect()
}

fn own_rows(stencil: Stencil, size: [u32; 3], input: &[u32], wrapped: bool) -> Vec<u32> {
    let mut rows = Vec::new();
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let mut row = 0_u32;
                for place in 0..stencil.positions() {
                    let p = i64::from(place);
                    let dz = if stencil == Stencil::Cube {
                        p / 9 - 1
                    } else {
                        0
                    };
                    let at = [
                        i64::from(x) + p % 3 - 1,
                        i64::from(y) + p / 3 % 3 - 1,
                        i64::from(z) + dz,
                    ];
                    let mut spot = [0_u32; 3];
                    let mut inside = true;
                    for axis in 0..3 {
                        let side = i64::from(size[axis]);
                        let value = if wrapped {
                            at[axis].rem_euclid(side)
                        } else {
                            at[axis]
                        };
                        if (0..side).contains(&value) {
                            spot[axis] = u32::try_from(value).unwrap();
                        } else {
                            inside = false;
                        }
                    }
                    if inside && input[stencil::index(size, spot)] & 1 == 1 {
                        row |= 1 << place;
                    }
                }
                rows.push(row);
            }
        }
    }
    rows
}

fn own_centre(stencil: Stencil) -> u32 {
    match stencil {
        Stencil::Square => 4,
        Stencil::Cube => 13,
    }
}

fn own_counts(tally: Tally, stencil: Stencil) -> u32 {
    match tally {
        Tally::Totalistic => stencil.positions() + 1,
        Tally::Outer => stencil.positions(),
    }
}

fn own_class(tally: Tally, stencil: Stencil, row: u32) -> u32 {
    let all = row.count_ones();
    match tally {
        Tally::Totalistic => all,
        Tally::Outer => {
            let centre = (row >> own_centre(stencil)) & 1;
            centre * own_counts(tally, stencil) + all - centre
        }
    }
}

fn own_classes(tally: Tally, stencil: Stencil) -> u32 {
    match tally {
        Tally::Totalistic => own_counts(tally, stencil),
        Tally::Outer => 2 * own_counts(tally, stencil),
    }
}

fn life(row: u32) -> u32 {
    let centre = (row >> 4) & 1;
    let around = (row & 0x1FF).count_ones() - centre;
    u32::from(around == 3 || (centre == 1 && around == 2))
}

fn cube_life(row: u32) -> u32 {
    let centre = (row >> 13) & 1;
    let around = row.count_ones() - centre;
    u32::from(if centre == 1 {
        (5..=7).contains(&around)
    } else {
        around == 6
    })
}

fn stepped(stencil: Stencil, size: [u32; 3], held: &[u32], truth: fn(u32) -> u32) -> Vec<u32> {
    own_rows(stencil, size, held, true)
        .into_iter()
        .map(truth)
        .collect()
}

struct Example {
    size: [u32; 3],
    input: Vec<u32>,
    output: Vec<u32>,
}

fn shown(examples: &[Example]) -> String {
    let shown: Vec<String> = examples
        .iter()
        .map(|one| {
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                one.size[0],
                one.size[1],
                one.size[2],
                listed(&one.input),
                listed(&one.output)
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn args(stencil: Stencil, rule: &str, border: &str, examples: &[Example], noise: &str) -> String {
    format!(
        "{{\"stencil\":\"{}\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{}{noise},\"budget_ms\":{LONG}}}",
        stencil.name(),
        shown(examples)
    )
}

const VOTE: &str = ",\"noise\":\"vote\"";

const REFUSE: &str = ",\"noise\":\"refuse\"";

fn torus_pairs(
    stencil: Stencil,
    truth: fn(u32) -> u32,
    seed: u64,
    count: usize,
    size: [u32; 3],
    permille: u64,
) -> Vec<Example> {
    let mut rng = Rng(seed);
    let cells = size.iter().product::<u32>() as usize;
    (0..count)
        .map(|at| {
            let mut input = random_bits(&mut rng, cells, permille);
            if at >= count / 2 {
                for _ in 0..10 {
                    input = stepped(stencil, size, &input, truth);
                }
            }
            let output = stepped(stencil, size, &input, truth);
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flip {
    Input,
    Output,
    Both,
}

impl Flip {
    const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Both => "both",
        }
    }
}

fn noisy(clean: &[Example], seed: u64, permille: u64, flip: Flip) -> (Vec<Example>, u64) {
    let mut rng = Rng(seed);
    let mut flips = 0_u64;
    let examples = clean
        .iter()
        .map(|one| {
            let input = if flip == Flip::Output {
                one.input.clone()
            } else {
                flipped(&mut rng, &one.input, permille)
            };
            let output = if flip == Flip::Input {
                one.output.clone()
            } else {
                flipped(&mut rng, &one.output, permille)
            };
            flips += input.iter().zip(&one.input).filter(|(a, b)| a != b).count() as u64;
            flips += output
                .iter()
                .zip(&one.output)
                .filter(|(a, b)| a != b)
                .count() as u64;
            Example {
                size: one.size,
                input,
                output,
            }
        })
        .collect();
    (examples, flips)
}

struct Host1 {
    tallies: Vec<[u64; 2]>,
    deviating: u64,
    cells: Vec<String>,
    places: Vec<(u32, [u32; 3])>,
}

fn host_vote(
    tally: Tally,
    stencil: Stencil,
    examples: &[Example],
    wrapped: bool,
    law: &dyn Fn(u32) -> u32,
) -> Host1 {
    let mut tallies = vec![[0_u64; 2]; own_classes(tally, stencil) as usize];
    let mut deviating = 0;
    let mut cells = Vec::new();
    let mut places = Vec::new();
    let counts = own_counts(tally, stencil);
    for (example, one) in examples.iter().enumerate() {
        let rows = own_rows(stencil, one.size, &one.input, wrapped);
        for z in 0..one.size[2] {
            for y in 0..one.size[1] {
                for x in 0..one.size[0] {
                    let index = stencil::index(one.size, [x, y, z]);
                    let row = rows[index];
                    let class = own_class(tally, stencil, row);
                    let seen = one.output[index] & 1;
                    tallies[class as usize][seen as usize] += 1;
                    let wanted = law(row);
                    if seen != wanted {
                        deviating += 1;
                        places.push((u32::try_from(example).unwrap(), [x, y, z]));
                        if cells.len() < 64 {
                            let centre = match tally {
                                Tally::Totalistic => String::new(),
                                Tally::Outer => format!(",\"centre\":{}", class / counts),
                            };
                            cells.push(format!(
                                "{{\"example\":{example},\"at\":[{x},{y},{z}],\"count\":{}{centre},\"seen\":{seen},\"law\":{wanted}}}",
                                class % counts
                            ));
                        }
                    }
                }
            }
        }
    }
    Host1 {
        tallies,
        deviating,
        cells,
        places,
    }
}

fn grouped(tally: Tally, stencil: Stencil, each: &dyn Fn(u32) -> Option<String>) -> String {
    let counts = own_counts(tally, stencil);
    let row = |from: u32| -> String {
        let held: Vec<String> = (from..from + counts).filter_map(each).collect();
        format!("[{}]", held.join(","))
    };
    match tally {
        Tally::Totalistic => row(0),
        Tally::Outer => format!("[{},{}]", row(0), row(counts)),
    }
}

fn vote_checked(got: &Json, tally: Tally, stencil: Stencil, host: &Host1, total: u64) {
    let tallies = grouped(tally, stencil, &|class| {
        let [zeros, ones] = host.tallies[class as usize];
        Some(format!("[{zeros},{ones}]"))
    });
    assert_eq!(shown_of(got, "tallies"), format!("[{tallies}]"), "tallies");
    let summed: u64 = host.tallies.iter().map(|pair| pair[0] + pair[1]).sum();
    assert_eq!(summed, total, "the tallies hold every voted cell");
    let ties = grouped(tally, stencil, &|class| {
        let [zeros, ones] = host.tallies[class as usize];
        (zeros == ones && zeros > 0).then(|| (class % own_counts(tally, stencil)).to_string())
    });
    assert_eq!(shown_of(got, "ties"), format!("[{ties}]"), "ties");
    assert_eq!(shown_of(got, "deviating"), format!("[{}]", host.deviating));
    assert_eq!(
        shown_of(got, "deviating_cells"),
        format!("[[{}]]", host.cells.join(","))
    );
    assert_eq!(
        shown_of(got, "deviating_share"),
        format!("[{},{total}]", host.deviating)
    );
}

fn device_life(mcp: &mut Mcp<'_>, law: &str, seed: u64) {
    let size = [64, 64, 2];
    let mut rng = Rng(seed);
    let mut image = random_bits(&mut rng, 64 * 64, 350);
    image.extend(vec![0_u32; 64 * 64]);
    let ran = mcp.ran(law, size, &image);
    let rows = own_rows(Stencil::Square, size, &image, true);
    for (at, row) in rows.iter().enumerate().take(64 * 64) {
        assert_eq!(ran[at] & 1, life(*row), "world {seed} cell {at}");
    }
}

fn life_pairs() -> Vec<Example> {
    torus_pairs(Stencil::Square, life, TRAIN, 16, [32, 32, 1], 350)
}

#[test]
fn n1_life_under_noise_votes_the_exact_table_and_runs_as_the_host() {
    let Some(host) = device("law_noise n1") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let clean = life_pairs();
    let plain = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &clean, ""),
    );
    assert_eq!(shown_of(&plain, "tables"), LIFE_TABLES);
    let law = text_at(&plain, "law");
    for (ri, permille) in [1_u64, 10, 50].into_iter().enumerate() {
        for (wi, flip) in [Flip::Input, Flip::Output, Flip::Both]
            .into_iter()
            .enumerate()
        {
            let seed = TRAIN + 100 * ri as u64 + wi as u64;
            let (examples, flips) = noisy(&clean, seed, permille, flip);
            let got = mcp.ok(
                "grit_law_learn",
                &args(Stencil::Square, "outer", "wrapped", &examples, VOTE),
            );
            let case = format!("{permille} permille in {}", flip.name());
            assert_eq!(shown_of(&got, "tables"), LIFE_TABLES, "{case}");
            assert_eq!(text_at(&got, "law"), law, "{case}");
            assert_eq!(shown_of(&got, "chip"), shown_of(&plain, "chip"), "{case}");
            let own = host_vote(Tally::Outer, Stencil::Square, &examples, true, &life);
            vote_checked(&got, Tally::Outer, Stencil::Square, &own, 16 * 1024);
            println!(
                "n1 {case}: {flips} flipped cells, {} deviating of 16384",
                own.deviating
            );
            device_life(&mut mcp, &law, seed + 7);
        }
    }
}

fn sha_of(held: &Json) -> String {
    grit::law::hex(&grit::branch::sha256(written(held).as_bytes()))
}

fn lt1_clean_life() -> Vec<Example> {
    let mut rng = Rng(202);
    [250, 300, 350, 400, 450, 500, 600, 700]
        .into_iter()
        .map(|permille| {
            let input = random_bits(&mut rng, 1024, permille);
            let output = stepped(Stencil::Square, [32, 32, 1], &input, life);
            Example {
                size: [32, 32, 1],
                input,
                output,
            }
        })
        .collect()
}

#[test]
fn n2_refuse_and_no_noise_answer_as_the_base_did() {
    let mut mcp = Mcp::new(None);
    let (examples, _) = noisy(&life_pairs(), TRAIN + 101, 10, Flip::Output);
    let bare = mcp.raw(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &examples, ""),
    );
    let told = result(&bare, true);
    assert_eq!(text_at(&told, "refused"), "conflict");
    assert_eq!(text_at(&told, "field"), "$.examples");
    let clean = mcp.raw(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &lt1_clean_life(), ""),
    );
    assert_eq!(shown_of(&result(&clean, false), "tables"), LIFE_TABLES);
    println!("n2 noisy {} clean {}", sha_of(&bare), sha_of(&clean));
    assert_eq!(sha_of(&bare), NOISY_SHA, "the noisy refusal as on the base");
    assert_eq!(sha_of(&clean), CLEAN_SHA, "the clean answer as on the base");
    let refused = mcp.raw(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &examples, REFUSE),
    );
    assert_eq!(written(&refused), written(&bare));
    let answered = mcp.raw(
        "grit_law_learn",
        &args(
            Stencil::Square,
            "outer",
            "wrapped",
            &lt1_clean_life(),
            REFUSE,
        ),
    );
    assert_eq!(written(&answered), written(&clean));
}

#[test]
fn n3_cube_life_under_output_noise_votes_the_exact_table() {
    let mut mcp = Mcp::new(None);
    let size = [16, 16, 16];
    let clean = torus_pairs(Stencil::Cube, cube_life, TRAIN + 300, 8, size, 220);
    let (examples, flips) = noisy(&clean, TRAIN + 302, 50, Flip::Output);
    let got = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Cube, "outer", "wrapped", &examples, VOTE),
    );
    let born: Vec<String> = (0..27).map(|n| u32::from(n == 6).to_string()).collect();
    let kept: Vec<String> = (0..27)
        .map(|n| u32::from((5..=7).contains(&n)).to_string())
        .collect();
    assert_eq!(
        shown_of(&got, "tables"),
        format!("[[[{}],[{}]]]", born.join(","), kept.join(","))
    );
    let own = host_vote(Tally::Outer, Stencil::Cube, &examples, true, &cube_life);
    vote_checked(&got, Tally::Outer, Stencil::Cube, &own, 8 * 4096);
    println!(
        "n3: {flips} flipped cells, {} deviating of 32768",
        own.deviating
    );
}

#[test]
fn n4_a_zero_border_crop_of_a_torus_votes_life_and_deviates_only_on_its_ring() {
    let mut mcp = Mcp::new(None);
    let big = [64, 64, 1];
    let side = 32;
    let worlds = torus_pairs(Stencil::Square, life, TRAIN + 400, 8, big, 350);
    let mut rng = Rng(TRAIN + 401);
    let examples: Vec<Example> = worlds
        .iter()
        .map(|one| {
            let from = [rng.below(32), rng.below(32)];
            let crop = |held: &[u32]| -> Vec<u32> {
                let mut out = Vec::with_capacity(side * side);
                for y in 0..side as u32 {
                    for x in 0..side as u32 {
                        out.push(held[stencil::index(big, [from[0] + x, from[1] + y, 0])]);
                    }
                }
                out
            };
            Example {
                size: [32, 32, 1],
                input: crop(&one.input),
                output: crop(&one.output),
            }
        })
        .collect();
    let got = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "zero", &examples, VOTE),
    );
    assert_eq!(shown_of(&got, "tables"), LIFE_TABLES);
    let own = host_vote(Tally::Outer, Stencil::Square, &examples, false, &life);
    vote_checked(&got, Tally::Outer, Stencil::Square, &own, 8 * 1024);
    assert!(own.deviating > 0, "the ring reads zeros past the crop");
    let ring = |at: [u32; 3]| at[0] == 0 || at[1] == 0 || at[0] == 31 || at[1] == 31;
    for (example, at) in &own.places {
        assert!(ring(*at), "example {example} cell {at:?}");
    }
    let cells = got.get("deviating_cells").and_then(Json::list).unwrap()[0]
        .list()
        .unwrap()
        .to_vec();
    assert_eq!(cells.len() as u64, own.deviating.min(64));
    for one in &cells {
        let at = <[u32; 3]>::try_from(words(one.get("at").unwrap())).unwrap();
        assert!(ring(at), "{}", written(one));
    }
    println!("n4: {} deviating, all on the ring", own.deviating);
}

fn tie_examples() -> Vec<Example> {
    let size = [8, 8, 1];
    let mut input = vec![0_u32; 64];
    input[stencil::index(size, [2, 2, 0])] = 1;
    let rows = own_rows(Stencil::Square, size, &input, false);
    let near: Vec<u32> = rows.iter().map(|row| u32::from(*row != 0)).collect();
    vec![
        Example {
            size,
            input: input.clone(),
            output: vec![0; 64],
        },
        Example {
            size,
            input,
            output: near,
        },
    ]
}

#[test]
fn n5_a_tie_is_decided_zero_and_listed_and_an_unseen_count_is_not_a_tie() {
    let mut mcp = Mcp::new(None);
    let examples = tie_examples();
    for (tally, tables, ties, never, tallies) in [
        (
            Tally::Totalistic,
            "[[0,0,0,0,0,0,0,0,0,0]]",
            "[[1]]",
            "[[2,3,4,5,6,7,8,9]]",
            "[[[110,0],[9,9],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]]]",
        ),
        (
            Tally::Outer,
            "[[[0,0,0,0,0,0,0,0,0],[0,0,0,0,0,0,0,0,0]]]",
            "[[[1],[0]]]",
            "[[[2,3,4,5,6,7,8],[1,2,3,4,5,6,7,8]]]",
            "[[[[110,0],[8,8],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]],[[1,1],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0],[0,0]]]]",
        ),
    ] {
        let told = mcp.no(
            "grit_law_learn",
            &args(Stencil::Square, tally.name(), "zero", &examples, ""),
        );
        assert_eq!(text_at(&told, "refused"), "conflict");
        let got = mcp.ok(
            "grit_law_learn",
            &args(Stencil::Square, tally.name(), "zero", &examples, VOTE),
        );
        assert_eq!(shown_of(&got, "tables"), tables, "{}", tally.name());
        assert_eq!(shown_of(&got, "ties"), ties, "{}", tally.name());
        assert_eq!(shown_of(&got, "unseen_counts"), never, "{}", tally.name());
        assert_eq!(shown_of(&got, "tallies"), tallies, "{}", tally.name());
        assert_eq!(shown_of(&got, "deviating"), "[9]", "{}", tally.name());
        assert_eq!(shown_of(&got, "deviating_share"), "[9,128]");
        let own = host_vote(tally, Stencil::Square, &examples, false, &|_| 0);
        vote_checked(&got, tally, Stencil::Square, &own, 128);
    }
}

#[test]
fn n6_vote_under_a_searching_rule_is_refused_by_name_and_an_unknown_noise_is_malformed() {
    let mut mcp = Mcp::new(None);
    let examples = tie_examples();
    for rule in ["minimal", "symmetric"] {
        let told = mcp.no(
            "grit_law_learn",
            &args(Stencil::Square, rule, "zero", &examples, VOTE),
        );
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ("noise_rule".to_owned(), "$.noise".to_owned()),
            "{}",
            written(&told)
        );
        assert!(
            text_at(&told, "message").contains(rule),
            "{}",
            written(&told)
        );
    }
    for rule in ["minimal", "symmetric", "totalistic", "outer"] {
        for noise in [",\"noise\":\"maybe\"", ",\"noise\":1", ",\"noise\":null"] {
            let told = mcp.no(
                "grit_law_learn",
                &args(Stencil::Square, rule, "zero", &examples, noise),
            );
            assert_eq!(
                (text_at(&told, "refused"), text_at(&told, "field")),
                ("malformed".to_owned(), "$.noise".to_owned()),
                "{rule} {noise}: {}",
                written(&told)
            );
        }
    }
    let described = mcp.ok("grit_describe", "{}");
    assert!(written(&described).contains("\"noise_rule\""));
}

fn majority_square(row: u32) -> u32 {
    u32::from((row & 0x1FF).count_ones() >= 5)
}

fn majority_cube(row: u32) -> u32 {
    u32::from(row.count_ones() >= 14)
}

fn labelled(
    stencil: Stencil,
    rng: &mut Rng,
    size: [u32; 3],
    densities: &[u64],
    wrapped: bool,
    truth: fn(u32) -> u32,
) -> Vec<Example> {
    let cells = size.iter().product::<u32>() as usize;
    densities
        .iter()
        .map(|permille| {
            let input = random_bits(rng, cells, *permille);
            let output = own_rows(stencil, size, &input, wrapped)
                .into_iter()
                .map(truth)
                .collect();
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

fn every_row_halves(truth: fn(u32) -> u32) -> Vec<Example> {
    let size = [256, 16, 1];
    (0..2_u32)
        .map(|half| {
            let mut input = vec![0_u32; 4096];
            for at in 0..256_u32 {
                let row = half * 256 + at;
                let base = [(at % 64) * 4 + 1, (at / 64) * 4 + 1];
                for place in 0..9 {
                    if (row >> place) & 1 == 1 {
                        let [dx, dy, _] = Stencil::Square.offset(place);
                        let x = u32::try_from(i64::from(base[0]) + dx).unwrap();
                        let y = u32::try_from(i64::from(base[1]) + dy).unwrap();
                        input[stencil::index(size, [x, y, 0])] = 1;
                    }
                }
            }
            let output = own_rows(Stencil::Square, size, &input, false)
                .into_iter()
                .map(truth)
                .collect();
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

struct Lt1 {
    name: &'static str,
    stencil: Stencil,
    tally: Tally,
    border: &'static str,
    examples: Vec<Example>,
}

fn lt1_data() -> Vec<Lt1> {
    let mut rng = Rng(101);
    let a1 = labelled(
        Stencil::Square,
        &mut rng,
        [32, 32, 1],
        &[300, 500, 700, 300, 500, 700, 300, 500, 700, 300, 500, 700],
        false,
        majority_square,
    );
    let mut rng = Rng(202);
    let a2 = labelled(
        Stencil::Square,
        &mut rng,
        [32, 32, 1],
        &[250, 300, 350, 400, 450, 500, 600, 700],
        true,
        life,
    );
    let mut rng = Rng(303);
    let a3 = labelled(
        Stencil::Cube,
        &mut rng,
        [16, 16, 16],
        &[30, 150, 300, 500, 700, 850, 970],
        true,
        majority_cube,
    );
    vec![
        Lt1 {
            name: "a1",
            stencil: Stencil::Square,
            tally: Tally::Totalistic,
            border: "zero",
            examples: a1,
        },
        Lt1 {
            name: "a2",
            stencil: Stencil::Square,
            tally: Tally::Outer,
            border: "wrapped",
            examples: a2,
        },
        Lt1 {
            name: "a3",
            stencil: Stencil::Cube,
            tally: Tally::Totalistic,
            border: "wrapped",
            examples: a3,
        },
        Lt1 {
            name: "a4 erosion",
            stencil: Stencil::Square,
            tally: Tally::Totalistic,
            border: "zero",
            examples: every_row_halves(|row| u32::from(row & 0x1FF == 0x1FF)),
        },
        Lt1 {
            name: "a4 dilation",
            stencil: Stencil::Square,
            tally: Tally::Totalistic,
            border: "zero",
            examples: every_row_halves(|row| u32::from(row & 0x1FF != 0)),
        },
    ]
}

#[test]
fn n7_vote_on_clean_examples_answers_the_refuse_law_with_nothing_deviating() {
    let mut mcp = Mcp::new(None);
    for one in lt1_data() {
        let name = one.name;
        let plain = mcp.ok(
            "grit_law_learn",
            &args(one.stencil, one.tally.name(), one.border, &one.examples, ""),
        );
        let got = mcp.ok(
            "grit_law_learn",
            &args(
                one.stencil,
                one.tally.name(),
                one.border,
                &one.examples,
                VOTE,
            ),
        );
        let (Json::Map(before), Json::Map(after)) = (&plain, &got) else {
            panic!("{name}: answers are maps");
        };
        assert_eq!(after.len(), before.len() + 5, "{name}");
        for (was, now) in before.iter().zip(after) {
            assert_eq!(was.0, now.0, "{name}");
            assert_eq!(written(&was.1), written(&now.1), "{name}: {}", was.0);
        }
        let keys: Vec<&str> = after[before.len()..]
            .iter()
            .map(|pair| pair.0.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "tallies",
                "ties",
                "deviating",
                "deviating_cells",
                "deviating_share"
            ],
            "{name}"
        );
        for key in ["law", "chip", "tables", "hash"] {
            assert!(before.iter().any(|pair| pair.0 == key), "{name}: {key}");
        }
        let empty = grouped(one.tally, one.stencil, &|_| None);
        assert_eq!(shown_of(&got, "ties"), format!("[{empty}]"), "{name}");
        assert_eq!(shown_of(&got, "deviating"), "[0]", "{name}");
        assert_eq!(shown_of(&got, "deviating_cells"), "[[]]", "{name}");
        let cells: usize = one
            .examples
            .iter()
            .map(|example| example.output.len())
            .sum();
        assert_eq!(
            shown_of(&got, "deviating_share"),
            format!("[0,{cells}]"),
            "{name}"
        );
    }
}

#[test]
fn n7_the_same_vote_request_answers_the_same_bytes_after_a_restart() {
    let host = device("law_noise n7");
    let (life, _) = noisy(&life_pairs(), TRAIN + 202, 50, Flip::Both);
    let cube = torus_pairs(Stencil::Cube, cube_life, TRAIN + 300, 8, [16, 16, 16], 220);
    let (cube, _) = noisy(&cube, TRAIN + 302, 50, Flip::Output);
    let requests = [
        args(Stencil::Square, "outer", "wrapped", &life, VOTE),
        args(Stencil::Square, "totalistic", "wrapped", &life, VOTE),
        args(Stencil::Cube, "outer", "wrapped", &cube, VOTE),
        args(Stencil::Square, "outer", "zero", &tie_examples(), VOTE),
    ];
    for arguments in &requests {
        let mut first = Mcp::new(host.as_ref());
        let one = written(&first.ok("grit_law_learn", arguments));
        let two = written(&first.ok("grit_law_learn", arguments));
        drop(first);
        let mut again = Mcp::new(host.as_ref());
        let three = written(&again.ok("grit_law_learn", arguments));
        let mut bare = Mcp::new(None);
        let four = written(&bare.ok("grit_law_learn", arguments));
        assert_eq!(one, two);
        assert_eq!(one, three, "after a restart");
        assert_eq!(one, four, "with no device");
    }
}

fn shown_writes(examples: &[Example], writes: u32) -> String {
    let shown: Vec<String> = examples
        .iter()
        .map(|one| {
            let outs: Vec<String> = (0..writes)
                .map(|write| {
                    let held: Vec<u32> =
                        one.output.iter().map(|word| word & (1 << write)).collect();
                    listed(&held)
                })
                .collect();
            format!(
                "{{\"size\":[{},{},{}],\"input\":[{}],\"output\":[{}]}}",
                one.size[0],
                one.size[1],
                one.size[2],
                listed(&one.input),
                outs.join(",")
            )
        })
        .collect();
    format!("[{}]", shown.join(","))
}

fn args_writes(rule: &str, border: &str, examples: &[Example], writes: u32) -> String {
    let named: Vec<String> = (0..writes)
        .map(|bit| format!("{{\"plane\":1,\"bit\":{bit}}}"))
        .collect();
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{}],\"border\":\"{border}\",\"rule\":\"{rule}\",\"examples\":{}{VOTE},\"budget_ms\":{LONG}}}",
        named.join(","),
        shown_writes(examples, writes)
    )
}

fn crowded(row: u32) -> u32 {
    let centre = (row >> 4) & 1;
    let around = (row & 0x1FF).count_ones() - centre;
    centre ^ u32::from(around >= 4)
}

struct Written {
    tallies: Vec<Vec<[u64; 2]>>,
    deviating: Vec<u64>,
    cells: Vec<Vec<String>>,
    voted: u64,
}

fn host_votes(examples: &[Example], wrapped: bool, writes: u32) -> Written {
    let tally = Tally::Outer;
    let stencil = Stencil::Square;
    let classes = own_classes(tally, stencil) as usize;
    let counts = own_counts(tally, stencil);
    let mut tallies = vec![vec![[0_u64; 2]; classes]; writes as usize];
    let mut voted = 0;
    let rows: Vec<Vec<u32>> = examples
        .iter()
        .map(|one| own_rows(stencil, one.size, &one.input, wrapped))
        .collect();
    for (one, held) in examples.iter().zip(&rows) {
        for (row, out) in held.iter().zip(&one.output) {
            voted += 1;
            let class = own_class(tally, stencil, *row) as usize;
            for (write, own) in tallies.iter_mut().enumerate() {
                own[class][((out >> write) & 1) as usize] += 1;
            }
        }
    }
    let mut deviating = vec![0_u64; writes as usize];
    let mut cells = vec![Vec::new(); writes as usize];
    for (example, (one, held)) in examples.iter().zip(&rows).enumerate() {
        for (at, (row, out)) in held.iter().zip(&one.output).enumerate() {
            let class = own_class(tally, stencil, *row);
            let x = at as u32 % one.size[0];
            let y = at as u32 / one.size[0];
            for write in 0..writes as usize {
                let [zeros, ones] = tallies[write][class as usize];
                let law = u32::from(ones > zeros);
                let seen = (out >> write) & 1;
                if seen != law {
                    deviating[write] += 1;
                    if cells[write].len() < 64 {
                        cells[write].push(format!(
                            "{{\"example\":{example},\"at\":[{x},{y},0],\"count\":{},\"centre\":{},\"seen\":{seen},\"law\":{law}}}",
                            class % counts,
                            class / counts
                        ));
                    }
                }
            }
        }
    }
    Written {
        tallies,
        deviating,
        cells,
        voted,
    }
}

#[test]
fn g1_a_two_write_vote_tallies_and_deviates_per_write_as_the_host() {
    let mut mcp = Mcp::new(None);
    let clean = torus_pairs(Stencil::Square, life, TRAIN + 500, 8, [32, 32, 1], 350);
    let mut rng = Rng(TRAIN + 501);
    let examples: Vec<Example> = clean
        .iter()
        .map(|one| {
            let input = flipped(&mut rng, &one.input, 10);
            let rows = own_rows(Stencil::Square, one.size, &one.input, true);
            let low = flipped(&mut rng, &one.output, 30);
            let high = flipped(
                &mut rng,
                &rows.iter().map(|row| crowded(*row)).collect::<Vec<_>>(),
                60,
            );
            Example {
                size: one.size,
                input,
                output: low.iter().zip(&high).map(|(a, b)| a | (b << 1)).collect(),
            }
        })
        .collect();
    let got = mcp.ok(
        "grit_law_learn",
        &args_writes("outer", "wrapped", &examples, 2),
    );
    let own = host_votes(&examples, true, 2);
    assert_ne!(own.tallies[0], own.tallies[1], "the writes vote apart");
    assert_ne!(own.deviating[0], own.deviating[1]);
    let per_write = |each: &dyn Fn(usize, u32) -> Option<String>| -> String {
        let held: Vec<String> = (0..2)
            .map(|write| grouped(Tally::Outer, Stencil::Square, &|class| each(write, class)))
            .collect();
        format!("[{}]", held.join(","))
    };
    assert_eq!(
        shown_of(&got, "tallies"),
        per_write(&|write, class| {
            let [zeros, ones] = own.tallies[write][class as usize];
            Some(format!("[{zeros},{ones}]"))
        }),
        "tallies"
    );
    for held in &own.tallies {
        let summed: u64 = held.iter().map(|pair| pair[0] + pair[1]).sum();
        assert_eq!(summed, own.voted, "each write's tallies hold every cell");
    }
    assert_eq!(
        shown_of(&got, "ties"),
        per_write(&|write, class| {
            let [zeros, ones] = own.tallies[write][class as usize];
            (zeros == ones && zeros > 0)
                .then(|| (class % own_counts(Tally::Outer, Stencil::Square)).to_string())
        }),
        "ties"
    );
    assert_eq!(
        shown_of(&got, "tables"),
        per_write(&|write, class| {
            let [zeros, ones] = own.tallies[write][class as usize];
            Some(u32::from(ones > zeros).to_string())
        }),
        "tables"
    );
    assert_eq!(
        shown_of(&got, "deviating"),
        format!("[{},{}]", own.deviating[0], own.deviating[1])
    );
    assert_eq!(
        shown_of(&got, "deviating_cells"),
        format!(
            "[[{}],[{}]]",
            own.cells[0].join(","),
            own.cells[1].join(",")
        )
    );
    assert_eq!(
        shown_of(&got, "deviating_share"),
        format!(
            "[{},{}]",
            own.deviating[0] + own.deviating[1],
            own.voted * 2
        )
    );
    println!(
        "g1: deviating {:?} of {} cells a write",
        own.deviating, own.voted
    );
}

#[test]
fn g2_a_vote_that_leaves_rows_unseen_decides_them_as_the_host() {
    let mut mcp = Mcp::new(None);
    let clean = torus_pairs(Stencil::Square, life, TRAIN + 600, 4, [8, 8, 1], 350);
    let (examples, flips) = noisy(&clean, TRAIN + 601, 80, Flip::Output);
    assert!(flips > 0);
    let got = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &examples, VOTE),
    );
    let own = host_votes(&examples, true, 1);
    let law = |row: u32| {
        let [zeros, ones] = own.tallies[0][own_class(Tally::Outer, Stencil::Square, row) as usize];
        u32::from(ones > zeros)
    };
    let mut first: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for one in &examples {
        let rows = own_rows(Stencil::Square, one.size, &one.input, true);
        for (row, out) in rows.iter().zip(&one.output) {
            first.entry(*row).or_insert(out & 1);
        }
    }
    let unseen: Vec<u32> = (0..512).filter(|row| !first.contains_key(row)).collect();
    assert!(unseen.len() > 100, "the examples leave rows unseen");
    let ones = unseen.iter().filter(|row| law(**row) == 1).count();
    let zeros = unseen.len() - ones;
    let unseen_shown = got.get("unseen").unwrap();
    assert_eq!(int_at(unseen_shown, "rows"), unseen.len() as i64);
    assert_eq!(shown_of(unseen_shown, "ones"), format!("[{ones}]"));
    assert_eq!(shown_of(unseen_shown, "zeros"), format!("[{zeros}]"));
    assert_eq!(
        words(unseen_shown.get("unseen_rows").unwrap()),
        unseen,
        "unseen rows"
    );
    let table = words(unseen_shown.get("table").unwrap());
    for row in 0..512_u32 {
        assert_eq!(table[row as usize], law(row), "row {row}");
    }
    let first_ones = first.values().filter(|out| **out == 1).count();
    let law_ones = first.keys().filter(|row| law(**row) == 1).count();
    assert_ne!(
        first_ones, law_ones,
        "a seen row's first output disagrees with the vote, so the first-seen count differs"
    );
    println!(
        "g2: {} seen rows, {} unseen, {ones} decided 1, first-seen ones {first_ones} vs law {law_ones}",
        first.len(),
        unseen.len()
    );
}

fn zero_pairs(seed: u64, count: usize, size: [u32; 3]) -> Vec<Example> {
    let mut rng = Rng(seed);
    let cells = size.iter().product::<u32>() as usize;
    (0..count)
        .map(|at| {
            let input = random_bits(&mut rng, cells, 250 + 60 * at as u64);
            let output = own_rows(Stencil::Square, size, &input, false)
                .into_iter()
                .map(life)
                .collect();
            Example {
                size,
                input,
                output,
            }
        })
        .collect()
}

fn plane_written(mcp: &mut Mcp<'_>, world: u32, plane: u32, cells: &[([u32; 3], u32)]) {
    let shown: Vec<String> = cells
        .iter()
        .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
        .collect();
    mcp.ok(
        "grit_world_write",
        &format!(
            "{{\"world\":{world},\"plane\":{plane},\"cells\":[{}],\"budget_ms\":{LONG}}}",
            shown.join(",")
        ),
    );
}

#[test]
fn n8_deviating_cells_of_a_world_example_are_placed_in_its_box() {
    let Some(host) = device("law_noise n8") else {
        return;
    };
    let mut mcp = Mcp::new(Some(&host));
    let plain = mcp.ok(
        "grit_law_learn",
        &args(Stencil::Square, "outer", "wrapped", &life_pairs(), ""),
    );
    let law = text_at(&plain, "law");
    let size = [32_u32, 32, 2];
    let layer = [32_u32, 32, 1];
    let from = [5_u32, 7, 0];
    let side = 20_u32;
    let to = [from[0] + side - 1, from[1] + side - 1, 0];
    let mut rng = Rng(TRAIN + 800);
    let image = random_bits(&mut rng, 32 * 32, 350);
    let input = world_of(&mut mcp, &law, size, &image);
    let crop: Vec<u32> = (0..side * side)
        .map(|at| image[stencil::index(layer, [from[0] + at % side, from[1] + at / side, 0])])
        .collect();
    let flips: [[u32; 3]; 4] = [[0, 0, 0], [19, 0, 0], [5, 13, 0], [19, 19, 0]];
    for (border, inline) in [
        ("wrapped", life_pairs()),
        ("zero", zero_pairs(TRAIN + 801, 8, layer)),
    ] {
        let truth: Vec<u32> = if border == "wrapped" {
            let whole = stepped(Stencil::Square, layer, &image, life);
            (0..side * side)
                .map(|at| {
                    whole[stencil::index(layer, [from[0] + at % side, from[1] + at / side, 0])]
                })
                .collect()
        } else {
            own_rows(Stencil::Square, [side, side, 1], &crop, false)
                .into_iter()
                .map(life)
                .collect()
        };
        let output = mcp.world(&law, size);
        let cells: Vec<([u32; 3], u32)> = (0..side * side)
            .map(|at| {
                let place = [at % side, at / side, 0];
                let flip = u32::from(flips.contains(&place));
                (
                    [from[0] + place[0], from[1] + place[1], 0],
                    truth[at as usize] ^ flip,
                )
            })
            .collect();
        plane_written(&mut mcp, output, 1, &cells);
        let world_example = format!(
            "{{\"world\":{input},\"output_world\":{output},\"from\":[{},{},{}],\"to\":[{},{},{}]}}",
            from[0], from[1], from[2], to[0], to[1], to[2]
        );
        let inline_shown = shown(&inline);
        let examples = format!(
            "{},{world_example}]",
            &inline_shown[..inline_shown.len() - 1]
        );
        let got = mcp.ok(
            "grit_law_learn",
            &format!(
                "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"{border}\",\"rule\":\"outer\",\"examples\":{examples}{VOTE},\"budget_ms\":{LONG}}}"
            ),
        );
        assert_eq!(shown_of(&got, "tables"), LIFE_TABLES, "{border}");
        assert_eq!(shown_of(&got, "deviating"), "[4]", "{border}");
        let listed = got.get("deviating_cells").and_then(Json::list).unwrap()[0]
            .list()
            .unwrap()
            .to_vec();
        let example = inline.len() as i64;
        let places: Vec<(i64, Vec<u32>)> = listed
            .iter()
            .map(|one| (int_at(one, "example"), words(one.get("at").unwrap())))
            .collect();
        let wanted: Vec<(i64, Vec<u32>)> = flips.iter().map(|at| (example, at.to_vec())).collect();
        assert_eq!(places, wanted, "{border}");
        for (one, at) in listed.iter().zip(&flips) {
            let truth_here = truth[(at[1] * side + at[0]) as usize];
            assert_eq!(int_at(one, "law"), i64::from(truth_here), "{border} {at:?}");
            assert_eq!(
                int_at(one, "seen"),
                i64::from(truth_here ^ 1),
                "{border} {at:?}"
            );
        }
        println!("n8 {border}: deviating cells at {places:?}");
    }
}

fn world_of(mcp: &mut Mcp<'_>, law: &str, size: [u32; 3], image: &[u32]) -> u32 {
    let world = mcp.world(law, size);
    let mut cells = Vec::new();
    for y in 0..size[1] {
        for x in 0..size[0] {
            let word = image[(y * size[0] + x) as usize];
            if word != 0 {
                cells.push(([x, y, 0], word));
            }
        }
    }
    mcp.write(world, &cells);
    world
}
