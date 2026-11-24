#![allow(dead_code)]

use grit::serve3::Server;
use grit::wire::{Json, written};

use super::pinned::LONG;
use super::served::{
    Rng, box_words, int_at, listed, ok, point, quoted, set_of, step, text_at, words_hash, write,
};

#[derive(Clone)]
pub struct Case {
    pub label: String,
    pub law: String,
    pub alternate: bool,
    pub params: Vec<u32>,
    pub size: [u32; 3],
    pub sparse: bool,
    pub absorb: Option<[u32; 3]>,
    pub zero: bool,
    pub steps: u32,
    pub planes: u32,
    pub sides: Option<(u32, u32)>,
    pub frac: Option<u32>,
}

impl Case {
    pub fn plane(&self, reads: bool, at: u32) -> u32 {
        match self.sides {
            Some((read, wrote)) => {
                if reads {
                    read
                } else {
                    wrote
                }
            }
            None => at & 1,
        }
    }

    pub fn world_size(&self) -> ([u32; 3], [u32; 3], Option<[u32; 3]>) {
        let mut size = self.size;
        let mut origin = [0; 3];
        let mut shell = self.absorb.unwrap_or([0; 3]);
        if self.zero {
            for axis in 0..3 {
                if self.size[axis] > 2 {
                    size[axis] = self.size[axis] * 2;
                    origin[axis] = self.size[axis] / 2;
                    shell[axis] += self.size[axis] / 2;
                }
            }
        }
        let absorb = if shell == [0; 3] { None } else { Some(shell) };
        (size, origin, absorb)
    }
}

pub struct Long {
    pub states: Vec<(u32, Vec<u32>)>,
    pub end: Vec<u32>,
    pub fingerprint: String,
    pub calls: usize,
}

pub fn long_way(server: &mut Server<'_>, case: &Case, start: &[u32], stops: &[u32]) -> Long {
    let (size, origin, absorb) = case.world_size();
    let mut calls = 0;
    let mut extra = String::new();
    if case.alternate {
        extra.push_str(",\"alternate\":[0,1]");
    }
    if case.sparse {
        extra.push_str(",\"sparse\":true");
    }
    if let Some(shell) = absorb {
        extra.push_str(&format!(",\"absorb\":{}", listed(&shell)));
    }
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{}{extra},\"budget_ms\":{LONG}}}",
            size[0],
            size[1],
            size[2],
            quoted(&case.law),
            listed(&case.params)
        ),
    );
    calls += 1;
    let world = int_at(&made, "world");
    let cells: Vec<([u32; 3], u32)> = set_of(case.size, start)
        .into_iter()
        .map(|(at, word)| ([0, 1, 2].map(|axis| at[axis] + origin[axis]), word))
        .collect();
    calls += cells.len().div_ceil(4096);
    write(server, world, case.plane(true, 0), &cells);
    let mut wanted: Vec<u32> = stops.to_vec();
    wanted.push(case.steps);
    wanted.sort_unstable();
    wanted.dedup();
    let mut now = 0;
    let mut states = Vec::new();
    let per = 4096 / case.size[0];
    let reads_of = |sides: [u32; 3]| (sides[1].div_ceil(per.max(1)) * sides[2]) as usize;
    for at in wanted {
        if at > now {
            step(server, world, at - now);
            calls += 1;
            now = at;
        }
        states.push((
            at,
            box_words(server, world, case.plane(false, at), origin, case.size),
        ));
        calls += reads_of(case.size);
    }
    let mut every = Vec::new();
    for plane in 0..case.planes {
        every.extend(box_words(server, world, plane, origin, case.size));
        calls += reads_of(case.size);
    }
    let end = states
        .iter()
        .find(|(at, _)| *at == case.steps)
        .map(|(_, words)| words.clone())
        .unwrap();
    ok(
        server,
        "grit_world_close",
        &format!("{{\"world\":{world}}}"),
    );
    calls += 1;
    Long {
        states,
        end,
        fingerprint: words_hash(&every),
        calls,
    }
}

pub struct Given {
    pub start: Vec<u32>,
    pub targets: Vec<(u32, Vec<u32>)>,
}

