#![allow(dead_code)]

use grit::Host;
use grit::border::Absorb;
use grit::branch::{Made, Stepping, World};
use grit::compact::Plan;
use grit::law::Law;
use grit::sand::Grid;
use grit::serve3::Server;
use grit::sweep;
use grit::wire::Json;

use super::pinned::Counted;
use super::served::learned;

pub fn iterating(text: &str) -> Law {
    let json = grit::law::parse::read(text).unwrap();
    let Json::Map(mut fields) = json else {
        panic!("a law is an object");
    };
    fields.push((
        "alternate".to_owned(),
        Json::List(vec![Json::Int(0), Json::Int(1)]),
    ));
    Law::from_json(&Json::Map(fields)).unwrap()
}

pub fn learned_law(rule: &Counted, parallel: bool, tables: &str) -> Law {
    let mut server = Server::new(None);
    iterating(&learned(&mut server, rule, parallel, tables))
}

pub fn law_world<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    sparse: bool,
    absorb: Option<[u32; 3]>,
) -> World<'host> {
    World::create_law_absorbing(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse,
            slots: None,
        },
        Stepping::Listed,
        absorb.and_then(Absorb::new),
    )
    .unwrap()
}

pub fn rule_world<'host>(
    host: &'host Host,
    rule: &str,
    size: [u32; 3],
    sparse: bool,
    absorb: Option<[u32; 3]>,
) -> World<'host> {
    World::create_absorbing(
        host,
        &Made {
            rule,
            params: &[],
            size,
            sparse,
            slots: None,
        },
        absorb.and_then(Absorb::new),
    )
    .unwrap()
}

pub fn cells_of(size: [u32; 3]) -> usize {
    size.iter().map(|side| *side as usize).product()
}

pub fn point_of(size: [u32; 3], index: usize) -> [u32; 3] {
    let wide = size[0] as usize;
    let deep = size[1] as usize;
    [
        (index % wide) as u32,
        (index / wide % deep) as u32,
        (index / wide / deep) as u32,
    ]
}

pub fn index_of(size: [u32; 3], at: [u32; 3]) -> usize {
    (at[2] as usize * size[1] as usize + at[1] as usize) * size[0] as usize + at[0] as usize
}

pub fn in_shell(size: [u32; 3], shell: [u32; 3], at: [u32; 3]) -> bool {
    (0..3).any(|axis| at[axis] < shell[axis] || at[axis] >= size[axis] - shell[axis])
}

pub fn shell_writes(
    size: [u32; 3],
    shell: [u32; 3],
    planes: usize,
    cells: &mut [u32],
) -> Vec<Vec<([u32; 3], u32)>> {
    let count = cells_of(size);
    let mut writes = vec![Vec::new(); planes];
    for (plane, list) in writes.iter_mut().enumerate() {
        for index in 0..count {
            let word = &mut cells[plane * count + index];
            if *word == 0 {
                continue;
            }
            let at = point_of(size, index);
            if in_shell(size, shell, at) {
                *word = 0;
                list.push((at, 0));
            }
        }
    }
    writes
}

pub fn first_difference(size: [u32; 3], got: &[u32], want: &[u32]) -> Option<String> {
    let count = cells_of(size);
    got.iter()
        .zip(want)
        .position(|(one, other)| one != other)
        .map(|at| {
            format!(
                "plane {} cell {:?}: device {:#x}, model {:#x}",
                at / count,
                point_of(size, at % count),
                got[at],
                want[at]
            )
        })
        .or_else(|| {
            (got.len() != want.len()).then(|| format!("{} words against {}", got.len(), want.len()))
        })
}

pub fn fitting(host: &Host, sides: &[u32]) -> u32 {
    let width = host.width();
    sides
        .iter()
        .copied()
        .find(|&side| {
            let grid = Grid::solid(side, side, side).unwrap();
            Plan::bricked(width, grid.blocks().div_ceil(sweep::cells(width))).is_ok()
        })
        .unwrap()
}

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, most: u64) -> u64 {
        self.next() % most
    }

    pub fn word(&mut self) -> u32 {
        u32::try_from(self.next() & 0xFFFF_FFFF).unwrap()
    }
}

pub fn soup(
    rng: &mut Rng,
    from: [u32; 3],
    sides: [u32; 3],
    permille: u64,
    word: &mut dyn FnMut(&mut Rng) -> u32,
) -> Vec<([u32; 3], u32)> {
    let mut out = Vec::new();
    for z in 0..sides[2] {
        for y in 0..sides[1] {
            for x in 0..sides[0] {
                if rng.below(1000) < permille {
                    let held = word(rng);
                    if held != 0 {
                        out.push(([from[0] + x, from[1] + y, from[2] + z], held));
                    }
                }
            }
        }
    }
    out
}

pub fn written(world: &mut World<'_>, plane: u32, cells: &[([u32; 3], u32)]) {
    for piece in cells.chunks(4096) {
        world.write_plane(plane, piece).unwrap();
    }
}

pub fn set_count(world: &mut World<'_>, plane: u32) -> u64 {
    let size = world.size();
    let top = [size[0] - 1, size[1] - 1, size[2] - 1];
    world
        .bit_tally(plane, [0, 0, 0], top, u32::MAX)
        .unwrap()
        .iter()
        .map(|bit| u64::from(bit.count))
        .sum()
}
