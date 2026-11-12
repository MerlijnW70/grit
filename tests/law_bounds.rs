#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use grit::law::value::Value;
use grit::law::{Code, Law, Refusal, limits, parse};

const STACK: usize = 2 << 20;

fn law(params: u32, defs: &str, step: &str) -> String {
    format!(
        "{{\"law\":1,\"name\":\"probe\",\"params\":{params},\"channels\":1,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{{defs}}},\"step\":{step}}}"
    )
}

fn probe(step: &str) -> String {
    law(0, "", step)
}

fn junk(expression: &str) -> String {
    format!("[\"let\",[[\"junk\",{expression}]],[\"vec\",\"cells\",[\"vec\",0]]]")
}

fn on_a_test_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(work)
        .unwrap()
        .join()
        .unwrap()
}

fn timed(text: &str) -> (Result<Law, Refusal>, Duration) {
    let start = Instant::now();
    let made = Law::compile(text);
    (made, start.elapsed())
}

fn refused(text: &str) -> Refusal {
    match Law::compile(text) {
        Ok(law) => panic!("compiled into {} nodes, work {}", law.nodes(), law.work()),
        Err(why) => why,
    }
}

const FLAGS: &str =
    "[\"fold\",\"v\",[\"vec\",[\"eq\",0,0]],\"i\",[\"range\",17],[\"concat\",\"v\",\"v\"]]";

fn counted(rounds: u32) -> String {
    probe(&format!(
        "[\"let\",[[\"xs\",{FLAGS}],[\"junk\",[\"fold\",\"acc\",0,\"i\",[\"range\",{rounds}],[\"count\",\"xs\"]]]],[\"vec\",\"cells\",[\"vec\",0]]]"
    ))
}

