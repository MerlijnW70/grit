#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Host;
use grit::branch::{Made, Stepping, World};
use grit::law::Law;
use grit::law::quiet;
use grit::sand3::Grid3;

const SIDE: u32 = 8;
const FRAMES: u32 = 3;

fn grid() -> Grid3 {
    Grid3::new(SIDE, SIDE, SIDE).unwrap()
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

const EDGES: [u32; 8] = [
    0,
    1,
    2,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFE,
    0xFFFF_FFFF,
    65_535,
];

fn law_of(name: &str, out: &str) -> String {
    let bindings: Vec<String> = (0..8)
        .map(|k| format!("[\"i{k}\",[\"at\",\"cells\",{k}]]"))
        .collect();
    let outs: Vec<String> = (0..8)
        .map(|k| {
            if k == 0 {
                out.to_owned()
            } else {
                format!("\"i{k}\"")
            }
        })
        .collect();
    format!(
        "{{\"law\":1,\"name\":\"{name}\",\"params\":1,\"channels\":1,\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"let\",[{}],[\"vec\",[\"vec\",{}],[\"vec\",0]]]}}",
        bindings.join(","),
        outs.join(",")
    )
}

fn replayed(law: &Law, cells: &[u32], params: &[u32]) -> Vec<Vec<u32>> {
    let mut held = cells.to_vec();
    let mut all = Vec::new();
    for frame in 0..FRAMES {
        held = grit::sand3::reference(law, grid(), frame, &held, params);
        all.push(held.clone());
    }
    all
}

fn twin(host: &Host, law: &Law, params: &[u32], cells: &[u32]) -> Result<(), String> {
    let want = replayed(law, cells, params);
    let stepping = if quiet::class(law, params, &[]).proved() {
        Stepping::Listed
    } else {
        Stepping::Whole
    };
    let mut world = World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params,
            size: [SIDE, SIDE, SIDE],
            sparse: false,
            slots: None,
        },
        stepping,
    )
    .map_err(|why| format!("create: {why}"))?;
    let g = grid();
    let mut placed = Vec::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                placed.push(([x, y, z], cells[g.index(x, y, z)]));
            }
        }
    }
    world
        .write_plane(0, &placed)
        .map_err(|why| format!("write: {why}"))?;
    for (frame, want) in want.iter().enumerate() {
        world
            .step(1)
            .map_err(|why| format!("step {frame}: {why}"))?;
        let got = world.cells().map_err(|why| format!("read: {why}"))?;
        if &got != want {
            let at = got.iter().zip(want).position(|(a, b)| a != b).unwrap();
            return Err(format!(
                "frame {frame}: cell {at} {} vs {}",
                got[at], want[at]
            ));
        }
    }
    Ok(())
}