pub fn arguments(case: &Case, given: &[Given], more: &str) -> String {
    let mut text = format!(
        "{{\"law\":{},\"params\":{},\"size\":{},\"frames\":{}",
        quoted(&case.law),
        listed(&case.params),
        listed(&case.size),
        case.steps
    );
    if case.alternate {
        text.push_str(",\"alternate\":[0,1]");
    }
    if case.sparse {
        text.push_str(",\"sparse\":true");
    }
    if let Some(shell) = case.absorb {
        text.push_str(&format!(",\"absorb\":{}", listed(&shell)));
    }
    if case.zero {
        text.push_str(",\"border\":\"zero\"");
    }
    if let Some((read, wrote)) = case.sides {
        text.push_str(&format!(",\"reads\":{read},\"writes\":{wrote}"));
    }
    if let Some(frac) = case.frac {
        text.push_str(&format!(",\"values\":\"real\",\"frac\":{frac}"));
    }
    let starts: Vec<String> = given
        .iter()
        .map(|one| {
            let targets: Vec<String> = one
                .targets
                .iter()
                .map(|(at, words)| format!("{{\"frame\":{at},\"dense\":{}}}", listed(words)))
                .collect();
            format!(
                "{{\"dense\":{},\"targets\":[{}]}}",
                listed(&one.start),
                targets.join(",")
            )
        })
        .collect();
    text.push_str(&format!(
        ",\"starts\":[{}]{more},\"budget_ms\":{LONG}}}",
        starts.join(",")
    ));
    text
}

pub fn returned(
    server: &mut Server<'_>,
    arguments: &str,
    first: &Json,
) -> Vec<(u32, [u32; 3], u32)> {
    let mut cells = Vec::new();
    let pages = int_at(first.get("return").unwrap(), "pages");
    for page in 0..pages {
        let got = if page == 0 {
            first.clone()
        } else {
            let asked = arguments.replacen(
                ",\"return\":true",
                &format!(",\"return\":true,\"page\":{page}"),
                1,
            );
            ok(server, "grit_law_check", &asked)
        };
        let shown = got.get("return").unwrap();
        assert_eq!(int_at(shown, "page"), page);
        for cell in shown.get("cells").and_then(Json::list).unwrap() {
            let held: Vec<u32> = cell
                .list()
                .unwrap()
                .iter()
                .map(|one| u32::try_from(one.int().unwrap()).unwrap())
                .collect();
            cells.push((held[0], [held[1], held[2], held[3]], held[4]));
        }
    }
    cells
}

pub fn scientific(value: f64) -> String {
    format!("{value:.9e}")
}

pub fn host_score(
    size: [u32; 3],
    got: &[u32],
    want: &[u32],
    frac: Option<u32>,
) -> Vec<(String, String)> {
    let mut wrong = 0_u64;
    let mut earliest = None;
    let mut squares = 0_u128;
    let mut worst = 0_u64;
    for (at, (one, other)) in got.iter().zip(want).enumerate() {
        if one == other {
            continue;
        }
        wrong += 1;
        if earliest.is_none() {
            earliest = Some((at, *other, *one));
        }
        let gap = i128::from(*one as i32) - i128::from(*other as i32);
        squares += gap.unsigned_abs() * gap.unsigned_abs();
        worst = worst.max(gap.unsigned_abs() as u64);
    }
    let cells = got.len() as u64;
    let mut shown = vec![
        ("cells".to_owned(), cells.to_string()),
        ("right".to_owned(), (cells - wrong).to_string()),
        ("wrong".to_owned(), wrong.to_string()),
    ];
    if let Some((at, want, got)) = earliest {
        let spot = point(size, at);
        shown.push((
            "first_wrong".to_owned(),
            format!(
                "{{\"at\":[{},{},{}],\"want\":{want},\"got\":{got}}}",
                spot[0], spot[1], spot[2]
            ),
        ));
    }
    if let Some(frac) = frac {
        let unit = f64::from(1_u32 << frac);
        let rms = (squares as f64 / cells as f64).sqrt();
        shown.extend([
            ("max".to_owned(), worst.to_string()),
            ("sum_squares".to_owned(), format!("\"{squares}\"")),
            ("rms".to_owned(), format!("\"{}\"", scientific(rms))),
            (
                "max_real".to_owned(),
                format!("\"{}\"", scientific(worst as f64 / unit)),
            ),
            (
                "rms_real".to_owned(),
                format!("\"{}\"", scientific(rms / unit)),
            ),
        ]);
    }
    shown
}

