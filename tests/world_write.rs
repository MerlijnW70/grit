#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Traffic;
use grit::branch::{Made, World, sha256};
use grit::law::Law;

const MAJORITY: &str = include_str!("world_write/majority.law.json");
const HEAT: &str = include_str!("law/heat.law.json");
const MIX: &str = include_str!("law/mix.law.json");

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn hex(sum: &[u8; 32]) -> String {
    sum.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn words_hash(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    hex(&sha256(&bytes))
}

const EDGE_WORDS: [u32; 6] = [0xFFFF_FFFF, 0x8000_0000, 0x7FFF_FFFF, 1, 0, 0x0001_0000];

fn scattered(seed: u64, size: [u32; 3], count: usize, mask: u32) -> Vec<([u32; 3], u32)> {
    let mut state = seed;
    let mut cells = Vec::with_capacity(count + count / 8);
    for _ in 0..count {
        let at = [
            (next(&mut state) % u64::from(size[0])) as u32,
            (next(&mut state) % u64::from(size[1])) as u32,
            (next(&mut state) % u64::from(size[2])) as u32,
        ];
        cells.push((at, (next(&mut state) as u32) & mask));
    }
    for again in 0..count / 8 {
        let (at, _) = cells[(next(&mut state) as usize) % cells.len()];
        cells.push((at, EDGE_WORDS[again % EDGE_WORDS.len()] & mask));
    }
    cells
}

fn edges(size: [u32; 3], mask: u32) -> Vec<([u32; 3], u32)> {
    let mut cells = Vec::new();
    let mut at = 0_usize;
    for x in [0, 1, size[0] / 2, size[0] - 2, size[0] - 1] {
        for y in [0, size[1] / 2 - 1, size[1] / 2, size[1] - 1] {
            for z in [0, size[2] - 1] {
                cells.push(([x, y, z], EDGE_WORDS[at % EDGE_WORDS.len()] & mask));
                at += 1;
            }
        }
    }
    cells
}

const SIDES: [u32; 3] = [64, 256, 1024];

const CELLS: usize = 4096;

fn spread(seed: u64) -> Vec<([u32; 3], u32)> {
    let mut state = seed;
    let mut cells = Vec::with_capacity(CELLS);
    'filled: for y in 4..60 {
        for x in 4..60 {
            for z in 0..2 {
                if cells.len() == CELLS {
                    break 'filled;
                }
                cells.push(([x, y, z], (next(&mut state) & 1) as u32));
            }
        }
    }
    cells
}

fn row(seed: u64, at: usize, count: usize) -> Vec<([u32; 3], u32)> {
    let mut state = seed;
    (0..count)
        .map(|index| {
            let place = at + index;
            let x = (place % 1000) as u32 + 8;
            let y = ((place / 1000) * 4) as u32 + 8;
            ([x, y, 0], (next(&mut state) & 1) as u32)
        })
        .collect()
}

fn majority_world(host: &grit::Host, side: u32) -> World<'_> {
    let law = Law::compile(MAJORITY).unwrap();
    World::create_law(
        host,
        &law,
        &Made {
            rule: law.name(),
            params: &[],
            size: [side, side, 2],
            sparse: false,
            slots: None,
        },
    )
    .unwrap()
}

fn written(world: &mut World<'_>, cells: &[([u32; 3], u32)]) -> Traffic {
    let before = world.traffic();
    world.write_plane(0, cells).unwrap();
    world.traffic().since(before)
}

fn bounded(label: &str, cells: usize, moved: Traffic) {
    let cells = cells as u64;
    assert!(moved.submits <= 3, "{label}: {moved:?}");
    assert!(moved.woken <= 54 * cells, "{label}: {moved:?}");
    let words = cells + 3 * moved.woken + 16;
    assert!(moved.uploaded <= 4 * words, "{label}: {moved:?}");
}

