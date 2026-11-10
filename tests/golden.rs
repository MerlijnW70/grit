#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;
use std::fmt::{Debug, Write as _};
use std::path::PathBuf;

use common::device;

use grit::compact::{self, Over, Plan, Tier};
use grit::layout::{
    BRUSH, COUNTS, DENSE, Dimension, GRAND, HOT_BASES, HOT_COUNTS, HOT_DENSE, PARAMS, STATE, TOTALS,
};
use grit::machine::Machine;
use grit::marking;
use grit::netlist::{KINDS, Netlist};
use grit::rule::{self, Kind, Pass};
use grit::rule3::code::Code;
use grit::rule3::heat::Heat;
use grit::sand::{self, AIR, Grid, SAND, View, WALL, WATER};
use grit::{Groups, live, sand3, sweep};

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const GRIDS: [(u32, u32); 3] = [(256, 128), (512, 256), (4096, 2048)];

const SOLIDS: [(u32, u32, u32); 3] = [(16, 16, 16), (64, 32, 16), (256, 256, 256)];

const SPIRV_FILE: &str = "tests/golden/spirv.txt";
const STATE_FILE: &str = "tests/golden/state.txt";

fn writing() -> bool {
    match std::env::var("GRIT_GOLDEN") {
        Err(_) => false,
        Ok(held) if held == "write" => true,
        Ok(held) => panic!("GRIT_GOLDEN is {held:?}; it must be write or unset"),
    }
}

fn path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file)
}

const ROUNDS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn sha256(bytes: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&(bytes.len() as u64 * 8).to_be_bytes());
    for block in padded.as_chunks::<64>().0.iter() {
        let mut w = [0_u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let a = w[i - 15];
            let b = w[i - 2];
            let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let choose = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(ROUNDS[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let majority = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(majority);
            v[7] = v[6];
            v[6] = v[5];
            v[5] = v[4];
            v[4] = v[3].wrapping_add(t1);
            v[3] = v[2];
            v[2] = v[1];
            v[1] = v[0];
            v[0] = t1.wrapping_add(t2);
        }
        for (slot, add) in state.iter_mut().zip(v) {
            *slot = slot.wrapping_add(add);
        }
    }
    state.iter().fold(String::new(), |mut out, word| {
        let _ = write!(out, "{word:08x}");
        out
    })
}

fn words_hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    sha256(&bytes)
}