#[test]
fn count_charges_work_for_every_item_it_visits() {
    let text = counted(250_000);
    let (made, took) = on_a_test_thread(move || timed(&text));
    let why = made.unwrap_err();
    println!("{why} in {took:?}");
    assert_eq!(
        (why.code, why.path.as_str()),
        (Code::Work, "$.step[1][1][1][5]"),
        "{why}"
    );
    assert_eq!(why.limit, Some(limits::WORK));
    assert!(took < Duration::from_secs(2), "{took:?}");
    let why = refused(&counted(2_000));
    assert_eq!(
        (why.code, why.limit),
        (Code::Work, Some(limits::WORK)),
        "{why}"
    );
    let one = Law::compile(&counted(1)).unwrap().work();
    let two = Law::compile(&counted(2)).unwrap().work();
    assert!(
        two - one > 1 << 17,
        "one more count of 2^17 flags cost {}",
        two - one
    );
    let reviewed = probe(
        "[\"let\",[[\"xs\",[\"range\",200000]],[\"junk\",[\"fold\",\"acc\",0,\"i\",[\"range\",250000],[\"count\",\"xs\"]]]],[\"vec\",\"cells\",[\"vec\",0]]]",
    );
    let (made, took) = on_a_test_thread(move || timed(&reviewed));
    let why = made.unwrap_err();
    assert_eq!(
        (why.code, why.path.as_str()),
        (Code::Type, "$.step[1][1][1][5][1]"),
        "{why}"
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
}

fn nested_step(rounds: u32) -> String {
    format!(
        "[\"let\",[[\"deep\",[\"fold\",\"acc\",[\"vec\"],\"i\",[\"range\",{rounds}],[\"vec\",\"acc\"]]]],[\"vec\",\"cells\",[\"vec\",0]]]"
    )
}

fn nested(rounds: u32) -> String {
    probe(&nested_step(rounds))
}

#[test]
fn a_value_nested_past_its_limit_is_refused_by_name_on_a_test_thread() {
    let deep = nested(240_000);
    assert_eq!(nested_step(240_000).len(), 102);
    let why = on_a_test_thread(move || Law::compile(&deep).map(|law| law.work()).unwrap_err());
    println!("{why}");
    assert_eq!(
        (why.code, why.path.as_str(), why.limit),
        (
            Code::ValueDepth,
            "$.step[1][0][1][5]",
            Some(limits::VALUES as u64)
        ),
        "{why}"
    );
    assert_eq!(why.code.name(), "value_depth");
    assert!(
        limits::ALL.contains(&("value_depth", limits::VALUES as u64)),
        "{:?}",
        limits::ALL
    );
    let most = (limits::VALUES - 1) as u32;
    let at = on_a_test_thread(move || Law::compile(&nested(most)).map(|law| law.nodes()));
    assert!(at.is_ok(), "{at:?}");
    let past = on_a_test_thread(move || Law::compile(&nested(most + 1)).unwrap_err());
    assert_eq!(past.code, Code::ValueDepth, "{past}");
    assert_eq!(
        past.found,
        format!("vecs nested {} deep", limits::VALUES + 1)
    );
    for (name, body) in [
        ("put", "[\"put\",[\"vec\",\"acc\"],0,\"acc\"]"),
        ("map", "[\"map\",\"x\",[\"vec\",0],\"acc\"]"),
        ("concat", "[\"concat\",[\"vec\",\"acc\"]]"),
    ] {
        let text = probe(&junk(&format!(
            "[\"fold\",\"acc\",[\"vec\"],\"i\",[\"range\",1000],{body}]"
        )));
        let why = on_a_test_thread(move || Law::compile(&text).map(|law| law.nodes()).unwrap_err());
        assert_eq!(why.code, Code::ValueDepth, "{name}: {why}");
    }
}

fn deepest(levels: usize) -> Value {
    let mut value = Value::Int(7);
    for _ in 0..levels {
        value = Value::list(vec![value, Value::Word(1)]);
    }
    value
}

#[test]
fn a_value_of_any_depth_drops_without_overflowing_a_test_thread() {
    let levels = 1_000_000;
    let depth = on_a_test_thread(move || {
        let value = deepest(levels);
        let depth = value.depth();
        drop(value);
        depth
    });
    assert_eq!(depth, levels);
    let shared = on_a_test_thread(move || {
        let value = deepest(levels);
        let again = value.clone();
        let wide = Value::list(vec![value.clone(), again, deepest(levels / 2)]);
        drop(value);
        let depth = wide.depth();
        drop(wide);
        depth
    });
    assert_eq!(shared, levels + 1);
    let branching = on_a_test_thread(|| {
        let mut value = Value::Int(0);
        for _ in 0..200_000 {
            let side = Value::list((0..4).map(Value::Int).collect());
            value = Value::list(vec![side, value]);
        }
        value.depth()
    });
    assert_eq!(branching, 200_000 + 1);
}

fn ops() -> Vec<&'static str> {
    parse::SPECIAL
        .iter()
        .chain(&parse::BINARY)
        .chain(&parse::UNARY)
        .chain(&parse::TERNARY)
        .copied()
        .collect()
}

fn folded(init: &str, body: &str) -> impl Fn(u32) -> String + use<> {
    let init = init.to_owned();
    let body = body.to_owned();
    move |rounds| {
        law(
            1,
            "\"f\":[[\"x\"],[\"add\",\"x\",1]]",
            &junk(&format!(
                "[\"fold\",\"a\",{init},\"i\",[\"range\",{rounds}],{body}]"
            )),
        )
    }
}

fn listed(list: &str, init: &str, body: &str) -> impl Fn(u32) -> String + use<> {
    let list = list.to_owned();
    let init = init.to_owned();
    let body = body.to_owned();
    move |rounds| {
        law(
            1,
            "",
            &junk(&format!(
                "[\"let\",[[\"xs\",{list}]],[\"fold\",\"a\",{init},\"i\",[\"range\",{rounds}],{body}]]"
            )),
        )
    }
}

type Stress = Box<dyn Fn(u32) -> String>;