#[test]
fn a_write_moves_the_same_whatever_the_size_of_the_world() {
    let Some(host) = device("world write bounds") else {
        return;
    };
    let cells = spread(0xB0_0D);
    let mut fresh = Vec::new();
    let mut quiet = Vec::new();
    for side in SIDES {
        let mut world = majority_world(&host, side);
        let first = written(&mut world, &cells);
        bounded(&format!("fresh {side}"), cells.len(), first);
        assert_eq!(first.downloaded, 8, "fresh {side}: {first:?}");
        fresh.push(first);
        world.step(3).unwrap();
        assert_eq!(world.listed().unwrap(), 0, "{side}: the world fell quiet");
        let again = written(&mut world, &cells);
        bounded(&format!("quiet {side}"), cells.len(), again);
        quiet.push(again);
        println!("bounds {side}: fresh {first:?} quiet {again:?}");
    }
    for moved in [&fresh, &quiet] {
        for one in moved {
            assert_eq!(one.submits, moved[0].submits, "{moved:?}");
            assert_eq!(one.woken, moved[0].woken, "{moved:?}");
        }
        let hot = moved[0].woken;
        for one in moved {
            assert!(
                one.uploaded.abs_diff(moved[0].uploaded) <= 4 * hot,
                "{moved:?}"
            );
            assert!(
                one.downloaded.abs_diff(moved[0].downloaded) <= 4 * hot,
                "{moved:?}"
            );
        }
    }
}

#[test]
fn the_first_step_after_writes_does_the_same_work_however_many_were_written() {
    let Some(host) = device("world write first step") else {
        return;
    };
    let mut steps = Vec::new();
    for count in [1, 4096, 8192, 16384] {
        let mut world = majority_world(&host, 1024);
        world.step(3).unwrap();
        let mut done = 0;
        while done < count {
            let part = (count - done).min(CELLS);
            let moved = written(&mut world, &row(0x57E9 ^ count as u64, done, part));
            bounded(&format!("{count} at {done}"), part, moved);
            done += part;
        }
        let before = world.traffic();
        world.step(1).unwrap();
        let step = world.traffic().since(before);
        println!("first step after {count}: {step:?}");
        steps.push(step);
    }
    for step in &steps {
        assert_eq!(*step, steps[0], "{steps:?}");
        assert_eq!(step.uploaded, 0, "{steps:?}");
        assert_eq!(step.woken, 0, "{steps:?}");
    }
}

mod library {
    use super::{Law, MAJORITY, MIX, Made, World, device, edges, scattered, words_hash};