#[test]
fn the_hash_function_gives_the_published_digests() {
    assert_eq!(
        sha256(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

struct Catalog {
    width: u32,
    found: Vec<(String, String)>,
}

impl Catalog {
    fn add<E: Debug>(&mut self, name: impl Into<String>, built: Result<Vec<u32>, E>) {
        let name = name.into();
        let words = built.unwrap_or_else(|why| {
            panic!(
                "{name} at width {}: the builder refused: {why:?}",
                self.width
            )
        });
        self.found.push((name, words_hash(&words)));
    }

    fn text(&mut self, name: impl Into<String>, text: &str) {
        self.found.push((name.into(), sha256(text.as_bytes())));
    }

    fn loop_of<E: Debug>(&mut self, name: &str, built: Result<live::Loop, E>) {
        let (passes, stages) = built.unwrap_or_else(|why| {
            panic!(
                "{name} at width {}: the builder refused: {why:?}",
                self.width
            )
        });
        let mut at = 0;
        for (stage, groups) in stages.iter().enumerate() {
            for (seat, over) in groups.iter().enumerate() {
                let module = passes
                    .get(at)
                    .unwrap_or_else(|| panic!("{name}: stage {stage} has more groups than passes"));
                self.found
                    .push((format!("{name}/{stage}.{seat}"), words_hash(module)));
                self.text(
                    format!("{name}/{stage}.{seat}/groups"),
                    &format!("{over:?}"),
                );
                at += 1;
            }
        }
        assert_eq!(at, passes.len(), "{name}: passes and groups disagree");
    }

    fn stages<E: Debug>(&mut self, name: &str, built: Result<live::Stages, E>) {
        let stages = built.unwrap_or_else(|why| {
            panic!(
                "{name} at width {}: the builder refused: {why:?}",
                self.width
            )
        });
        for (stage, members) in stages.iter().enumerate() {
            for (seat, (module, over)) in members.iter().enumerate() {
                self.found
                    .push((format!("{name}/{stage}.{seat}"), words_hash(module)));
                self.text(
                    format!("{name}/{stage}.{seat}/groups"),
                    &format!("{over:?}"),
                );
            }
        }
    }
}

fn catalog(width: u32) -> Vec<(String, String)> {
    let mut all = Catalog {
        width,
        found: Vec::new(),
    };
    let w = width;
    let tiers = [("fine", Tier::FINE), ("coarse", Tier::COARSE)];
    let overs = [
        ("every", Over::Every),
        ("listed", Over::Listed(DENSE)),
        ("hot-listed", Over::Listed(HOT_DENSE)),
    ];

    all.add("live::tick", live::tick(w));
    all.add("live::wipe", live::wipe(w));
    all.add("live::wipe_counts/fine", live::wipe_counts(w, COUNTS));
    all.add("live::wipe_counts/coarse", live::wipe_counts(w, HOT_COUNTS));
    all.add("sweep::sweep", sweep::sweep(w));
    for (label, over) in overs {
        all.add(format!("live::decay/{label}"), live::decay(w, over));
    }
    for (tier_label, tier) in tiers {
        for (label, over) in overs {
            all.add(
                format!("live::mark/{tier_label}/{label}"),
                live::mark(w, tier, over),
            );
            all.add(
                format!("compact::gather/{tier_label}/{label}/dump-7"),
                compact::gather(w, tier, over, 7),
            );
        }
        all.add(
            format!("compact::whole/{tier_label}"),
            compact::whole(w, tier.counts, tier.bases, tier),
        );
        all.add(
            format!("compact::scan/{tier_label}"),
            compact::scan(w, tier.counts, tier.bases, tier.totals),
        );
        all.add(
            format!("compact::prepare/{tier_label}/1"),
            compact::prepare(w, 1, tier),
        );
        all.add(
            format!("compact::prepare/{tier_label}/width"),
            compact::prepare(w, w, tier),
        );
    }
    all.add(
        "compact::scan/totals",
        compact::scan(w, TOTALS, TOTALS, GRAND),
    );
    all.add(
        "compact::whole/hot-counts",
        compact::whole(w, HOT_COUNTS, HOT_BASES, Tier::COARSE),
    );

    for (wide, deep) in GRIDS {
        let grid = Grid::of(wide, deep);
        let at = format!("@{wide}x{deep}");
        let Ok(plan) = Plan::over(w, grid.quads().div_ceil(sweep::cells(w))) else {
            continue;
        };
        let view = View::of(wide, deep);
        let small = View::of(wide / 2, deep / 2);
        let sizes = format!(
            "{:?} dump {} hot {}",
            plan.sizes(),
            plan.dump(),
            plan.hot_dump()
        );
        all.text(format!("plan{at}"), &sizes);

        all.add(format!("live::unclaimed{at}"), live::unclaimed(w, &plan));
        all.add(format!("live::settle{at}"), live::settle(w, grid));
        all.add(format!("sand::settle{at}"), sand::settle(w, grid));
        all.add(
            format!("sand::brush/waking{at}"),
            sand::brush(w, grid, true),
        );
        all.add(
            format!("sand::brush/plain{at}"),
            sand::brush(w, grid, false),
        );
        all.add(format!("sand::paint/full{at}"), sand::paint(w, grid, view));
        all.add(format!("sand::paint/half{at}"), sand::paint(w, grid, small));
        all.add(
            format!("live::mark_below/fine/every{at}"),
            live::mark_below(w, Tier::FINE, Over::Every, grid.quads()),
        );
        all.add(
            format!("live::mark_below/fine/hot-listed{at}"),
            live::mark_below(w, Tier::FINE, Over::Listed(HOT_DENSE), grid.quads()),
        );
        all.add(
            format!("live::mark_below/fine/every-plan-dump{at}"),
            live::mark_below(w, Tier::FINE, Over::Every, plan.dump()),
        );
        all.add(
            format!("live::mark_below/coarse/every{at}"),
            live::mark_below(w, Tier::COARSE, Over::Every, plan.blocks),
        );
        all.add(
            format!("compact::gather/fine/every{at}"),
            compact::gather(w, Tier::FINE, Over::Every, plan.dump()),
        );
        all.add(
            format!("compact::gather/fine/hot-listed{at}"),
            compact::gather(w, Tier::FINE, Over::Listed(HOT_DENSE), plan.dump()),
        );
        all.add(
            format!("compact::gather/coarse/every{at}"),
            compact::gather(w, Tier::COARSE, Over::Every, plan.hot_dump()),
        );
        for pass in [Pass::Ahead, Pass::Back] {
            for kind in [
                Kind::Product,
                Kind::Sum,
                Kind::Turn,
                Kind::Bound,
                Kind::Smooth,
                Kind::Living,
                Kind::Twinned,
            ] {
                all.add(
                    format!("rule::built/{pass:?}/{kind:?}{at}"),
                    rule::built(w, grid, pass, kind),
                );
            }
        }
        for coarse in [false, true] {
            let tier = if coarse { "coarse" } else { "fine" };
            all.loop_of(
                &format!("live::arranged/blind/{tier}{at}"),
                live::arranged(w, grid, &plan, None, coarse),
            );
            all.loop_of(
                &format!("live::arranged/painting/{tier}{at}"),
                live::arranged(w, grid, &plan, Some(view), coarse),
            );
            match live::keeping(w, &plan, coarse) {
                Ok(held) => {
                    all.stages(
                        &format!("live::keeping/{tier}/before{at}"),
                        Ok::<_, ()>(held.before),
                    );
                    all.stages(
                        &format!("live::keeping/{tier}/after{at}"),
                        Ok::<_, ()>(held.after),
                    );
                    all.text(
                        format!("live::keeping/{tier}/groups{at}"),
                        &format!("{:?} {:?}", held.over, held.brush),
                    );
                }
                Err(why) => panic!("live::keeping/{tier}{at} at width {w}: refused: {why:?}"),
            }
        }
    }

    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        for most in [u32::MAX, 65_535] {
            let at = format!("@{wide}x{deep}x{tall}/most-{most}");
            match sand3::settle(w, grid, most) {
                Ok(built) => {
                    all.found
                        .push((format!("sand3::settle{at}"), words_hash(&built.words)));
                    all.text(
                        format!("sand3::settle{at}/spread"),
                        &format!("{:?}", built.spread),
                    );
                }
                Err(why) => panic!("sand3::settle{at} at width {w}: refused: {why:?}"),
            }
        }
    }

    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let at = format!("@{wide}x{deep}x{tall}");
        all.add(
            format!("sand3::settle_by/heat{at}"),
            sand3::settle_by(&Heat, w, grid, u32::MAX).map(|built| built.words),
        );
        all.add(
            format!("live::settle3_by/heat{at}"),
            live::settle3_by(&Heat, w, grid),
        );
    }

    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let at = format!("@{wide}x{deep}x{tall}");
        all.add(
            format!("sand3::settle_by/code{at}"),
            sand3::settle_by(&Code, w, grid, u32::MAX).map(|built| built.words),
        );
        all.add(
            format!("live::settle3_by/code{at}"),
            live::settle3_by(&Code, w, grid),
        );
    }

    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let at = format!("@{wide}x{deep}x{tall}");
        all.add(format!("live::settle3{at}"), live::settle3(w, grid));
        let Ok(plan) = Plan::over_in(w, grid.blocks().div_ceil(sweep::cells(w)), Dimension::Three)
        else {
            continue;
        };
        all.text(
            format!("plan3{at}"),
            &format!("{:?} dump {}", plan.sizes(), plan.dump()),
        );
        for coarse in [false, true] {
            let tier = if coarse { "coarse" } else { "fine" };
            all.loop_of(
                &format!("live::arranged3/{tier}{at}"),
                live::arranged3(w, grid, &plan, coarse),
            );
        }
    }

    let shapes = [
        Netlist::over(4, 2, 8, 4).unwrap(),
        Netlist::over(3, 1, 4, 4).unwrap(),
        Netlist::over(12, 4, 40, 16).unwrap(),
        Netlist::over(2, 2, 2, KINDS).unwrap(),
    ];
    for shape in shapes {
        let shape_label = format!(
            "{}.{}.{}.{}",
            shape.inputs, shape.outputs, shape.gates, shape.kinds
        );
        all.add(
            format!("marking::marked/{shape_label}"),
            marking::marked(w, shape),
        );
        all.add(
            format!("marking::turned_on/{shape_label}"),
            marking::turned_on(w, shape),
        );
        for spread in [0, 256] {
            all.add(
                format!("marking::chosen/{shape_label}/spread-{spread}"),
                marking::chosen(w, shape, spread, 256),
            );
        }
    }
    for (state, inputs, outputs, gates, kinds, steps) in [
        (2, 1, 1, 8, 16, 8),
        (2, 1, 2, 12, 16, 16),
        (2, 3, 1, 9, 16, 8),
    ] {
        let held = Machine::over(state, inputs, outputs, gates, kinds, steps).unwrap();
        for wide in [1, 4] {
            all.add(
                format!("marking::stepped/{state}.{inputs}.{outputs}.{gates}.{kinds}.{steps}/wide-{wide}"),
                marking::stepped(w, held, wide),
            );
        }
    }

    all.found
}

