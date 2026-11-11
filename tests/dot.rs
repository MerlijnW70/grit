#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Groups;
use grit::ops::{Cpu, Device, Dots, Ops, signed_lanes, unsigned_lanes};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

const CHUNK: usize = 1 << 16;

const RANDOM: usize = 1 << 20;

const EDGES: [u8; 5] = [0x00, 0x01, 0x7F, 0x80, 0xFF];

const DOT_PRODUCT: u32 = 6019;

const PACKED: u32 = 6018;

const CAPABILITY: u32 = 17;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn signed(left: u32, right: u32) -> u32 {
    Cpu.dot_signed(left, right)
}

fn unsigned(left: u32, right: u32) -> u32 {
    Cpu.dot_unsigned(left, right)
}

fn packed(lanes: [u8; 4]) -> u32 {
    u32::from_le_bytes(lanes)
}

#[test]
fn lane_zero_is_the_lowest_byte_and_lane_three_the_highest() {
    let counting = packed([1, 2, 3, 4]);
    for (lane, want) in [(0, 1), (1, 2), (2, 3), (3, 4)] {
        let mut alone = [0_u8; 4];
        alone[lane] = 1;
        assert_eq!(signed(packed(alone), counting), want, "lane {lane}");
        assert_eq!(unsigned(packed(alone), counting), want, "lane {lane}");
    }
    assert_eq!(signed(counting, counting), 30);
    assert_eq!(signed_lanes(0x8001_FF7F), [127, -1, 1, -128]);
    assert_eq!(unsigned_lanes(0x8001_FF7F), [127, 255, 1, 128]);
}

#[test]
fn a_signed_lane_extends_its_sign_and_an_unsigned_lane_does_not() {
    for lane in 0..4 {
        let mut minus = [0_u8; 4];
        minus[lane] = 0xFF;
        let mut one = [0_u8; 4];
        one[lane] = 1;
        assert_eq!(signed(packed(minus), packed(one)), u32::MAX, "lane {lane}");
        assert_eq!(unsigned(packed(minus), packed(one)), 255, "lane {lane}");
        let mut low = [0_u8; 4];
        low[lane] = 0x80;
        assert_eq!(
            signed(packed(low), packed(one)),
            (-128_i32).cast_unsigned(),
            "lane {lane}"
        );
        assert_eq!(unsigned(packed(low), packed(one)), 128, "lane {lane}");
    }
}

#[test]
fn the_extremes_reach_the_range_of_the_sum_and_nothing_wraps() {
    let low = 0x8080_8080;
    let high = 0x7F7F_7F7F;
    assert_eq!(signed(low, low), 65_536);
    assert_eq!(signed(low, high), (-65_024_i32).cast_unsigned());
    assert_eq!(signed(high, high), 64_516);
    assert_eq!(signed(u32::MAX, u32::MAX), 4);
    assert_eq!(signed(u32::MAX, low), 512);
    assert_eq!(unsigned(u32::MAX, u32::MAX), 260_100);
    assert_eq!(unsigned(low, high), 65_024);
    assert_eq!(signed(0, u32::MAX), 0);
    assert_eq!(unsigned(0, u32::MAX), 0);
}

#[test]
fn every_byte_pair_in_every_lane_is_its_product() {
    for lane in 0..4 {
        for one in 0..=255_u8 {
            for other in 0..=255_u8 {
                let mut left = [0_u8; 4];
                let mut right = [0_u8; 4];
                left[lane] = one;
                right[lane] = other;
                let want = i32::from(one.cast_signed()) * i32::from(other.cast_signed());
                assert_eq!(signed(packed(left), packed(right)), want.cast_unsigned());
                let want = u32::from(one) * u32::from(other);
                assert_eq!(unsigned(packed(left), packed(right)), want);
            }
        }
    }
}

fn edge_words() -> Vec<u32> {
    let mut words = Vec::new();
    for a in EDGES {
        for b in EDGES {
            for c in EDGES {
                for d in EDGES {
                    words.push(packed([a, b, c, d]));
                }
            }
        }
    }
    words
}

fn pairs() -> Vec<(u32, u32)> {
    let edges = edge_words();
    let mut out = Vec::with_capacity(edges.len() * edges.len() + RANDOM);
    for &left in &edges {
        for &right in &edges {
            out.push((left, right));
        }
    }
    let mut state = 0xD07_5EED_u64;
    for _ in 0..RANDOM {
        let roll = next(&mut state);
        out.push(((roll >> 32) as u32, roll as u32));
    }
    out
}