fn stresses(op: &str) -> Vec<(String, Stress)> {
    let mut cases: Vec<(String, Stress)> = Vec::new();
    let mut add = |name: &str, make: Stress| cases.push((name.to_owned(), make));
    let known = |op: &str| format!("[\"{op}\",\"a\",\"i\"]");
    let live = |op: &str| format!("[\"{op}\",\"a\",[\"param\",0]]");
    let false_flag = "[\"ne\",0,0]";
    match op {
        "let" => add(
            "let",
            Box::new(folded(
                "0",
                "[\"let\",[[\"x\",\"i\"],[\"y\",\"x\"],[\"z\",\"y\"],[\"w\",[\"vec\",\"z\",\"y\"]]],\"z\"]",
            )),
        ),
        "if" => add(
            "if",
            Box::new(folded("0", "[\"if\",[\"and\",\"i\",1],\"a\",\"i\"]")),
        ),
        "vec" => add(
            "vec",
            Box::new(folded(
                "0",
                "[\"at\",[\"vec\",\"i\",\"i\",\"i\",\"i\",\"i\",\"i\",\"i\",\"i\",\"i\",\"i\"],3]",
            )),
        ),
        "range" => add(
            "range",
            Box::new(folded("0", "[\"at\",[\"range\",1000],7]")),
        ),
        "at" => add(
            "at",
            Box::new(listed(
                "[\"range\",4096]",
                "0",
                "[\"at\",\"xs\",[\"and\",\"i\",4095]]",
            )),
        ),
        "put" => add(
            "put",
            Box::new(listed(
                "0",
                "[\"range\",2000]",
                "[\"put\",\"a\",[\"and\",\"i\",1023],\"i\"]",
            )),
        ),
        "map" => add(
            "map",
            Box::new(folded(
                "0",
                "[\"at\",[\"map\",\"x\",[\"range\",500],\"x\"],1]",
            )),
        ),
        "filter" => add(
            "filter",
            Box::new(folded(
                "0",
                "[\"at\",[\"filter\",\"x\",[\"range\",500],[\"lt\",\"x\",400]],1]",
            )),
        ),
        "fold" => add(
            "fold",
            Box::new(folded(
                "0",
                "[\"fold\",\"b\",\"a\",\"j\",[\"range\",50],[\"add\",\"b\",\"j\"]]",
            )),
        ),
        "rev" => add(
            "rev",
            Box::new(listed("0", "[\"range\",2000]", "[\"rev\",\"a\"]")),
        ),
        "concat" => add(
            "concat",
            Box::new(listed(
                "0",
                "[\"range\",2000]",
                "[\"concat\",[\"rev\",\"a\"],[\"vec\"]]",
            )),
        ),
        "param" => add("param", Box::new(folded("0", "[\"param\",0]"))),
        "noise" => add("noise", Box::new(folded("0", "[\"noise\"]"))),
        "look" => {}
        "shl" | "shr" => {
            add(
                op,
                Box::new(folded("1", &format!("[\"{op}\",\"a\",[\"and\",\"i\",31]]"))),
            );
            add(
                &format!("{op} live"),
                Box::new(folded("[\"param\",0]", &format!("[\"{op}\",\"a\",3]"))),
            );
        }
        "eq" | "ne" | "lt" | "le" | "gt" | "ge" => {
            add(
                op,
                Box::new(folded("0", &format!("[\"word\",[\"{op}\",\"a\",\"i\"]]"))),
            );
            add(
                &format!("{op} live"),
                Box::new(folded(
                    "[\"param\",0]",
                    &format!("[\"word\",[\"{op}\",\"a\",\"i\"]]"),
                )),
            );
        }
        "both" | "either" => {
            add(
                op,
                Box::new(folded(
                    false_flag,
                    &format!("[\"{op}\",\"a\",[\"eq\",\"i\",3]]"),
                )),
            );
            add(
                &format!("{op} live"),
                Box::new(folded(
                    "[\"eq\",[\"param\",0],1]",
                    &format!("[\"{op}\",\"a\",[\"eq\",\"i\",3]]"),
                )),
            );
        }
        "not" => {
            add("not", Box::new(folded("0", "[\"not\",\"a\"]")));
            add(
                "not live",
                Box::new(folded("[\"param\",0]", "[\"not\",\"a\"]")),
            );
        }
        "denied" => {
            add("denied", Box::new(folded(false_flag, "[\"denied\",\"a\"]")));
            add(
                "denied live",
                Box::new(folded("[\"eq\",[\"param\",0],1]", "[\"denied\",\"a\"]")),
            );
        }
        "word" => {
            add("word", Box::new(folded("0", "[\"word\",[\"eq\",\"i\",1]]")));
            add(
                "word live",
                Box::new(folded("0", "[\"word\",[\"eq\",[\"param\",0],\"i\"]]")),
            );
        }
        "count" => {
            add(
                "count",
                Box::new(listed(
                    "[\"map\",\"x\",[\"range\",10000],[\"eq\",\"x\",1]]",
                    "0",
                    "[\"count\",\"xs\"]",
                )),
            );
            add(
                "count live",
                Box::new(listed(
                    "[\"map\",\"x\",[\"range\",1000],[\"eq\",\"x\",[\"param\",0]]]",
                    "0",
                    "[\"count\",\"xs\"]",
                )),
            );
        }
        "sel" => {
            add(
                "sel",
                Box::new(folded("0", "[\"sel\",[\"eq\",\"i\",1],\"a\",\"i\"]")),
            );
            add(
                "sel live",
                Box::new(folded(
                    "[\"param\",0]",
                    "[\"sel\",[\"eq\",\"a\",1],\"a\",\"i\"]",
                )),
            );
        }
        _ if parse::BINARY.contains(&op) => {
            add(op, Box::new(folded("1", &known(op))));
            add(&format!("{op} live"), Box::new(folded("1", &live(op))));
        }
        _ => {}
    }
    cases
}

