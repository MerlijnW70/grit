#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::Groups;
use grit::ops::{Cpu, Device, Flag, Helpers, Ops};
use grit::rule3::sand::Sand;
use grit::rule3::{Axis, CORNERS, Corner, Oct, Octet, Rule};
use grit::sand::{AIR, PATIENCE, SAND, WALL, WATER};
use grit::sand3::{settle_oct, x_first};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

/// The shift amounts tried, past 32 so the modulo shows.
const SHIFTS: u32 = 70;

/// Every operation [`Ops`] offers and every [`Helpers`] function, each once
/// or more on `a`, `b` and `c`, named. The same function runs on the CPU
/// and on the device, so its outputs must agree one by one.
fn every_op<O: Ops>(ops: &mut O, [a, b, c]: [O::Word; 3]) -> Vec<(String, O::Word)> {
    let mut out: Vec<(String, O::Word)> = Vec::new();
    for value in [0, 1, 0x8000_0000, u32::MAX, 0x9E37_79B9] {
        out.push((format!("constant {value:#x}"), ops.constant(value)));
    }
    out.push(("and".into(), ops.and(a, b)));
    out.push(("or".into(), ops.or(a, b)));
    out.push(("xor".into(), ops.xor(a, b)));
    out.push(("not".into(), ops.not(a)));
    for by in 0..SHIFTS {
        out.push((format!("shift_left {by}"), ops.shift_left(a, by)));
        out.push((format!("shift_right {by}"), ops.shift_right(a, by)));
    }
    out.push(("shift_left max".into(), ops.shift_left(a, u32::MAX)));
    out.push(("shift_right max".into(), ops.shift_right(a, u32::MAX)));
    out.push(("add".into(), ops.add(a, b)));
    out.push(("sub".into(), ops.sub(a, b)));
    out.push(("sub small".into(), ops.sub(c, a)));
    out.push(("mul".into(), ops.mul(a, b)));
    out.push(("div".into(), ops.div(a, b)));
    out.push(("div small".into(), ops.div(a, c)));
    out.push(("div of small".into(), ops.div(c, a)));

    let mut flags: Vec<(String, Flag<O::Word>)> = Vec::new();
    for (pair, left, right) in [("a b", a, b), ("a c", a, c), ("c b", c, b)] {
        flags.push((format!("equal {pair}"), ops.equal(left, right)));
        flags.push((format!("not_equal {pair}"), ops.not_equal(left, right)));
        flags.push((format!("less {pair}"), ops.less(left, right)));
        flags.push((format!("less_equal {pair}"), ops.less_equal(left, right)));
        flags.push((format!("greater {pair}"), ops.greater(left, right)));
        flags.push((
            format!("greater_equal {pair}"),
            ops.greater_equal(left, right),
        ));
    }
    let made = flags.clone();
    for pair in made.windows(2) {
        let (first, second) = (&pair[0], &pair[1]);
        flags.push((
            format!("both {} {}", first.0, second.0),
            ops.both(first.1, second.1),
        ));
        flags.push((
            format!("either {} {}", first.0, second.0),
            ops.either(first.1, second.1),
        ));
        flags.push((format!("denied {}", first.0), ops.denied(first.1)));
    }
    flags.push(("truth true".into(), ops.truth(true)));
    flags.push(("truth false".into(), ops.truth(false)));
    flags.push(("is c 0".into(), ops.is(c, 0)));
    flags.push(("is c 2".into(), ops.is(c, 2)));
    flags.push(("is a max".into(), ops.is(a, u32::MAX)));

    for (name, flag) in &flags {
        out.push((format!("flag {name}"), flag.word()));
        out.push((format!("select {name}"), ops.select(*flag, a, b)));
    }
    out.push((
        "count".into(),
        ops.count(flags.iter().map(|(_, flag)| *flag)),
    ));
    out.push(("masked".into(), ops.masked(a, 0x00FF_F00F)));
    for (from, bits) in [(0, 1), (0, 31), (3, 5), (24, 4), (28, 3), (31, 1), (16, 16)] {
        out.push((format!("field {from} {bits}"), ops.field(a, from, bits)));
    }
    out
}