fn dot_kernel<const LANES: u32>(subgroup: u32, dots: Dots) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, LANES, 4))?;
    let left = kernel.load::<LANES>(0)?;
    let right = kernel.load::<LANES>(1)?;
    let (signed, unsigned) = {
        let mut lanes = kernel.lanes()?;
        let mut device = Device::<LANES>::dotting(&mut lanes, dots)?;
        let signed = device.dot_signed(left, right);
        let unsigned = device.dot_unsigned(left, right);
        device.finish((signed, unsigned))?
    };
    kernel.store::<LANES>(2, signed)?;
    kernel.store::<LANES>(3, unsigned)?;
    kernel.finish()
}

fn built(subgroup: u32, dots: Dots) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => dot_kernel::<4>(subgroup, dots),
        8 => dot_kernel::<8>(subgroup, dots),
        16 => dot_kernel::<16>(subgroup, dots),
        32 => dot_kernel::<32>(subgroup, dots),
        64 => dot_kernel::<64>(subgroup, dots),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

fn declared(module: &[u32]) -> Vec<u32> {
    let mut found = Vec::new();
    let mut at = 5;
    while at < module.len() {
        let word = module[at];
        let length = (word >> 16) as usize;
        if word & 0xFFFF == CAPABILITY {
            found.push(module[at + 1]);
        }
        at += length.max(1);
    }
    found
}

fn dotting(module: &[u32]) -> bool {
    let found = declared(module);
    let both = found.contains(&DOT_PRODUCT) && found.contains(&PACKED);
    let either = found.contains(&DOT_PRODUCT) || found.contains(&PACKED);
    assert_eq!(both, either, "{found:?}");
    both
}

#[test]
fn only_the_native_path_declares_the_dot_product_capabilities() {
    for subgroup in [4, 8, 16, 32, 64] {
        let native = built(subgroup, Dots::Native).unwrap();
        let exact = built(subgroup, Dots::Exact).unwrap();
        assert!(dotting(&native), "width {subgroup}");
        assert!(!dotting(&exact), "width {subgroup}");
    }
}

fn run(host: &grit::Host, dots: Dots, all: &[(u32, u32)]) -> usize {
    let module = built(host.width(), dots).unwrap();
    let mut held = host
        .resident(&[CHUNK, CHUNK, CHUNK, CHUNK], &[&module])
        .unwrap();
    let groups = [Groups::Fixed(u32::try_from(CHUNK).unwrap() / host.width())];
    let stages: Vec<&[Groups]> = vec![&groups];
    let mut compared = 0;
    for part in all.chunks(CHUNK) {
        let mut left: Vec<u32> = part.iter().map(|pair| pair.0).collect();
        let mut right: Vec<u32> = part.iter().map(|pair| pair.1).collect();
        left.resize(CHUNK, 0);
        right.resize(CHUNK, 0);
        held.write(0, &left).unwrap();
        held.write(1, &right).unwrap();
        let (_, got_signed) = held.staged_reading(&stages, 2, CHUNK).unwrap();
        let got_unsigned = held.read(3, CHUNK).unwrap();
        for (at, &(one, other)) in part.iter().enumerate() {
            assert_eq!(
                got_signed[at],
                signed(one, other),
                "{} {dots:?} signed {one:#010x} {other:#010x}",
                host.name()
            );
            assert_eq!(
                got_unsigned[at],
                unsigned(one, other),
                "{} {dots:?} unsigned {one:#010x} {other:#010x}",
                host.name()
            );
            compared += 1;
        }
    }
    compared
}

#[test]
fn both_dots_on_the_device_equal_the_cpu_on_both_paths() {
    let Some(host) = device("dot") else {
        return;
    };
    let all = pairs();
    let mut paths = vec![Dots::Exact];
    if host.dots() == Dots::Native {
        paths.push(Dots::Native);
    }
    for dots in paths {
        let compared = run(&host, dots, &all);
        assert_eq!(compared, all.len());
        println!(
            "{} width {} offers {:?}: {dots:?} path equals the CPU on {compared} pairs",
            host.name(),
            host.width(),
            host.dots()
        );
    }
}