fn widest(make: &dyn Fn(u32) -> String) -> (u32, Law) {
    let mut rounds = 1;
    let mut last = None;
    loop {
        match Law::compile(&make(rounds)) {
            Ok(law) => {
                last = Some((rounds, law));
                rounds = rounds.saturating_mul(2);
            }
            Err(why) => {
                assert!(
                    matches!(why.code, Code::Work | Code::Nodes),
                    "{rounds} rounds: {why}"
                );
                break;
            }
        }
        assert!(rounds < 1 << 24, "never reached a limit");
    }
    last.unwrap_or_else(|| panic!("not even one round compiles: {}", make(1)))
}

const NANOS_PER_UNIT: f64 = if cfg!(debug_assertions) {
    4000.0
} else {
    400.0
};

#[test]
fn every_operation_costs_bounded_time_per_unit_it_charges() {
    let mut worst = (0.0_f64, String::new());
    let mut table = Vec::new();
    for op in ops() {
        let cases = stresses(op);
        if op == "look" {
            let why = refused(&probe(&junk("[\"look\",0,0,0,0,0]")));
            assert_eq!(why.code, Code::Unsupported, "{why}");
            continue;
        }
        assert!(!cases.is_empty(), "no stress case for the operation {op}");
        for (name, make) in cases {
            let (rounds, law) = widest(&make);
            let units = law.work() + law.nodes() as u64;
            assert!(
                law.work() * 2 > limits::WORK || law.nodes() * 4 > limits::NODES,
                "{name}: {rounds} rounds reach only work {} and {} nodes",
                law.work(),
                law.nodes()
            );
            let text = make(rounds);
            let took = (0..3)
                .map(|_| timed(&text).1)
                .min()
                .unwrap_or(Duration::ZERO);
            let nanos = took.as_secs_f64() * 1e9 / units as f64;
            table.push(format!(
                "{name}: {rounds} rounds, work {}, nodes {}, {:.2} ms, {nanos:.1} ns a unit",
                law.work(),
                law.nodes(),
                took.as_secs_f64() * 1e3
            ));
            if nanos > worst.0 {
                worst = (nanos, name);
            }
        }
    }
    let call = folded("0", "[\"f\",\"a\"]");
    let (rounds, law) = widest(&call);
    let took = (0..3)
        .map(|_| timed(&call(rounds)).1)
        .min()
        .unwrap_or(Duration::ZERO);
    let nanos = took.as_secs_f64() * 1e9 / (law.work() + law.nodes() as u64) as f64;
    table.push(format!(
        "call: {rounds} rounds, work {}, {nanos:.1} ns a unit",
        law.work()
    ));
    if nanos > worst.0 {
        worst = (nanos, String::from("call"));
    }
    for line in &table {
        println!("{line}");
    }
    println!(
        "worst {:.1} ns a unit ({}), bound {NANOS_PER_UNIT}",
        worst.0, worst.1
    );
    assert!(
        worst.0 <= NANOS_PER_UNIT,
        "{} costs {:.1} ns a unit",
        worst.1,
        worst.0
    );
}