/// The inputs of one invocation: random words, with the edges of `u32`,
/// equal and neighbouring pairs, and small words with zeros among them.
fn inputs(state: &mut u64) -> [u32; 3] {
    let roll = next(state);
    let a = match roll % 8 {
        0 => [0, 1, u32::MAX, 0x8000_0000, 0x7FFF_FFFF, 2, 0xFFFF][(roll >> 8) as usize % 7],
        _ => next(state) as u32,
    };
    let b = match (roll >> 4) % 6 {
        0 => a,
        1 => a.wrapping_add(1),
        2 => a.wrapping_sub(1),
        3 => (roll >> 20) as u32 % 4,
        _ => next(state) as u32,
    };
    let c = (roll >> 32) as u32 % 4;
    [a, b, c]
}

const INVOCATIONS: u32 = 4096;

fn op_kernel<const LANES: u32>(subgroup: u32, per: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, 2))?;
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let mine = kernel.module().i_add(uint, base, lane)?;
    let three = kernel.module().constant_u32(3)?;
    let first = kernel.module().i_mul(uint, mine, three)?;
    let mut loaded = [first; 3];
    for (at, held) in (0_u32..).zip(&mut loaded) {
        let along = kernel.module().constant_u32(at)?;
        let place = kernel.module().i_add(uint, first, along)?;
        *held = kernel.load_at(0, place)?;
    }
    let outs = {
        let mut lanes = kernel.lanes()?;
        let mut words = Vec::with_capacity(3);
        for id in loaded {
            words.push(lanes.splat_id::<U32, LANES>(id)?);
        }
        let mut device = Device::<LANES>::new(&mut lanes)?;
        let outs = every_op(&mut device, [words[0], words[1], words[2]]);
        device.finish(outs)?
    };
    let per = kernel.module().constant_u32(per)?;
    let start = kernel.module().i_mul(uint, mine, per)?;
    for (at, (_, word)) in (0_u32..).zip(&outs) {
        let along = kernel.module().constant_u32(at)?;
        let place = kernel.module().i_add(uint, start, along)?;
        kernel.store_at(1, place, word.id())?;
    }
    kernel.finish()
}

fn built(subgroup: u32, per: u32) -> Result<Vec<u32>, LaneError> {
    match subgroup {
        4 => op_kernel::<4>(subgroup, per),
        8 => op_kernel::<8>(subgroup, per),
        16 => op_kernel::<16>(subgroup, per),
        32 => op_kernel::<32>(subgroup, per),
        64 => op_kernel::<64>(subgroup, per),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }
}