fn constant_divisor_laws() -> Vec<String> {
    let mut outs = Vec::new();
    for c in [
        2_u32,
        3,
        7,
        255,
        65_535,
        0x8000_0001,
        0xFFFF_FFFE,
        0xFFFF_FFFF,
    ] {
        outs.push(format!(
            "[\"sel\",[\"ne\",[\"div\",[\"mul\",\"i1\",{c}],{c}],\"i1\"],1,0]"
        ));
        outs.push(format!(
            "[\"sel\",[\"eq\",[\"div\",[\"mul\",{c},\"i1\"],{c}],\"i1\"],7,9]"
        ));
        outs.push(format!(
            "[\"sel\",[\"lt\",[\"div\",4294967295,{c}],\"i1\"],1,0]"
        ));
        outs.push(format!(
            "[\"sel\",[\"lt\",[\"div\",[\"not\",0],{c}],\"i1\"],1,0]"
        ));
        outs.push(format!(
            "[\"sel\",[\"lt\",[\"div\",[\"sub\",0,1],{c}],\"i1\"],1,0]"
        ));
        outs.push(format!(
            "[\"sel\",[\"lt\",[\"div\",[\"or\",\"i2\",4294967295],{c}],\"i1\"],1,0]"
        ));
        outs.push(format!(
            "[\"sel\",[\"ge\",[\"div\",[\"or\",\"i2\",4294967295],{c}],\"i1\"],1,0]"
        ));
    }
    outs.push("[\"sel\",[\"ne\",[\"div\",[\"mul\",\"i1\",\"i2\"],\"i1\"],\"i2\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",[\"or\",\"i2\",4294967295],\"i3\"],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",[\"not\",0],\"i3\"],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",[\"sub\",0,1],\"i3\"],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",4294967295,[\"param\",0]],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"ge\",[\"div\",4294967295,[\"or\",\"i3\",1]],\"frame\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",4294967295,[\"and\",\"i3\",0]],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",4294967295,[\"sub\",\"i3\",\"i3\"]],\"i1\"],1,0]".into());
    outs.push("[\"sel\",[\"lt\",[\"div\",4294967295,[\"mul\",\"i3\",0]],\"i1\"],1,0]".into());
    outs
}

#[test]
fn constant_and_foldable_divisor_idioms_build_and_match_on_the_device() {
    let Some(host) = device("constant_and_foldable_divisor_idioms_build_and_match_on_the_device")
    else {
        return;
    };
    let mut failed = Vec::new();
    let mut state = 0x1234_5678_9ABC_DEF1_u64;
    let laws = constant_divisor_laws();
    for (index, out) in laws.iter().enumerate() {
        let text = law_of(&format!("r{index}"), out);
        let law = match Law::compile(&text) {
            Ok(law) => law,
            Err(why) => {
                println!("refused {index}: {why} {out}");
                continue;
            }
        };
        let cells: Vec<u32> = (0..grid().cells())
            .map(|_| {
                let r = next(&mut state);
                if r.is_multiple_of(2) {
                    EDGES[(r >> 8) as usize % EDGES.len()]
                } else {
                    (r >> 32) as u32
                }
            })
            .collect();
        for params in [[0_u32], [1], [u32::MAX]] {
            if let Err(why) = twin(&host, &law, &params, &cells) {
                failed.push(format!("{index} {params:?}: {why}\n{out}"));
            }
        }
    }
    for one in &failed {
        println!("FAILED {one}");
    }
    println!(
        "{} laws on {}, {} failed",
        laws.len(),
        host.name(),
        failed.len()
    );
    assert!(failed.is_empty());
}

fn halved(a: u32, b: u32) -> u32 {
    let d = b.max(1);
    let h = a >> 1;
    let q1 = h / d;
    let back = q1.wrapping_mul(d);
    let rest = h.wrapping_sub(back).wrapping_shl(1) | (a & 1);
    let carry = u32::from(rest >= d);
    q1.wrapping_shl(1).wrapping_add(carry)
}

#[test]
fn the_halved_quotient_equals_the_plain_one_on_edges_and_random_pairs() {
    let mut edges: Vec<u32> = Vec::new();
    for k in 0..32 {
        let p = 1_u32 << k;
        edges.extend([
            p,
            p.wrapping_sub(1),
            p.wrapping_add(1),
            !p,
            !p + 1,
            p | (p >> 1),
        ]);
    }
    edges.extend([0, 1, 2, 3, u32::MAX, u32::MAX - 1, 0x5555_5555, 0xAAAA_AAAA]);
    for &a in &edges {
        for &b in &edges {
            assert_eq!(halved(a, b), a / b.max(1), "a={a:#x} b={b:#x}");
        }
    }
    for b in 0..=4096_u32 {
        for &a in &edges {
            assert_eq!(halved(a, b), a / b.max(1), "a={a:#x} b={b:#x}");
        }
    }
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let pairs: u64 = std::env::var("REVIEW_PAIRS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000_000);
    for _ in 0..pairs {
        let r = next(&mut state);
        let a = r as u32;
        let shift = (r >> 59) as u32;
        let b = ((r >> 32) as u32) >> shift;
        assert_eq!(halved(a, b), a / b.max(1), "a={a:#x} b={b:#x}");
    }
}

#[cfg(feature = "faults")]
mod pipeline {
    use super::*;

    use ash::vk;
    use grit::resident::faults::{self, Step};
    use grit::serve3::Server;
    use grit::wire::{Json, written};

    fn call(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
        );
        server.heard(&line).unwrap()
    }

    fn params_of(rule: &str) -> Vec<u32> {
        grit::serve3::rules()
            .iter()
            .find(|one| one.name() == rule)
            .map(|one| vec![0_u32; one.params()])
            .unwrap()
    }

    #[test]
    fn a_rule_world_whose_kernel_fails_to_build_is_not_called_a_law_fault() {
        let Some(host) =
            device("a_rule_world_whose_kernel_fails_to_build_is_not_called_a_law_fault")
        else {
            return;
        };
        let mut server = Server::new(Some(&host));
        for rule in ["sand", "heat", "nature"] {
            faults::reset();
            faults::arm_with(Step::Pipeline, 0, vk::Result::ERROR_UNKNOWN);
            let got = call(
                &mut server,
                "grit_world_create",
                &format!(
                    "{{\"wide\":{SIDE},\"deep\":{SIDE},\"tall\":{SIDE},\"rule\":\"{rule}\",\"params\":{:?}}}",
                    params_of(rule)
                ),
            );
            faults::disarm();
            assert!(faults::fired(), "{rule}: the fault never fired");
            let held = got.get("result").unwrap();
            let told = held.get("structuredContent").cloned().unwrap();
            println!("{rule}: {}", written(&told));
            assert_ne!(
                told.get("code"),
                Some(&Json::Text("law_not_built".into())),
                "{rule}: a rule world with no law is answered as a law fault: {}",
                written(&told)
            );
            assert_ne!(
                told.get("field"),
                Some(&Json::Text("$.law".into())),
                "{rule}: points at $.law, which the request does not have"
            );
        }
    }
}