fn lifted(lifts: u32) -> String {
    let cells: Vec<String> = (0..8)
        .map(|k| match k {
            0 => format!(
                "[\"fold\",\"w\",[\"at\",\"cells\",0],\"i\",[\"range\",{lifts}],[\"add\",\"w\",\"b\"]]"
            ),
            _ => format!("[\"at\",\"cells\",{k}]"),
        })
        .collect();
    let shared = limits::NODES as u32 - lifts - 40;
    law(
        1,
        "",
        &format!(
            "[\"let\",[[\"b\",[\"fold\",\"a\",[\"param\",0],\"i\",[\"range\",{shared}],[\"not\",\"a\"]]]],[\"vec\",[\"vec\",{}],[\"vec\",0]]]",
            cells.join(",")
        ),
    )
    .replacen("\"halo\":0,", "\"halo\":0,\"reversible\":true,", 1)
}

#[test]
fn a_reversible_law_is_checked_in_time_bounded_per_unit() {
    for lifts in [2_000, 4_000, 8_000] {
        let text = lifted(lifts);
        let law = Law::compile(&text).unwrap();
        assert!(law.reversible());
        let units = law.work() + law.nodes() as u64;
        let took = (0..3)
            .map(|_| timed(&text).1)
            .min()
            .unwrap_or(Duration::ZERO);
        let nanos = took.as_secs_f64() * 1e9 / units as f64;
        println!(
            "{lifts} lifts over a shared {} nodes: {} nodes, work {}, {:.2} ms, {nanos:.1} ns a unit",
            limits::NODES as u32 - lifts - 40,
            law.nodes(),
            law.work(),
            took.as_secs_f64() * 1e3
        );
        assert!(
            nanos <= NANOS_PER_UNIT,
            "{lifts} lifts: {nanos:.1} ns a unit"
        );
    }
}

struct Gen {
    state: u64,
    ints: Vec<String>,
    vecs: Vec<String>,
    flags: Vec<String>,
    fresh: u32,
    defs: u32,
}

impl Gen {
    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn below(&mut self, most: u64) -> u64 {
        if most == 0 { 0 } else { self.next() % most }
    }

    fn chance(&mut self, per_mille: u64) -> bool {
        self.below(1000) < per_mille
    }

    fn size(&mut self) -> u64 {
        let bits = if self.chance(20) {
            12 + self.below(9)
        } else {
            self.below(9)
        };
        self.below(1 << bits) + 1
    }

    fn name(&mut self, stem: &str) -> String {
        self.fresh += 1;
        format!("{stem}{}", self.fresh)
    }

    fn pick(&mut self, from: &[String]) -> Option<String> {
        if from.is_empty() {
            return None;
        }
        let at = self.below(from.len() as u64) as usize;
        Some(format!("\"{}\"", from[at]))
    }

