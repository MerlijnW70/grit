#![allow(dead_code)]

use grit::serve3::Server;
use grit::wire::{Json, written};

use super::paged::{encoded, packed, read_page};
use super::pinned::LONG;
use super::served::{Rng, int_at, listed, ok, quoted};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    List,
    Words,
    BoxBits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bit,
    Word,
    Fixed,
}

#[derive(Clone, Debug)]
pub struct Fed {
    pub cells: Vec<[u32; 3]>,
    pub values: Vec<Vec<u32>>,
    pub plane: Option<u32>,
    pub from: Option<([u32; 3], [u32; 3])>,
}

pub fn word_of(rng: &mut Rng, kind: Kind) -> u32 {
    match kind {
        Kind::Bit => u32::from(rng.below(1000) < 400),
        Kind::Word => rng.word(),
        Kind::Fixed => {
            super::served::fixed(rng.below(1 << 16) as f64 / 65536.0, super::served::FRAC)
        }
    }
}

pub fn spread(
    rng: &mut Rng,
    size: [u32; 3],
    many: usize,
    within: ([u32; 3], [u32; 3]),
) -> Vec<[u32; 3]> {
    let (from, to) = within;
    let mut cells: Vec<[u32; 3]> = Vec::new();
    while cells.len() < many {
        let cell = [0, 1, 2]
            .map(|axis| from[axis] + rng.below(u64::from(to[axis] - from[axis] + 1)) as u32);
        assert!(cell.iter().zip(size).all(|(one, side)| *one < side));
        if !cells.contains(&cell) {
            cells.push(cell);
        }
    }
    cells
}

pub fn box_of(from: [u32; 3], to: [u32; 3]) -> Vec<[u32; 3]> {
    let mut out = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                out.push([x, y, z]);
            }
        }
    }
    out
}

impl Fed {
    pub fn new(
        rng: &mut Rng,
        cells: Vec<[u32; 3]>,
        count: u32,
        kind: Kind,
        plane: Option<u32>,
    ) -> Self {
        let values = (0..count)
            .map(|_| cells.iter().map(|_| word_of(rng, kind)).collect())
            .collect();
        Self {
            cells,
            values,
            plane,
            from: None,
        }
    }

    pub fn boxed(
        rng: &mut Rng,
        from: [u32; 3],
        to: [u32; 3],
        count: u32,
        kind: Kind,
        plane: Option<u32>,
    ) -> Self {
        let mut fed = Self::new(rng, box_of(from, to), count, kind, plane);
        fed.from = Some((from, to));
        fed
    }

    pub fn side(&self) -> String {
        match self.plane {
            Some(plane) => format!("\"plane\":{plane}"),
            None => "\"current\":true".to_owned(),
        }
    }

    pub fn cells_text(&self) -> String {
        match self.from {
            Some((from, to)) => format!("\"from\":{},\"to\":{}", listed(&from), listed(&to)),
            None => {
                let shown: Vec<String> = self.cells.iter().map(|cell| listed(cell)).collect();
                format!("\"cells\":[{}]", shown.join(","))
            }
        }
    }

    pub fn json(&self, form: Form) -> String {
        let side = self.side();
        let cells = self.cells_text();
        match form {
            Form::List => {
                let rows: Vec<String> = self.values.iter().map(|row| listed(row)).collect();
                format!("{{{side},{cells},\"values\":[{}]}}", rows.join(","))
            }
            Form::Words | Form::BoxBits => {
                let format = if form == Form::Words { "words" } else { "bits" };
                let all: Vec<u32> = self.values.iter().flatten().copied().collect();
                let data = encoded(&packed(format, &all));
                format!("{{{side},{cells},\"format\":\"{format}\",\"data\":\"{data}\"}}")
            }
        }
    }

    pub fn row(&self, at: usize, origin: [u32; 3]) -> Vec<([u32; 3], u32)> {
        self.cells
            .iter()
            .zip(&self.values[at])
            .map(|(cell, word)| ([0, 1, 2].map(|axis| cell[axis] + origin[axis]), *word))
            .collect()
    }
}

pub fn created(
    server: &mut Server<'_>,
    law: &str,
    params: &str,
    size: [u32; 3],
    (alternate, sparse, absorb): (bool, bool, Option<[u32; 3]>),
) -> i64 {
    let mut extra = String::new();
    if alternate {
        extra.push_str(",\"alternate\":[0,1]");
    }
    if sparse {
        extra.push_str(",\"sparse\":true");
    }
    if let Some(shell) = absorb {
        extra.push_str(&format!(",\"absorb\":{}", listed(&shell)));
    }
    let made = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":{},\"deep\":{},\"tall\":{},\"law\":{},\"params\":{params}{extra},\"budget_ms\":{LONG}}}",
            size[0],
            size[1],
            size[2],
            quoted(law)
        ),
    );
    int_at(&made, "world")
}

pub fn put(server: &mut Server<'_>, world: i64, side: &str, cells: &[([u32; 3], u32)]) {
    for piece in cells.chunks(4096) {
        let shown: Vec<String> = piece
            .iter()
            .map(|(at, word)| format!("[{},{},{},{word}]", at[0], at[1], at[2]))
            .collect();
        ok(
            server,
            "grit_world_write",
            &format!(
                "{{\"world\":{world},{side},\"cells\":[{}],\"budget_ms\":{LONG}}}",
                shown.join(",")
            ),
        );
    }
}

pub fn plane_words(server: &mut Server<'_>, world: i64, plane: u32, size: [u32; 3]) -> Vec<u32> {
    let layer = size[0] * size[1];
    let slab = (65536 / layer).max(1);
    let mut words = Vec::new();
    let mut z = 0;
    while z < size[2] {
        let top = (z + slab).min(size[2]) - 1;
        words.extend(read_page(
            server,
            world,
            plane,
            ([0, 0, z], [size[0] - 1, size[1] - 1, top]),
            "words",
        ));
        z = top + 1;
    }
    words
}

pub fn cells_at(server: &mut Server<'_>, world: i64, plane: u32, cells: &[[u32; 3]]) -> Vec<u32> {
    cells
        .iter()
        .map(|cell| {
            let got = ok(
                server,
                "grit_world_read",
                &format!(
                    "{{\"world\":{world},\"plane\":{plane},\"from\":{},\"to\":{}}}",
                    listed(cell),
                    listed(cell)
                ),
            );
            let listed_cells = got.get("cells").and_then(Json::list).unwrap();
            u32::try_from(listed_cells[0].int().unwrap()).unwrap()
        })
        .collect()
}

pub fn stepped_with(server: &mut Server<'_>, world: i64, frames: u32, driven: &str) -> Json {
    ok(
        server,
        "grit_world_step",
        &format!(
            "{{\"world\":{world},\"frames\":{frames},\"driven\":{driven},\"budget_ms\":{LONG}}}"
        ),
    )
}

pub fn shown(held: &Json) -> String {
    let all = written(held);
    all[..all.len().min(600)].to_owned()
}
