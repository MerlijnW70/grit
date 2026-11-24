use std::sync::Arc;

use grit::Host;
use grit::clock::{Fake, Time};
use grit::serve3::{Options, Server};

use super::pinned::{bays, life};
use super::served::{
    BAYS_TABLES, LIFE_TABLES, Scratch, every_plane, hex, int_at, journal_hash, learned, ok, quoted,
    soup_cells, steady, step, words_hash, write,
};

pub struct Case {
    pub name: &'static str,
    pub size: [u32; 3],
    pub sparse: bool,
    pub planes: u32,
}

pub const CASES: [Case; 4] = [
    Case {
        name: "life-dense-64x64x2",
        size: [64, 64, 2],
        sparse: false,
        planes: 2,
    },
    Case {
        name: "life-parallel-dense-64x64x2",
        size: [64, 64, 2],
        sparse: false,
        planes: 2,
    },
    Case {
        name: "bays-sparse-64",
        size: [64, 64, 64],
        sparse: true,
        planes: 2,
    },
    Case {
        name: "bays-parallel-dense-32",
        size: [32, 32, 32],
        sparse: false,
        planes: 2,
    },
];

pub fn timed<'host>(host: &'host Host, dir: &std::path::Path, fake: &Arc<Fake>) -> Server<'host> {
    let time: Arc<dyn Time> = Arc::clone(fake) as Arc<dyn Time>;
    let options = Options {
        dir: Some(dir.to_path_buf()),
        memory: None,
    };
    Server::timed(Some(host), &options, time).unwrap()
}

fn law_of(server: &mut Server<'_>, name: &str) -> String {
    match name {
        "life-dense-64x64x2" => learned(server, &life(), false, LIFE_TABLES),
        "life-parallel-dense-64x64x2" => learned(server, &life(), true, LIFE_TABLES),
        "bays-sparse-64" => learned(server, &bays(), false, BAYS_TABLES),
        _ => learned(server, &bays(), true, BAYS_TABLES),
    }
}

fn start(case: &Case, seed: u64) -> Vec<([u32; 3], u32)> {
    let parallel = case.name.contains("parallel");
    let word = move |rng: &mut super::served::Rng| {
        if parallel {
            u32::try_from(rng.next() & 0xFFFF_FFFF).unwrap()
        } else {
            1
        }
    };
    let size = case.size;
    let sides = [
        (size[0] / 2).min(24),
        (size[1] / 2).min(24),
        if size[2] == 2 { 2 } else { size[2] / 4 },
    ];
    let from = [
        size[0] / 4,
        size[1] / 4,
        if size[2] == 2 { 0 } else { size[2] / 3 },
    ];
    soup_cells(seed, from, sides, 320, word)
}

pub fn played(server: &mut Server<'_>, case: &Case, more: &str) -> (i64, Vec<String>) {
    let law = law_of(server, case.name);
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"alternate\":[0,1],\"sparse\":{}{more},\"budget_ms\":600000}}",
            case.size[0],
            case.size[1],
            case.size[2],
            quoted(&law),
            case.sparse
        ),
    );
    let world = int_at(&made, "world");
    let size = case.size;
    let sparse = case.sparse;
    let mut answers = vec![steady(&made, sparse)];
    answers.push(steady(
        &write(server, world, 0, &start(case, 0xB0D1)),
        sparse,
    ));
    let stepped = step(server, world, 9);
    let current = u32::try_from(int_at(&stepped, "current")).unwrap();
    answers.push(steady(&stepped, sparse));
    let fork = ok(server, "grit_world_fork", &format!("{{\"world\":{world}}}"));
    let forked = int_at(&fork, "world");
    answers.push(steady(&fork, sparse));
    answers.push(steady(
        &write(server, world, current, &start(case, 0xB0D2)),
        sparse,
    ));
    answers.push(steady(&step(server, world, 4), sparse));
    answers.push(steady(&step(server, forked, 3), sparse));
    answers.push(words_hash(&every_plane(server, world, size, case.planes)));
    answers.push(words_hash(&every_plane(server, forked, size, case.planes)));
    answers.push(steady(
        &ok(
            server,
            "grit_world_restore",
            &format!("{{\"world\":{forked},\"from\":{world}}}"),
        ),
        sparse,
    ));
    answers.push(steady(&step(server, forked, 2), sparse));
    answers.push(steady(
        &ok(
            server,
            "grit_world_save",
            &format!("{{\"world\":{forked},\"name\":\"pinned\"}}"),
        ),
        sparse,
    ));
    let opened = ok(server, "grit_world_open", "{\"name\":\"pinned\"}");
    let reopened = int_at(&opened, "world");
    answers.push(steady(&opened, sparse));
    answers.push(steady(&step(server, reopened, 5), sparse));
    answers.push(words_hash(&every_plane(
        server,
        reopened,
        size,
        case.planes,
    )));
    answers.push(steady(&step(server, world, 6), sparse));
    answers.push(words_hash(&every_plane(server, world, size, case.planes)));
    (world, answers)
}

pub fn trace(host: &Host, case: &Case) -> (String, String) {
    let scratch = Scratch::new(case.name);
    let dir = scratch.dir();
    let (world, answers, journal) = {
        let fake = Arc::new(Fake::new());
        let mut server = timed(host, &dir, &fake);
        let (world, answers) = played(&mut server, case, "");
        drop(server);
        (world, answers, journal_hash(&dir))
    };
    let fake = Arc::new(Fake::new());
    let mut again = timed(host, &dir, &fake);
    let opened = ok(
        &mut again,
        "grit_world_open",
        &format!("{{\"name\":\"world-{world}\",\"budget_ms\":600000}}"),
    );
    let reopened = int_at(&opened, "world");
    let replayed = words_hash(&every_plane(&mut again, reopened, case.size, case.planes));
    assert_eq!(
        Some(&replayed),
        answers.last(),
        "{}: the world replayed from its journal reads back otherwise",
        case.name
    );
    let mut both = answers;
    both.push(steady(&opened, case.sparse));
    both.push(replayed);
    (
        hex(&grit::branch::sha256(both.join("\n").as_bytes())),
        journal,
    )
}