    fn int(&mut self, depth: u32) -> String {
        if depth == 0 || self.chance(250) {
            return match self.below(3) {
                0 => self.pick(&self.ints.clone()).unwrap_or_else(|| "7".into()),
                1 => ["\"X\"", "\"Y\"", "\"Z\""][self.below(3) as usize].into(),
                _ => self.size().to_string(),
            };
        }
        let d = depth - 1;
        match self.below(11) {
            0 | 1 => {
                let op =
                    ["add", "sub", "mul", "and", "or", "xor", "shr", "div"][self.below(8) as usize];
                format!("[\"{op}\",{},{}]", self.int(d), self.int(d))
            }
            2 => format!("[\"count\",{}]", self.flag_vec(d)),
            3 => format!("[\"at\",{},0]", self.vec(d)),
            4 => {
                let acc = self.name("a");
                let each = self.name("i");
                let init = self.int(d);
                let list = self.vec(d);
                self.ints.push(acc.clone());
                self.ints.push(each.clone());
                let body = self.int(d);
                self.ints.pop();
                self.ints.pop();
                format!("[\"fold\",\"{acc}\",{init},\"{each}\",{list},{body}]")
            }
            5 => format!("[\"if\",{},{},{}]", self.flag(d), self.int(d), self.int(d)),
            6 => format!("[\"word\",{}]", self.flag(d)),
            7 => format!("[\"sel\",{},{},{}]", self.flag(d), self.int(d), self.int(d)),
            8 => self.call(d),
            9 => {
                let mut bindings = Vec::new();
                let wide = self.size().min(400);
                for _ in 0..wide {
                    let name = self.name("b");
                    bindings.push(format!("[\"{name}\",{}]", self.int(d.min(2))));
                    self.ints.push(name);
                }
                let body = self.int(d);
                for _ in 0..wide {
                    self.ints.pop();
                }
                format!("[\"let\",[{}],{body}]", bindings.join(","))
            }
            _ => format!("[\"not\",{}]", self.int(d)),
        }
    }

    fn flag(&mut self, depth: u32) -> String {
        if depth == 0 || self.chance(200) {
            return match self.pick(&self.flags.clone()) {
                Some(name) if self.chance(500) => name,
                _ => format!("[\"eq\",{},{}]", self.size(), self.size() % 3),
            };
        }
        let d = depth - 1;
        match self.below(5) {
            0 => format!("[\"lt\",{},{}]", self.int(d), self.int(d)),
            1 => format!("[\"both\",{},{}]", self.flag(d), self.flag(d)),
            2 => format!("[\"either\",{},{}]", self.flag(d), self.flag(d)),
            3 => format!("[\"denied\",{}]", self.flag(d)),
            _ => format!("[\"ne\",{},[\"param\",0]]", self.int(d)),
        }
    }

    fn flag_vec(&mut self, depth: u32) -> String {
        let d = depth.saturating_sub(1);
        let name = self.name("x");
        let list = self.vec(d);
        self.ints.push(name.clone());
        let test = format!("[\"lt\",\"{name}\",{}]", self.int(d));
        self.ints.pop();
        format!("[\"map\",\"{name}\",{list},{test}]")
    }

