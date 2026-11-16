#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::law::Law;
use grit::law::adjoint::{self, Aim, Reversible};
use grit::question::Region;
use grit::sand3::Grid3;
use grit::serve3::Server;
use grit::wire::{Json, written};

const SIDE: u32 = 8;

fn grid() -> Grid3 {
    Grid3::new(SIDE, SIDE, SIDE).unwrap()
}

fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    let line = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    );
    server.heard(&line).unwrap()
}

fn content(got: &Json) -> (bool, Json) {
    let held = got.get("result").unwrap();
    (
        held.get("isError") == Some(&Json::Bool(true)),
        held.get("structuredContent").cloned().unwrap(),
    )
}

fn whole_of(held: &Json) -> u64 {
    match held {
        Json::Int(one) => u64::try_from(*one).unwrap(),
        Json::Text(one) => one.parse().unwrap(),
        other => panic!("{}", written(other)),
    }
}

fn law_text(name: &str, bindings: &[&str]) -> String {
    let out: Vec<String> = (0..8)
        .map(|k| {
            let last = bindings
                .iter()
                .rev()
                .find_map(|one| {
                    let head = format!("[\"w{k}v");
                    one.starts_with(&head)
                        .then(|| one[2..].split('"').next().unwrap().to_owned())
                })
                .unwrap_or(format!("w{k}v0"));
            format!("\"{last}\"")
        })
        .collect();
    let mut all: Vec<String> = (0..8)
        .map(|k| format!("[\"w{k}v0\",[\"at\",\"cells\",{k}]]"))
        .collect();
    all.extend(bindings.iter().map(|one| (*one).to_owned()));
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":1,\"channels\":1,\"halo\":0,\"reversible\":true,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        all.join(","),
        out.join(",")
    )
}

fn start() -> Vec<u32> {
    (0..grid().cells())
        .map(|i| (1 << 22) + u32::try_from(i).unwrap() * 997)
        .collect()
}

fn ask(server: &mut Server<'_>, text: &str, param: u32) -> Option<(u64, i128)> {
    let last = SIDE - 1;
    let target = vec![0_u32; grid().cells()];
    let arguments = format!(
        "{{\"law\":{},\"size\":[{SIDE},{SIDE},{SIDE}],\"cells\":{:?},\"params\":[{param}],\"frames\":2,\"aim\":{{\"from\":[0,0,0],\"to\":[{last},{last},{last}],\"target\":{target:?}}},\"budget_ms\":60000}}",
        written(&Json::Text(text.to_owned())),
        start()
    );
    let got = call(server, "grit_law_gradient", &arguments);
    let (error, held) = content(&got);
    if error {
        assert_eq!(
            held.get("code").and_then(Json::text),
            Some("no_gradient"),
            "refused: {}",
            written(&held)
        );
        return None;
    }
    let gradient: i128 = held.get("gradient").unwrap().list().unwrap()[0]
        .text()
        .unwrap()
        .parse()
        .unwrap();
    Some((whole_of(held.get("objective").unwrap()), gradient))
}

const SHARE: &str = "[\"shr\",[\"mul\",[\"shr\",\"w0v0\",6],[\"param\",0]],12]";

fn cases() -> Vec<(&'static str, String)> {
    let plain = format!("[\"w1v1\",[\"add\",\"w1v0\",{SHARE}]]");
    vec![
        ("plain add", law_text("plain", &[&plain])),
        (
            "sub of not",
            law_text(
                "notted",
                &[&format!("[\"w1v1\",[\"sub\",\"w1v0\",[\"not\",{SHARE}]]]")],
            ),
        ),
        (
            "xor 1 after the add",
            law_text("xored", &[&plain, "[\"w1v2\",[\"xor\",\"w1v1\",1]]"]),
        ),
        (
            "and with a high mask",
            law_text(
                "anded",
                &[&format!(
                    "[\"w1v1\",[\"add\",\"w1v0\",[\"and\",{SHARE},4294967040]]]"
                )],
            ),
        ),
        (
            "dot_u lane 0",
            law_text(
                "dotted",
                &[&format!(
                    "[\"w1v1\",[\"add\",\"w1v0\",[\"dot_u\",[\"shr\",[\"mul\",[\"shr\",\"w0v0\",14],[\"param\",0]],12],200]]]"
                )],
            ),
        ),
    ]
}

#[test]
fn review_a1b_the_gradient_agrees_with_finite_differences_on_laws_the_tool_accepts() {
    let mut server = Server::new(None);
    let at = 3_000_u32;
    let step = 64_u32;
    let mut wrong = Vec::new();
    for (label, text) in cases() {
        Reversible::of(&Law::compile(&text).unwrap()).unwrap();
        let Some((objective, gradient)) = ask(&mut server, &text, at) else {
            println!("{label}: refused no_gradient");
            assert_ne!(label, "plain add");
            continue;
        };
        let (above, _) = ask(&mut server, &text, at + step).unwrap();
        let (under, _) = ask(&mut server, &text, at - step).unwrap();
        assert!(objective > 0);
        let differenced = (i128::from(above) - i128::from(under)) as f64 / f64::from(2 * step);
        let slope = gradient as f64 / 2_f64.powi(adjoint::FRACTION as i32);
        let off = 100.0 * (slope - differenced).abs() / differenced.abs();
        println!(
            "{label}: adjoint {slope:.3}, central difference {differenced:.3}, {off:.2} % apart"
        );
        if !(off <= 1.0 && slope.is_sign_positive() == differenced.is_sign_positive()) {
            wrong.push(format!(
                "{label}: adjoint {slope:.3} vs finite difference {differenced:.3}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "accepted laws whose answered gradient is wrong:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn review_a1b_the_library_walk_gives_the_same_wrong_gradient() {
    for (label, text) in cases() {
        let law = Reversible::of(&Law::compile(&text).unwrap()).unwrap();
        let target = vec![0_u32; grid().cells()];
        let aim = Aim {
            region: Region::whole([SIDE, SIDE, SIDE]),
            target: &target,
        };
        let walked = law.walk(grid(), &start(), &[3_000], 0, 2, &aim).unwrap();
        println!("{label}: library told {:?}", walked.told);
    }
}
