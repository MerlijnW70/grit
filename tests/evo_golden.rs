#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use common::device;

use grit::bricks::Bricks;
use grit::compact::Plan;
use grit::law::Law;
use grit::layout::{Dimension, PARAMS, STATE};
use grit::rule3::evo::{self, Evo};
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3};
use grit::{Groups, live, sweep};

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];

const SOLIDS: [(u32, u32, u32); 3] = [(16, 16, 16), (64, 32, 16), (256, 256, 256)];

const FILE: &str = "tests/golden/evo.txt";

const EVO: &str = include_str!("law/evo.law.json");

const SCENE: (u32, u32, u32) = (32, 32, 32);

const SCENE_FRAMES: u32 = 256;

fn writing() -> bool {
    std::env::var("GRIT_GOLDEN").is_ok_and(|held| held == "write")
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FILE)
}

fn hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    grit::law::hex(&grit::branch::sha256(&bytes))
}

fn kernels<R: Rule>(name: &str, rule: &R, width: u32, out: &mut Vec<(String, String)>) {
    for (wide, deep, tall) in SOLIDS {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        let at = format!("@{wide}x{deep}x{tall}");
        let whole = sand3::settle_by(rule, width, grid, u32::MAX).unwrap().words;
        out.push((format!("{width} sand3::settle_by/{name}{at}"), hash(&whole)));
        let listed = live::settle3_by(rule, width, grid).unwrap();
        out.push((
            format!("{width} live::settle3_by/{name}{at}"),
            hash(&listed),
        ));
        if wide > 64 {
            continue;
        }
        let plan = Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).unwrap();
        let pool = Bricks::new(grid, &plan, plan.blocks.min(1 << 12)).unwrap();
        let (passes, _) = grit::bricks::arranged(rule, &pool, &plan, true).unwrap();
        let joined: Vec<u32> = passes.concat();
        out.push((
            format!("{width} bricks::arranged/{name}{at}"),
            hash(&joined),
        ));
    }
}

fn scene() -> Vec<u32> {
    let (wide, deep, tall) = SCENE;
    let grid = Grid3::new(wide, deep, tall).unwrap();
    let mut cells = vec![0_u32; grid.cells()];
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let near = |at: u32, side: u32| at < 3 || at + 3 >= side;
                let spot = x * 7 + y * 13 + z * 29;
                if near(x, wide) && near(y, deep) && near(z, tall) && spot % 3 != 0 {
                    let genome = evo::genome(spot % 4, (spot / 4) % 4, (spot / 16) % 4, spot);
                    cells[grid.index(x, y, z)] = evo::cell(genome, 60 + spot % 90, spot % 20);
                }
            }
        }
    }
    cells
}

fn scene_params() -> Vec<u32> {
    let mut params = evo::DEFAULTS.to_vec();
    params[evo::MUTATION] = 1024;
    params[evo::SEED] = 0x0E70;
    params
}

fn scene_hash() -> String {
    let (wide, deep, tall) = SCENE;
    let grid = Grid3::new(wide, deep, tall).unwrap();
    let params = scene_params();
    let mut cells = scene();
    for frame in 0..SCENE_FRAMES {
        cells = sand3::reference(&Evo, grid, frame, &cells, &params);
    }
    hash(&cells)
}

fn catalog() -> Vec<(String, String)> {
    let law = Law::compile(EVO).unwrap();
    let mut out = Vec::new();
    for width in WIDTHS {
        kernels("evo", &Evo, width, &mut out);
        kernels("evo-law", &law, width, &mut out);
    }
    let (wide, deep, tall) = SCENE;
    out.push((
        format!("state evo/corner@{wide}x{deep}x{tall}/frames-{SCENE_FRAMES}"),
        scene_hash(),
    ));
    out
}

fn on_record() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path()).unwrap_or_else(|why| {
        panic!("{FILE} is unreadable: {why}; refresh with GRIT_GOLDEN=write cargo test --release --test evo_golden")
    });
    let mut map = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (key, hash) = line.rsplit_once(' ').unwrap();
        assert!(
            map.insert(key.to_owned(), hash.to_owned()).is_none(),
            "{key} twice"
        );
    }
    map
}

#[test]
fn every_evo_kernel_and_the_evo_scene_are_the_ones_on_record() {
    let now = catalog();
    if writing() {
        let mut text = String::new();
        for (key, hash) in &now {
            let _ = writeln!(text, "{key} {hash}");
        }
        std::fs::write(path(), text).unwrap();
        println!("wrote {} lines to {FILE}", now.len());
        return;
    }
    let mut record = on_record();
    let mut wrong = Vec::new();
    for (key, hash) in &now {
        match record.remove(key) {
            None => wrong.push(format!("{key} is not on record; now {hash}")),
            Some(golden) if golden != *hash => {
                wrong.push(format!("{key} changed: record {golden}, now {hash}"));
            }
            Some(_) => {}
        }
    }
    wrong.extend(
        record
            .keys()
            .map(|key| format!("{key} is on record and no longer built")),
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    println!("{} evo goldens match {FILE}", now.len());
}

#[test]
fn the_evo_scene_ends_in_the_state_on_record_on_the_device() {
    let Some(host) = device("evo golden scene") else {
        return;
    };
    let (wide, deep, tall) = SCENE;
    let key = format!("state evo/corner@{wide}x{deep}x{tall}/frames-{SCENE_FRAMES}");
    let record = on_record();
    let golden = record.get(&key).unwrap();
    let width = host.width();
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap();
    let (passes, stages) = live::wired3_by(&Evo, width, grid, &plan).unwrap();
    let sizes = live::sizes3(&Evo, &plan);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &room) in sizes.iter().enumerate() {
        if binding != grit::layout::PIXELS as usize {
            held.write(binding, &vec![0; room]).unwrap();
        }
    }
    held.write(STATE as usize, &scene()).unwrap();
    held.write_at(PARAMS as usize, FIRST_PARAM, &scene_params())
        .unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    held.write(PARAMS as usize, &[u32::MAX]).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    held.staged_frames(&groups, SCENE_FRAMES).unwrap();
    let cells = held.read(STATE as usize, grid.cells()).unwrap();
    println!(
        "evo scene on {} width {width}: {}",
        host.name(),
        hash(&cells)
    );
    assert_eq!(&hash(&cells), golden);
}