    fn world<'host>(
        host: &'host grit::Host,
        law: &Law,
        params: &[u32],
        size: [u32; 3],
        sparse: bool,
    ) -> World<'host> {
        World::create_law(
            host,
            law,
            &Made {
                rule: law.name(),
                params,
                size,
                sparse,
                slots: None,
            },
        )
        .unwrap()
    }

    fn played(world: &mut World<'_>, size: [u32; 3], mask: u32, seed: u64) -> Vec<String> {
        let mut seen = Vec::new();
        let note = |world: &mut World<'_>, seen: &mut Vec<String>| {
            let listed = if world.pool().is_some() {
                0
            } else {
                world.listed().unwrap()
            };
            let cells = world.cells().unwrap();
            seen.push(format!("{listed}:{}", words_hash(&cells)));
        };
        world
            .write_plane(0, &scattered(seed, size, 700, mask))
            .unwrap();
        note(world, &mut seen);
        world.step(1).unwrap();
        note(world, &mut seen);
        world.step(2).unwrap();
        note(world, &mut seen);
        world
            .write_plane(0, &scattered(seed ^ 1, size, 300, mask))
            .unwrap();
        note(world, &mut seen);
        world.write_plane(0, &edges(size, mask)).unwrap();
        note(world, &mut seen);
        world.step(1).unwrap();
        note(world, &mut seen);
        world.step(3).unwrap();
        note(world, &mut seen);
        world
            .write_plane(0, &scattered(seed ^ 2, size, 40, mask))
            .unwrap();
        note(world, &mut seen);
        if world.planes() > 1 {
            world
                .write_plane(1, &scattered(seed ^ 3, size, 200, mask))
                .unwrap();
            note(world, &mut seen);
        }
        world.step(2).unwrap();
        note(world, &mut seen);
        seen
    }

    pub fn trace(host: &grit::Host, case: &str) -> Vec<String> {
        match case {
            "majority-256" => {
                let law = Law::compile(MAJORITY).unwrap();
                let size = [256, 256, 2];
                played(
                    &mut world(host, &law, &[], size, false),
                    size,
                    1,
                    0x5EED_0256,
                )
            }
            "majority-64x4" => {
                let law = Law::compile(MAJORITY).unwrap();
                let size = [64, 64, 4];
                played(
                    &mut world(host, &law, &[], size, false),
                    size,
                    3,
                    0x5EED_0064,
                )
            }
            "heat-32" => {
                let law = Law::compile(super::HEAT).unwrap();
                let size = [32, 32, 32];
                played(
                    &mut world(host, &law, &[1, 2, 3], size, false),
                    size,
                    7,
                    0x5EED_0032,
                )
            }
            "mix-sparse-64" => {
                let law = Law::compile(MIX).unwrap();
                let size = [64, 64, 32];
                played(
                    &mut world(host, &law, &[1, 2], size, true),
                    size,
                    15,
                    0x5EED_5064,
                )
            }
            other => panic!("no case {other}"),
        }
    }

    #[test]
    fn library_writes_read_back_as_on_the_recorded_base() {
        let Some(host) = device("world write library") else {
            return;
        };
        let got: Vec<(&str, Vec<String>)> = super::recorded::LIBRARY
            .iter()
            .map(|(case, _)| (*case, trace(&host, case)))
            .collect();
        for (case, trace) in &got {
            println!("RECORD library {case} {}", words_hash_of(trace));
        }
        for ((case, trace), (_, want)) in got.iter().zip(super::recorded::LIBRARY) {
            assert_eq!(&words_hash_of(trace), want, "{case}: {trace:?}");
        }
    }

    pub fn words_hash_of(trace: &[String]) -> String {
        super::hex(&grit::branch::sha256(trace.join("\n").as_bytes()))
    }
}

mod recorded {
    pub const LIBRARY: [(&str, &str); 4] = [
        (
            "majority-256",
            "a9479ca5b2b4934cde01452a495030eeb9bb22cf87d76dcca48806de2efa1bc9",
        ),
        (
            "majority-64x4",
            "16013d344ff05714a047e7b65584ef27f44c361a9e5ebb3fe7ebda7941dab524",
        ),
        (
            "heat-32",
            "9ad94013d105a4e14334ba00dcee95fd4669cce945a9884353367d29a123f537",
        ),
        (
            "mix-sparse-64",
            "42b09c0f476c2fa3a540de1afa5e078ea03e218d03ed5957a1ef0544ade99868",
        ),
    ];

    pub const SERVED: [(&str, &str, &str); 3] = [
        (
            "majority-64x4",
            "b674ddfd457a28c39dc2db24d270316dd0c7211b0534eb598b5fbdeb5d3da801",
            "053c93aa149a8b785b5799e846fd47636f5976855597431e5e8f06ea72a51299",
        ),
        (
            "heat-32",
            "90fe02e074dec9827deaa20abaa2a040b56d7f8ad1fb9da092c16be1ec5a2d6b",
            "bee6c559d426bb931840705086ba674c52c758aad14b2838bea6b2ffdbfe08c4",
        ),
        (
            "mix-sparse-64",
            "44373b15b32b08e2eba5178218fc0b5d6fea6e9c852a020c669bc3aa6b43efd0",
            "",
        ),
    ];
}