/// Every operation of [`Ops`], and every [`Helpers`] function, on the
/// device equals the same on the CPU, op by op, on 4096 invocations of
/// random and edge inputs from each of 4 seeds.
#[test]
fn every_op_on_the_device_equals_the_same_op_on_the_cpu() {
    let Some(host) = device("every op") else {
        return;
    };
    let names: Vec<String> = every_op(&mut Cpu, [0, 0, 0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let per = u32::try_from(names.len()).unwrap();
    let module = built(host.width(), per).unwrap();
    let count = INVOCATIONS as usize;
    let mut held = host
        .resident(&[3 * count, count * names.len()], &[&module])
        .unwrap();
    let groups = [Groups::Fixed(INVOCATIONS / host.width())];
    let stages: Vec<&[Groups]> = vec![&groups];
    let mut compared = 0_u64;
    for seed in [1_u64, 7, 0x5EED, 0xDEAD_BEEF] {
        let mut state = seed;
        let drawn: Vec<[u32; 3]> = (0..count).map(|_| inputs(&mut state)).collect();
        held.write(0, &drawn.concat()).unwrap();
        let (_, got) = held
            .staged_reading(&stages, 1, count * names.len())
            .unwrap();
        for (invocation, input) in drawn.iter().enumerate() {
            let wanted = every_op(&mut Cpu, *input);
            for (op, (name, want)) in wanted.iter().enumerate() {
                let had = got[invocation * names.len() + op];
                assert_eq!(
                    had, *want,
                    "{name} on {input:#x?}: device {had:#x}, cpu {want:#x} (seed {seed:#x}, invocation {invocation})"
                );
                compared += 1;
            }
        }
    }
    println!(
        "{} width {}: {} ops x {count} invocations x 4 seeds, {compared} results equal",
        host.name(),
        host.width(),
        names.len()
    );
}

/// The device's ops build at every width, and the op list holds each
/// operation the module names.
#[test]
fn the_op_kernel_builds_at_every_width_and_covers_every_op() {
    let names: Vec<String> = every_op(&mut Cpu, [0, 0, 0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for op in [
        "constant",
        "and",
        "or",
        "xor",
        "not",
        "shift_left",
        "shift_right",
        "add",
        "sub",
        "mul",
        "div",
        "flag equal",
        "flag not_equal",
        "flag less",
        "flag less_equal",
        "flag greater",
        "flag greater_equal",
        "select",
        "flag truth",
        "flag is",
        "flag both",
        "flag either",
        "flag denied",
        "masked",
        "field",
        "count",
    ] {
        assert!(names.iter().any(|name| name.starts_with(op)), "{op}");
    }
    let per = u32::try_from(names.len()).unwrap();
    for width in [4, 8, 16, 32, 64] {
        assert!(!built(width, per).unwrap().is_empty());
    }
    assert!(built(12, per).is_err());
}

/// The CPU ops mean what the module says, on the edges.
#[test]
fn the_cpu_ops_mean_what_the_module_says() {
    let ops = &mut Cpu;
    assert_eq!(ops.shift_left(1, 33), 2);
    assert_eq!(ops.shift_right(0x8000_0000, 63), 1);
    assert_eq!(ops.add(u32::MAX, 2), 1);
    assert_eq!(ops.sub(0, 1), u32::MAX);
    assert_eq!(ops.mul(0x8000_0000, 2), 0);
    assert_eq!(ops.div(7, 0), 7);
    assert_eq!(ops.div(7, 2), 3);
    assert_eq!(ops.less(u32::MAX, 0).word(), 0);
    assert_eq!(ops.greater(u32::MAX, 0).word(), 1);
    let yes = ops.equal(3, 3);
    let no = ops.equal(3, 4);
    assert_eq!((yes.word(), no.word()), (1, 0));
    assert_eq!(ops.select(yes, 10, 20), 10);
    assert_eq!(ops.select(no, 10, 20), 20);
    let denied = ops.denied(yes);
    assert_eq!(denied.word(), 0);
    assert_eq!(ops.count([yes, no, yes]), 2);
    assert_eq!(ops.field(0xABCD_1234, 28, 4), 0xA);
    assert_eq!(ops.field(u32::MAX, 0, 31), 0x7FFF_FFFF);
}

#[test]
fn the_corner_helpers_follow_the_corner_order() {
    assert_eq!(Axis::ALL.map(Axis::bit), [1, 4, 2]);
    assert_eq!(Axis::ALL.map(Axis::index), [0, 1, 2]);
    assert_eq!(Axis::of(3), None);
    for (index, axis) in Axis::ALL.into_iter().enumerate() {
        assert_eq!(Axis::of(index), Some(axis));
    }
    assert_eq!(Corner::new(CORNERS), None);
    for (index, corner) in Corner::ALL.into_iter().enumerate() {
        assert_eq!(Corner::new(index), Some(corner));
        assert_eq!(corner.index(), index);
        for axis in Axis::ALL {
            let near = corner.neighbour(axis);
            assert_eq!(near.index(), index ^ axis.bit());
            assert_eq!(near.neighbour(axis), corner);
            assert_eq!(corner.side(axis), u32::from(index & axis.bit() != 0));
            assert_ne!(near.side(axis), corner.side(axis));
            for other in Axis::ALL.into_iter().filter(|other| *other != axis) {
                assert_eq!(near.side(other), corner.side(other));
            }
        }
    }
    let x = [0, 1, 0, 1, 0, 1, 0, 1];
    let y = [0, 0, 0, 0, 1, 1, 1, 1];
    let z = [0, 0, 1, 1, 0, 0, 1, 1];
    for corner in Corner::ALL {
        let side = |axis| corner.side(axis);
        assert_eq!(
            [side(Axis::X), side(Axis::Y), side(Axis::Z)],
            [x[corner.index()], y[corner.index()], z[corner.index()]]
        );
    }
    let values: [u32; CORNERS] = std::array::from_fn(|at| 10 * u32::try_from(at).unwrap());
    let mut octet = Octet::from(values);
    for corner in Corner::ALL {
        assert_eq!(octet[corner], values[corner.index()]);
        octet[corner] += 1;
    }
    assert_eq!(octet, Octet::from_fn(|corner| values[corner.index()] + 1));
    assert_eq!(
        octet.iter().map(|(corner, _)| corner).collect::<Vec<_>>(),
        Corner::ALL
    );
    assert_eq!(octet.map(|value| value / 10).0, [0, 1, 2, 3, 4, 5, 6, 7]);
}

/// The sand step as it was written before it was one step over the ops,
/// branch by branch on the CPU: the oracle the generic step is held to.
mod before {
    use super::{AIR, CORNERS, PATIENCE, SAND, WALL, WATER};

    const MATERIAL_BITS: u32 = 2;
    const RUNNING_BITS: u32 = 3;
    const CARRIED_BITS: u32 = MATERIAL_BITS + RUNNING_BITS;
    const TIRED_BITS: u32 = 3;
    const HELD: u32 = 1 << CARRIED_BITS;
    const TIRED: u32 = HELD << 1;
    const MINUS_X: u32 = 1;
    const PLUS_X: u32 = 2;
    const MINUS_Z: u32 = 5;
    const PLUS_Z: u32 = 6;

    const fn material_of(word: u32) -> u32 {
        word % (1 << MATERIAL_BITS)
    }

    const fn running_of(word: u32) -> u32 {
        (word >> MATERIAL_BITS) % (1 << RUNNING_BITS)
    }

    const fn held_of(word: u32) -> u32 {
        (word >> CARRIED_BITS) % 2
    }

    const fn tired_of(word: u32) -> u32 {
        (word >> (CARRIED_BITS + 1)) % (1 << TIRED_BITS)
    }

    pub const fn hold_of(word: u32, under: u32) -> u32 {
        let carried = word % HELD;
        let worn = tired_of(word) * TIRED;
        if material_of(word) == WATER && material_of(under) == WATER {
            carried + HELD + worn
        } else {
            carried + worn
        }
    }

    #[derive(Clone, Copy)]
    enum Axis {
        X,
        Z,
    }

    impl Axis {
        const fn bit(self) -> usize {
            match self {
                Self::X => 1,
                Self::Z => 2,
            }
        }

        const fn minus(self) -> u32 {
            match self {
                Self::X => MINUS_X,
                Self::Z => MINUS_Z,
            }
        }

        const fn plus(self) -> u32 {
            match self {
                Self::X => PLUS_X,
                Self::Z => PLUS_Z,
            }
        }
    }

    const fn halted(word: u32) -> u32 {
        material_of(word) + held_of(word) * HELD + tired_of(word) * TIRED
    }

    const fn ready(word: u32) -> bool {
        material_of(word) == WATER
            && running_of(word) == 0
            && held_of(word) != 0
            && tired_of(word) < PATIENCE
    }

    const fn started(word: u32, way: u32) -> u32 {
        WATER + (way << MATERIAL_BITS) + HELD + (tired_of(word) + 1) * TIRED
    }

    fn along(cells: &mut [u32; CORNERS], axis: Axis, low: usize, high: usize) {
        let (here, there) = (cells[low], cells[high]);
        let goes_up = running_of(here) == axis.plus();
        let goes_down = running_of(there) == axis.minus();
        let up_free = material_of(there) == AIR;
        let down_free = material_of(here) == AIR;
        let kept_here = if goes_up && !up_free {
            halted(here)
        } else {
            here
        };
        let kept_there = if goes_down && !down_free {
            halted(there)
        } else {
            there
        };
        if (goes_up && up_free) || (goes_down && down_free) {
            cells[low] = kept_there;
            cells[high] = kept_here;
        } else {
            cells[low] = kept_here;
            cells[high] = kept_there;
        }
    }

    fn fall(cells: &mut [u32; CORNERS], upper: usize, lower: usize) {
        let (up, low) = (cells[upper], cells[lower]);
        let (above, below) = (material_of(up), material_of(low));
        if above > below && above != WALL {
            cells[upper] = low;
            cells[lower] = above;
        }
    }

    const UPPER: [usize; 4] = [0, 1, 2, 3];
    const LOWER: usize = 4;
    const CORNER: usize = 3;

    pub fn settle_oct(oct: [u32; CORNERS], x_first: bool) -> [u32; CORNERS] {
        let axes = if x_first {
            [Axis::X, Axis::Z]
        } else {
            [Axis::Z, Axis::X]
        };
        let mut cells = oct;
        for top in UPPER {
            cells[top] = hold_of(cells[top], cells[top + LOWER]);
        }
        let before = cells;
        for at in 0..CORNERS {
            let word = before[at];
            if !ready(word) {
                continue;
            }
            for axis in axes {
                let other = at ^ axis.bit();
                if material_of(before[other]) == AIR {
                    let way = if at & axis.bit() == 0 {
                        axis.plus()
                    } else {
                        axis.minus()
                    };
                    cells[at] = started(word, way);
                    break;
                }
            }
        }
        for axis in axes {
            for low in 0..CORNERS {
                if low & axis.bit() == 0 {
                    along(&mut cells, axis, low, low | axis.bit());
                }
            }
        }
        for top in UPPER {
            fall(&mut cells, top, top + LOWER);
        }
        for axis in axes {
            for top in UPPER {
                fall(&mut cells, top, LOWER + (top ^ axis.bit()));
            }
        }
        for top in UPPER {
            fall(&mut cells, top, LOWER + (top ^ CORNER));
        }
        cells
    }

    pub fn counts(oct: [u32; CORNERS], frame: u32) -> [u32; 3] {
        let mut held = oct;
        for top in UPPER {
            held[top] = hold_of(held[top], held[top + LOWER]);
        }
        let out = settle_oct(oct, super::x_first(frame));
        let moved = held.iter().zip(&out).filter(|(a, b)| a != b).count();
        let of = |want| {
            out.iter()
                .filter(|&&word| material_of(word) == want)
                .count()
        };
        [moved, of(WATER), of(SAND)].map(|count| u32::try_from(count).unwrap())
    }
}

/// A sand word: any material, any running way, held bit and tired steps,
/// and random bits above them now and then.
fn sand_word(state: &mut u64) -> u32 {
    let roll = next(state);
    let low = (roll & 0x1FF) as u32;
    if (roll >> 12).is_multiple_of(5) {
        low | (((roll >> 16) as u32) << 9)
    } else {
        low
    }
}

/// The sand step over the ops equals the step as it was written before,
/// branch by branch, on random octs in both axis orders, with its counts;
/// and the hold equals the old hold on every pair of low words.
#[test]
fn the_sand_step_over_the_ops_equals_the_sand_step_it_replaced() {
    let mut state = 0x5A4D_u64;
    let mut moved = 0_u64;
    for round in 0..400_000_u32 {
        let words: [u32; CORNERS] = std::array::from_fn(|_| sand_word(&mut state));
        let frame = next(&mut state) as u32;
        for order in [true, false] {
            let got = settle_oct(words, order);
            assert_eq!(
                got,
                before::settle_oct(words, order),
                "{words:#x?} x first {order}"
            );
            moved += u64::from(got != words);
        }
        let ruled = Sand.step(Oct {
            words,
            frame,
            at: [round, 0, 0],
            params: &[],
        });
        assert_eq!(ruled.out, before::settle_oct(words, x_first(frame)));
        assert_eq!(ruled.counts, before::counts(words, frame), "{words:#x?}");
    }
    assert!(moved > 100_000, "only {moved} steps changed an oct");
    for word in 0..0x200 {
        for under in 0..0x200 {
            assert_eq!(
                grit::sand3::hold_of(word, under),
                before::hold_of(word, under)
            );
        }
    }
    println!("{moved} of 800000 sand steps changed their oct");
}