    fn vec(&mut self, depth: u32) -> String {
        if depth == 0 || self.chance(200) {
            return match self.pick(&self.vecs.clone()) {
                Some(name) if self.chance(500) => name,
                _ => format!("[\"range\",{}]", self.size()),
            };
        }
        let d = depth - 1;
        match self.below(10) {
            0 => format!("[\"range\",{}]", self.size()),
            1 => {
                let many = self.below(12);
                let items: Vec<String> = (0..many).map(|_| self.int(d.min(1))).collect();
                format!("[\"vec\",{}]", items.join(","))
            }
            2 => {
                let name = self.name("x");
                let list = self.vec(d);
                self.ints.push(name.clone());
                let body = self.int(d);
                self.ints.pop();
                format!("[\"map\",\"{name}\",{list},{body}]")
            }
            3 => {
                let name = self.name("x");
                let list = self.vec(d);
                self.ints.push(name.clone());
                let test = self.flag(d);
                self.ints.pop();
                format!("[\"filter\",\"{name}\",{list},{test}]")
            }
            4 => format!("[\"rev\",{}]", self.vec(d)),
            5 => format!("[\"concat\",{},{}]", self.vec(d), self.vec(d)),
            6 => format!("[\"put\",{},0,{}]", self.vec(d), self.int(d)),
            7 => {
                let acc = self.name("v");
                let each = self.name("i");
                let init = self.vec(d);
                format!(
                    "[\"fold\",\"{acc}\",{init},\"{each}\",[\"range\",{}],[\"concat\",\"{acc}\",\"{acc}\"]]",
                    self.below(24)
                )
            }
            8 => {
                let acc = self.name("v");
                let each = self.name("i");
                let rounds = if self.chance(100) {
                    self.size()
                } else {
                    self.below(80)
                };
                format!(
                    "[\"at\",[\"fold\",\"{acc}\",[\"vec\"],\"{each}\",[\"range\",{rounds}],[\"vec\",\"{acc}\",\"{acc}\"]],0]"
                )
            }
            _ => {
                let acc = self.name("v");
                let each = self.name("i");
                let init = self.vec(d);
                let list = self.vec(d);
                self.vecs.push(acc.clone());
                self.ints.push(each.clone());
                let body = self.vec(d);
                self.vecs.pop();
                self.ints.pop();
                format!("[\"fold\",\"{acc}\",{init},\"{each}\",{list},{body}]")
            }
        }
    }

    fn call(&mut self, depth: u32) -> String {
        if self.defs == 0 {
            return self.int(0);
        }
        let which = self.below(u64::from(self.defs));
        format!("[\"d{which}\",{}]", self.int(depth))
    }

    fn deep(&mut self) -> String {
        let levels = self.below(140);
        let mut out = String::from("1");
        for _ in 0..levels {
            out = match self.below(3) {
                0 => format!("[\"fold\",\"a\",{out},\"i\",[\"range\",2],[\"add\",\"a\",\"i\"]]"),
                1 => format!("[\"not\",{out}]"),
                _ => format!("[\"at\",[\"vec\",{out}],0]"),
            };
        }
        out
    }

    fn program(&mut self) -> String {
        self.ints.clear();
        self.vecs.clear();
        self.flags.clear();
        self.fresh = 0;
        self.defs = if self.chance(300) {
            self.below(200) as u32
        } else {
            0
        };
        let mut defs = Vec::new();
        let chained = self.chance(300);
        for at in 0..self.defs {
            let body = if at + 1 < self.defs && (chained || self.chance(700)) {
                if !chained && self.chance(300) {
                    format!(
                        "[\"fold\",\"a\",\"x\",\"i\",[\"range\",{}],[\"d{}\",[\"add\",\"a\",\"i\"]]]",
                        self.below(4),
                        at + 1
                    )
                } else {
                    format!("[\"d{}\",[\"add\",\"x\",{}]]", at + 1, self.below(9))
                }
            } else {
                self.ints.push("x".into());
                let saved = self.defs;
                self.defs = 0;
                let body = self.int(3);
                self.defs = saved;
                self.ints.pop();
                body
            };
            defs.push(format!("\"d{at}\":[[\"x\"],{body}]"));
        }
        let mut bindings = Vec::new();
        let wide = 1 + self.below(6);
        for _ in 0..wide {
            let name = self.name("t");
            let depth = 1 + self.below(6) as u32;
            match self.below(4) {
                0 => {
                    bindings.push(format!("[\"{name}\",{}]", self.vec(depth)));
                    self.vecs.push(name);
                }
                1 => {
                    bindings.push(format!("[\"{name}\",{}]", self.flag(depth)));
                    self.flags.push(name);
                }
                2 if self.chance(150) => {
                    bindings.push(format!("[\"{name}\",{}]", self.deep()));
                    self.ints.push(name);
                }
                _ => {
                    bindings.push(format!("[\"{name}\",{}]", self.int(depth)));
                    self.ints.push(name);
                }
            }
        }
        let count = if chained && self.defs > 0 {
            String::from("[\"d0\",1]")
        } else {
            self.int(2)
        };
        let out = if self.chance(100) {
            String::from("[\"rev\",\"cells\"]")
        } else {
            String::from("\"cells\"")
        };
        let step = format!(
            "[\"let\",[{}],[\"vec\",{out},[\"vec\",{count}]]]",
            bindings.join(",")
        );
        let text = law(1, &defs.join(","), &step);
        if self.chance(100) {
            text.replacen("\"halo\":0,", "\"halo\":0,\"reversible\":true,", 1)
        } else {
            text
        }
    }
}