#[cfg(feature = "faults")]
mod served {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use grit::Host;
    use grit::clock::{Fake, Time};
    use grit::serve3::{Options, Server};
    use grit::wire::{Json, written};

    use super::{HEAT, MAJORITY, MIX, device, edges, hex, scattered, words_hash};

    pub struct Scratch(PathBuf);

    impl Scratch {
        pub fn new(label: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "grit-world-write-{label}-{}-{nanos}",
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

    fn line(name: &str, arguments: &str) -> String {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        )
    }

    pub fn fine(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let got = server.heard(&line(name, arguments)).unwrap();
        let result = got.get("result").unwrap();
        assert!(
            result.get("isError") != Some(&Json::Bool(true)),
            "{name}: {}",
            written(&got)
        );
        result.get("structuredContent").cloned().unwrap()
    }

    pub fn timed<'host>(host: &'host Host, dir: Option<&Path>, fake: &Arc<Fake>) -> Server<'host> {
        let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
        let options = Options {
            dir: dir.map(Path::to_path_buf),
            memory: None,
        };
        Server::timed(Some(host), &options, time).unwrap()
    }

    pub fn cells_text(cells: &[([u32; 3], u32)]) -> String {
        let listed: Vec<String> = cells
            .iter()
            .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
            .collect();
        format!("[{}]", listed.join(","))
    }

    pub fn write(
        server: &mut Server<'_>,
        world: i64,
        plane: u32,
        cells: &[([u32; 3], u32)],
    ) -> Json {
        fine(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},\"plane\":{plane},\"cells\":{},\"budget_ms\":600000}}",
                cells_text(cells)
            ),
        )
    }

    pub fn step(server: &mut Server<'_>, world: i64, frames: u32) -> Json {
        fine(
            server,
            "grit_world_step",
            &format!("{{\"world\":{world},\"frames\":{frames},\"budget_ms\":600000}}"),
        )
    }

    fn read_all(server: &mut Server<'_>, world: i64, size: [u32; 3], planes: u32) -> String {
        let per = (4096 / (size[0] * size[1])).max(1);
        let mut words = Vec::new();
        for plane in 0..planes {
            let mut z = 0;
            while z < size[2] {
                let top = (z + per).min(size[2]) - 1;
                let got = fine(
                    server,
                    "grit_world_read",
                    &format!(
                        "{{\"world\":{world},\"plane\":{plane},\"from\":[0,0,{z}],\"to\":[{},{},{top}]}}",
                        size[0] - 1,
                        size[1] - 1
                    ),
                );
                for cell in got.get("cells").and_then(Json::list).unwrap() {
                    words.push(u32::try_from(cell.int().unwrap()).unwrap());
                }
                z = top + 1;
            }
        }
        words_hash(&words)
    }

    struct Case {
        name: &'static str,
        law: &'static str,
        params: &'static str,
        size: [u32; 3],
        sparse: bool,
        mask: u32,
        planes: u32,
        seed: u64,
    }

    const CASES: [Case; 3] = [
        Case {
            name: "majority-64x4",
            law: MAJORITY,
            params: "[]",
            size: [64, 64, 4],
            sparse: false,
            mask: 3,
            planes: 2,
            seed: 0xC0DE_0064,
        },
        Case {
            name: "heat-32",
            law: HEAT,
            params: "[1,2,3]",
            size: [32, 32, 32],
            sparse: false,
            mask: 7,
            planes: 1,
            seed: 0xC0DE_0032,
        },
        Case {
            name: "mix-sparse-64",
            law: MIX,
            params: "[1,2]",
            size: [64, 64, 32],
            sparse: true,
            mask: 15,
            planes: 1,
            seed: 0xC0DE_5064,
        },
    ];

    const STEADY: [&str; 7] = [
        "world", "from", "name", "written", "frame", "frames", "tally",
    ];

    fn steady(answer: &Json, sparse: bool) -> String {
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

    fn quoted(law: &str) -> String {
        written(&Json::Text(law.to_owned()))
    }

    fn played(server: &mut Server<'_>, case: &Case) -> (i64, Vec<String>) {
        let made = fine(
            server,
            "grit_world_create",
            &format!(
                "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{},\"sparse\":{},\"budget_ms\":600000}}",
                case.size[0],
                case.size[1],
                case.size[2],
                quoted(case.law),
                case.params,
                case.sparse
            ),
        );
        let world = made.get("world").and_then(Json::int).unwrap();
        let mut answers = vec![steady(&made, case.sparse)];
        let size = case.size;
        let mask = case.mask;
        answers.push(steady(
            &write(server, world, 0, &scattered(case.seed, size, 600, mask)),
            case.sparse,
        ));
        answers.push(steady(
            &write(server, world, 0, &scattered(case.seed ^ 1, size, 250, mask)),
            case.sparse,
        ));
        let fork = fine(server, "grit_world_fork", &format!("{{\"world\":{world}}}"));
        let forked = fork.get("world").and_then(Json::int).unwrap();
        answers.push(steady(&fork, case.sparse));
        answers.push(steady(&step(server, world, 1), case.sparse));
        answers.push(steady(&step(server, world, 2), case.sparse));
        answers.push(steady(
            &write(server, world, 0, &edges(size, mask)),
            case.sparse,
        ));
        if case.planes > 1 {
            answers.push(steady(
                &write(server, world, 1, &scattered(case.seed ^ 2, size, 150, mask)),
                case.sparse,
            ));
        }
        answers.push(steady(&step(server, world, 3), case.sparse));
        answers.push(steady(
            &write(server, world, 0, &scattered(case.seed ^ 3, size, 30, mask)),
            case.sparse,
        ));
        answers.push(steady(&step(server, world, 1), case.sparse));
        answers.push(read_all(server, world, size, case.planes));
        answers.push(steady(
            &fine(
                server,
                "grit_world_restore",
                &format!("{{\"world\":{forked},\"from\":{world}}}"),
            ),
            case.sparse,
        ));
        answers.push(steady(
            &write(server, forked, 0, &scattered(case.seed ^ 4, size, 90, mask)),
            case.sparse,
        ));
        answers.push(steady(&step(server, forked, 2), case.sparse));
        answers.push(read_all(server, forked, size, case.planes));
        answers.push(steady(
            &fine(
                server,
                "grit_world_save",
                &format!("{{\"world\":{forked},\"name\":\"restored\"}}"),
            ),
            case.sparse,
        ));
        let opened = fine(server, "grit_world_open", "{\"name\":\"restored\"}");
        let reopened = opened.get("world").and_then(Json::int).unwrap();
        answers.push(steady(&opened, case.sparse));
        answers.push(steady(
            &write(
                server,
                reopened,
                0,
                &scattered(case.seed ^ 5, size, 60, mask),
            ),
            case.sparse,
        ));
        answers.push(steady(&step(server, reopened, 1), case.sparse));
        answers.push(read_all(server, reopened, size, case.planes));
        answers.push(read_all(server, world, size, case.planes));
        (world, answers)
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

    fn journal_hash(dir: &Path) -> String {
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
            println!("JOURNAL {name} {}", std::fs::metadata(&path).unwrap().len());
            joined.extend_from_slice(name.as_bytes());
            joined.push(0);
            joined.extend_from_slice(&std::fs::read(&path).unwrap());
            joined.push(0);
        }
        hex(&grit::branch::sha256(&joined))
    }

    pub fn trace(host: &Host, name: &str) -> (String, String) {
        let case = CASES.iter().find(|case| case.name == name).unwrap();
        let scratch = Scratch::new(name);
        let dir = scratch.dir();
        let (world, answers, journal) = {
            let fake = Arc::new(Fake::new());
            let mut server = timed(host, Some(&dir), &fake);
            let (world, answers) = played(&mut server, case);
            drop(server);
            (world, answers, journal_hash(&dir))
        };
        let fake = Arc::new(Fake::new());
        let mut again = timed(host, Some(&dir), &fake);
        let opened = fine(
            &mut again,
            "grit_world_open",
            &format!("{{\"name\":\"world-{world}\",\"budget_ms\":600000}}"),
        );
        let reopened = opened.get("world").and_then(Json::int).unwrap();
        let opened = steady(&opened, case.sparse);
        let replayed = read_all(&mut again, reopened, case.size, case.planes);
        assert_eq!(
            Some(&replayed),
            answers.last(),
            "{name}: the world replayed from its journal reads back otherwise"
        );
        let mut both = answers;
        both.push(opened);
        both.push(replayed);
        (
            hex(&grit::branch::sha256(both.join("\n").as_bytes())),
            journal,
        )
    }

    #[test]
    fn the_first_served_step_after_writes_does_the_same_work_however_many_were_written() {
        let Some(host) = device("world write served first step") else {
            return;
        };
        let side = 1024_u32;
        grit::serve3::tools::faults::stall_cells(std::time::Duration::from_micros(1));
        let mut steps = Vec::new();
        for count in [1_usize, 4096, 8192, 16384] {
            let scratch = Scratch::new("first-step");
            let dir = scratch.dir();
            let fake = Arc::new(Fake::new());
            let mut server = timed(&host, Some(&dir), &fake);
            let made = fine(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{side},\"deep\":{side},\"tall\":2,\"law\":{},\"params\":[],\"budget_ms\":600000}}",
                    quoted(MAJORITY)
                ),
            );
            let world = made.get("world").and_then(Json::int).unwrap();
            let number = u32::try_from(world).unwrap();
            step(&mut server, world, 3);
            let mut done = 0;
            while done < count {
                let part = (count - done).min(4096);
                let before = server.traffic(number).unwrap();
                let told = write(
                    &mut server,
                    world,
                    0,
                    &super::row(0xF5 ^ count as u64, done, part),
                );
                assert_eq!(told.get("written").and_then(Json::int), Some(part as i64));
                let moved = server.traffic(number).unwrap().since(before);
                assert!(
                    moved.submits <= 3 * part as u64 + 3,
                    "{count} at {done}: {moved:?}"
                );
                assert!(
                    moved.woken <= 54 * part as u64,
                    "{count} at {done}: {moved:?}"
                );
                done += part;
            }
            let before = server.traffic(number).unwrap();
            let told = step(&mut server, world, 1);
            assert_eq!(told.get("frames").and_then(Json::int), Some(1));
            let moved = server.traffic(number).unwrap().since(before);
            println!("served first step after {count}: {moved:?}");
            steps.push(moved);
        }
        grit::serve3::tools::faults::disarm();
        for moved in &steps {
            assert_eq!(*moved, steps[0], "{steps:?}");
            assert_eq!(moved.uploaded, 0, "{steps:?}");
        }
    }

    #[test]
    fn served_writes_answer_and_journal_as_on_the_recorded_base() {
        let Some(host) = device("world write served") else {
            return;
        };
        let got: Vec<(String, String)> = super::recorded::SERVED
            .iter()
            .map(|(name, _, _)| trace(&host, name))
            .collect();
        for ((name, _, _), (answers, journal)) in super::recorded::SERVED.iter().zip(&got) {
            println!("RECORD served {name} {answers} {journal}");
        }
        for ((name, want_answers, want_journal), (answers, journal)) in
            super::recorded::SERVED.iter().zip(&got)
        {
            assert_eq!(answers, want_answers, "{name}: answers");
            if !want_journal.is_empty() {
                assert_eq!(journal, want_journal, "{name}: journal");
            }
        }
    }
}