fn key_of(width: u32, name: &str) -> String {
    format!("{width} {name}")
}

fn read_lines(file: &str, columns: usize) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path(file)).unwrap_or_else(|why| {
        panic!("{file} is unreadable: {why}; refresh with GRIT_GOLDEN=write cargo test --release --test golden")
    });
    let mut map = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let parts: Vec<&str> = line.split(' ').collect();
        assert_eq!(parts.len(), columns, "{file}: malformed line {line:?}");
        let (hash, key) = parts.split_last().unwrap();
        let before = map.insert(key.join(" "), (*hash).to_owned());
        assert!(
            before.is_none(),
            "{file}: key {:?} appears twice",
            key.join(" ")
        );
    }
    map
}

fn write_lines(file: &str, lines: &[(String, String)]) {
    let mut text = String::new();
    for (key, hash) in lines {
        let _ = writeln!(text, "{key} {hash}");
    }
    std::fs::write(path(file), text).unwrap();
}

#[test]
fn every_kernel_the_engine_builds_is_the_one_on_record() {
    let mut now: Vec<(String, String)> = Vec::new();
    for width in WIDTHS {
        for (name, hash) in catalog(width) {
            now.push((key_of(width, &name), hash));
        }
    }
    let keys: std::collections::BTreeSet<&String> = now.iter().map(|(key, _)| key).collect();
    assert_eq!(keys.len(), now.len(), "two kernels share one name");

    if writing() {
        write_lines(SPIRV_FILE, &now);
        println!("wrote {} lines to {SPIRV_FILE}", now.len());
        return;
    }

    let mut on_record = read_lines(SPIRV_FILE, 3);
    let mut wrong = Vec::new();
    for (key, hash) in &now {
        let (width, name) = key.split_once(' ').unwrap();
        match on_record.remove(key) {
            None => wrong.push(format!(
                "kernel {name} at width {width} is not on record; now {hash}"
            )),
            Some(golden) if golden != *hash => wrong.push(format!(
                "kernel {name} at width {width} changed: record {golden}, now {hash}"
            )),
            Some(_) => {}
        }
    }
    for key in on_record.keys() {
        let (width, name) = key.split_once(' ').unwrap();
        wrong.push(format!(
            "kernel {name} at width {width} is on record and the engine no longer builds it"
        ));
    }
    assert!(
        wrong.is_empty(),
        "{} kernels differ from {SPIRV_FILE} (refresh with GRIT_GOLDEN=write only when the change is meant):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

type Brushing = fn(u32, Grid) -> [u32; 4];
type Starting = fn(Grid) -> Vec<u32>;

struct Scene {
    name: &'static str,
    frames: u32,
    start: Starting,
    brushing: Brushing,
}

fn walled(grid: Grid, fill: impl Fn(u32, u32) -> bool) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in 0..grid.deep {
        for column in 0..grid.wide {
            let edge = row + 1 >= grid.deep || column == 0 || column + 1 >= grid.wide;
            let at = (row * grid.wide + column) as usize;
            cells[at] = if edge {
                WALL
            } else if fill(column, row) {
                SAND
            } else {
                AIR
            };
        }
    }
    cells
}

fn idle_room(grid: Grid) -> Vec<u32> {
    walled(grid, |_, _| false)
}

fn row_of_sand(grid: Grid) -> Vec<u32> {
    walled(grid, |column, row| {
        row == grid.deep / 8
            && column >= grid.wide / 2 - 128
            && column < grid.wide / 2 + 128
            && column % 4 == 0
    })
}

fn column_of_sand(grid: Grid) -> Vec<u32> {
    walled(grid, |column, row| {
        column == grid.wide / 2 && row >= grid.deep / 8 && row < grid.deep / 8 + 128 && row % 4 == 0
    })
}

fn busy_room(grid: Grid) -> Vec<u32> {
    walled(grid, |_, row| row % 4 == 2 && row + 8 < grid.deep)
}

fn floor_only(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for column in 0..grid.wide {
        cells[((grid.deep - 1) * grid.wide + column) as usize] = WALL;
    }
    cells
}

fn blob_on_side_seam(grid: Grid) -> Vec<u32> {
    let mut cells = floor_only(grid);
    for row in 180..250 {
        for column in (grid.wide - 30)..(grid.wide + 30) {
            let column = column % grid.wide;
            cells[(row * grid.wide + column) as usize] = if row < 200 { SAND } else { WATER };
        }
    }
    cells
}

fn blob_on_floor_seam(grid: Grid) -> Vec<u32> {
    let mut cells = vec![AIR; grid.cells()];
    for row in (grid.deep - 20)..(grid.deep + 20) {
        let row = row % grid.deep;
        for column in 100..160 {
            cells[(row * grid.wide + column) as usize] = if column < 130 { SAND } else { WATER };
        }
    }
    cells
}

fn no_brush(_frame: u32, _grid: Grid) -> [u32; 4] {
    [0, 0, 0, AIR]
}

fn pour_at_the_side_seam(frame: u32, _grid: Grid) -> [u32; 4] {
    let material = match frame {
        5..60 => WATER,
        60..90 => SAND,
        _ => AIR,
    };
    [3, 200, 36, material]
}

fn a_stroke(frame: u32, grid: Grid) -> [u32; 4] {
    if frame >= 100 {
        return [0, 0, 0, AIR];
    }
    let x = (frame * 5) % grid.wide;
    let y = 40 + (frame * 3) % 100;
    let reach = 1 + frame % 6;
    let material = match (frame / 10) % 4 {
        0 => SAND,
        1 => WATER,
        2 => WALL,
        _ => AIR,
    };
    [x, y, reach, material]
}

const SCENES: [Scene; 7] = [
    Scene {
        name: "idle",
        frames: 64,
        start: idle_room,
        brushing: no_brush,
    },
    Scene {
        name: "row",
        frames: 120,
        start: row_of_sand,
        brushing: no_brush,
    },
    Scene {
        name: "column",
        frames: 120,
        start: column_of_sand,
        brushing: no_brush,
    },
    Scene {
        name: "busy",
        frames: 120,
        start: busy_room,
        brushing: no_brush,
    },
    Scene {
        name: "side-seam",
        frames: 120,
        start: blob_on_side_seam,
        brushing: pour_at_the_side_seam,
    },
    Scene {
        name: "floor-seam",
        frames: 120,
        start: blob_on_floor_seam,
        brushing: no_brush,
    },
    Scene {
        name: "brush",
        frames: 140,
        start: idle_room,
        brushing: a_stroke,
    },
];

const OPEN: Grid = Grid::of(512, 256);

fn run(host: &grit::Host, scene: &Scene, coarse: bool) -> String {
    let width = host.width();
    let grid = OPEN;
    let plan = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap();
    let (passes, stages) = live::arranged(width, grid, &plan, None, coarse).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = plan.sizes();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        if binding == grit::layout::PIXELS as usize {
            continue;
        }
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
    held.write(STATE as usize, &(scene.start)(grid)).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    for frame in 0..scene.frames {
        let brush = (scene.brushing)(frame, grid);
        held.write_at(PARAMS as usize, BRUSH, &brush).unwrap();
        held.staged(&groups).unwrap();
    }
    let state = held.read(STATE as usize, grid.cells()).unwrap();
    words_hash(&state)
}

#[test]
fn the_fixed_scenes_end_in_the_state_on_record() {
    let Some(host) = device("the_fixed_scenes_end_in_the_state_on_record") else {
        return;
    };
    let mut now: Vec<(String, String)> = Vec::new();
    for scene in &SCENES {
        for (tier, coarse) in [("fine", false), ("coarse", true)] {
            let hash = run(&host, scene, coarse);
            now.push((format!("{} {tier} {}", scene.name, scene.frames), hash));
        }
    }

    if writing() {
        write_lines(STATE_FILE, &now);
        println!("wrote {} lines to {STATE_FILE}", now.len());
        return;
    }

    let mut on_record = read_lines(STATE_FILE, 4);
    let mut wrong = Vec::new();
    for (key, hash) in &now {
        match on_record.remove(key) {
            None => wrong.push(format!("scene {key} is not on record; now {hash}")),
            Some(golden) if golden != *hash => wrong.push(format!(
                "scene {key} frames ended in a different state on {} at subgroup {}: record {golden}, now {hash}",
                host.name(),
                host.width()
            )),
            Some(_) => {}
        }
    }
    for key in on_record.keys() {
        wrong.push(format!("scene {key} is on record and no longer run"));
    }
    assert!(
        wrong.is_empty(),
        "{} scenes differ from {STATE_FILE} (refresh with GRIT_GOLDEN=write only when the change is meant):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    println!(
        "{} scenes end in the recorded state on {} at subgroup {}",
        now.len(),
        host.name(),
        host.width()
    );
}