const EXPENSIVE: u64 = 100_000;

const WORKERS: u64 = 4;

const CASE_MOST: Duration = Duration::from_secs(if cfg!(debug_assertions) { 10 } else { 1 });

type Tally = (
    std::collections::BTreeMap<&'static str, u64>,
    u64,
    (Duration, String),
    Duration,
    u64,
);

fn fuzzed(worker: u64) -> Tally {
    let mut generator = Gen {
        state: 0xC1F0_0000 + worker * 7919 + 1,
        ints: Vec::new(),
        vecs: Vec::new(),
        flags: Vec::new(),
        fresh: 0,
        defs: 0,
    };
    let mut codes = std::collections::BTreeMap::new();
    let mut laws = 0_u64;
    let mut slowest = (Duration::ZERO, String::new());
    let mut total = Duration::ZERO;
    let mut heavy = 0_u64;
    for _ in 0..EXPENSIVE / WORKERS {
        let text = generator.program();
        let (made, took) = timed(&text);
        total += took;
        if took > slowest.0 {
            slowest = (took, text.clone());
        }
        match made {
            Ok(law) => {
                laws += 1;
                if law.work() * 2 > limits::WORK || law.nodes() * 2 > limits::NODES {
                    heavy += 1;
                }
            }
            Err(why) => {
                assert!(why.path.starts_with('$'), "{why}");
                assert!(Code::ALL.contains(&why.code), "{why}");
                if matches!(
                    why.code,
                    Code::Work | Code::Nodes | Code::ValueDepth | Code::Depth
                ) {
                    heavy += 1;
                }
                *codes.entry(why.code.name()).or_insert(0_u64) += 1;
            }
        }
        assert!(took <= CASE_MOST, "{took:?} for {text}");
    }
    (codes, laws, slowest, total, heavy)
}

#[test]
fn compile_is_bounded_on_a_seeded_fuzz_of_expensive_programs() {
    let workers: Vec<_> = (0..WORKERS)
        .map(|worker| {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn(move || fuzzed(worker))
                .unwrap()
        })
        .collect();
    let mut codes = std::collections::BTreeMap::new();
    let mut laws = 0;
    let mut heavy = 0;
    let mut slowest = (Duration::ZERO, String::new());
    let mut total = Duration::ZERO;
    for worker in workers {
        let (some, made, slow, spent, weighty) = worker.join().unwrap();
        for (code, many) in some {
            *codes.entry(code).or_insert(0) += many;
        }
        laws += made;
        heavy += weighty;
        total += spent;
        if slow.0 > slowest.0 {
            slowest = slow;
        }
    }
    println!(
        "{EXPENSIVE} programs: {laws} laws, {heavy} at a cost limit or past half of one, refusals {codes:?}"
    );
    println!(
        "compile time: mean {:.3} ms, slowest {:.1} ms ({} bytes), bound {} ms",
        total.as_secs_f64() * 1e3 / EXPENSIVE as f64,
        slowest.0.as_secs_f64() * 1e3,
        slowest.1.len(),
        CASE_MOST.as_millis()
    );
    assert!(laws * 10 > EXPENSIVE, "{laws}");
    assert!(heavy * 50 > EXPENSIVE, "{heavy}");
    for code in ["work", "nodes", "value_depth", "depth", "index"] {
        assert!(codes.contains_key(code), "no {code} refusal: {codes:?}");
    }
}