pub fn assert_score(label: &str, shown: &Json, wanted: &[(String, String)]) {
    for (key, value) in wanted {
        let got = shown
            .get(key)
            .map(written)
            .unwrap_or_else(|| panic!("{label}: no {key} in {}", written(shown)));
        assert_eq!(&got, value, "{label}: {key} in {}", written(shown));
    }
    if !wanted.iter().any(|(key, _)| key == "first_wrong") {
        assert!(
            shown.get("first_wrong").is_none(),
            "{label}: {}",
            written(shown)
        );
    }
}

pub fn flipped(words: &[u32], rng: &mut Rng, many: usize, last: bool, real: bool) -> Vec<u32> {
    let mut hit = std::collections::BTreeSet::new();
    while hit.len() < many.min(words.len()) {
        hit.insert(rng.below(words.len() as u64) as usize);
    }
    if last {
        hit.insert(words.len() - 1);
    }
    let mut out = words.to_vec();
    for at in hit {
        out[at] = if real {
            out[at].wrapping_add(1 + rng.below(4000) as u32)
        } else {
            out[at] ^ 1
        };
    }
    out
}

pub struct Checked {
    pub answer: Json,
    pub long: Vec<Long>,
}

pub fn compared(server: &mut Server<'_>, case: &Case, starts: &[Vec<u32>], seed: u64) -> Checked {
    let mut rng = Rng(seed);
    let half = (case.steps / 2).max(1);
    let stops = [0, half, case.steps];
    let mut longs = Vec::new();
    let mut given = Vec::new();
    for start in starts {
        let long = long_way(server, case, start, &stops);
        let mut targets = Vec::new();
        for (at, words) in &long.states {
            let many = rng.below(5) as usize;
            let last = rng.below(2) == 0;
            let target = flipped(words, &mut rng, many, last, case.frac.is_some());
            targets.push((*at, target));
        }
        targets.reverse();
        given.push(Given {
            start: start.clone(),
            targets,
        });
        longs.push(long);
    }
    let asked = arguments(case, &given, ",\"return\":true");
    let answer = ok(server, "grit_law_check", &asked);
    let shown_starts = answer.get("starts").and_then(Json::list).unwrap();
    assert_eq!(shown_starts.len(), starts.len(), "{}", case.label);
    let mut wanted_cells = Vec::new();
    for (at, ((shown, long), one)) in shown_starts.iter().zip(&longs).zip(&given).enumerate() {
        let label = format!("{} start {at}", case.label);
        assert_eq!(int_at(shown, "start"), at as i64);
        assert_eq!(text_at(shown, "fingerprint"), long.fingerprint, "{label}");
        let shown_targets = shown.get("targets").and_then(Json::list).unwrap();
        assert_eq!(shown_targets.len(), one.targets.len(), "{label}");
        for ((when, target), score) in one.targets.iter().zip(shown_targets) {
            assert_eq!(int_at(score, "frame"), i64::from(*when), "{label}");
            let state = &long.states.iter().find(|(at, _)| at == when).unwrap().1;
            assert_score(
                &format!("{label} frame {when}"),
                score,
                &host_score(case.size, state, target, case.frac),
            );
        }
        for (spot, word) in set_of(case.size, &long.end) {
            wanted_cells.push((at as u32, spot, word));
        }
    }
    let got_cells = returned(server, &asked, &answer);
    assert_eq!(
        got_cells.len(),
        wanted_cells.len(),
        "{}: returned cells",
        case.label
    );
    assert!(
        got_cells == wanted_cells,
        "{}: returned cells differ",
        case.label
    );
    Checked {
        answer,
        long: longs,
    }
}
