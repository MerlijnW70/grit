use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::wire::{self, Json};

use super::Refused;

pub const MAGIC: [u8; 8] = *b"GRITWLD1";
pub const VERSION: u32 = 1;
pub const CHANNELS: u32 = 1;

pub const MOST_CHANNELS: u32 = 4;

pub const RULE: [u8; 4] = *b"RULE";
pub const LAW: [u8; 4] = *b"LAW ";
pub const TABLE: [u8; 4] = *b"TABL";
pub const POOL: [u8; 4] = *b"POOL";
pub const DENSE: [u8; 4] = *b"DENS";
pub const SUM: [u8; 4] = *b"SUM ";

pub const UNIFORM: u32 = u32::MAX;

pub const SUM_BYTES: usize = 44;

const CHUNK: usize = 1 << 16;

const ROUNDS: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const START: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: START,
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.length = self.length.wrapping_add(bytes.len() as u64);
        let mut rest = bytes;
        if self.filled > 0 {
            let take = (64 - self.filled).min(rest.len());
            let (head, tail) = rest.split_at(take);
            if let Some(room) = self.block.get_mut(self.filled..self.filled + take) {
                room.copy_from_slice(head);
            }
            self.filled += take;
            rest = tail;
            if self.filled == 64 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
        let (blocks, tail) = rest.as_chunks::<64>();
        for block in blocks {
            self.compress(block);
        }
        if let Some(room) = self.block.get_mut(..tail.len()) {
            room.copy_from_slice(tail);
        }
        self.filled += tail.len();
    }

    #[must_use]
    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        let mut padding = vec![0x80_u8];
        let used = (self.filled + 1) % 64;
        let zeros = if used <= 56 { 56 - used } else { 120 - used };
        padding.resize(1 + zeros, 0);
        padding.extend_from_slice(&bits.to_be_bytes());
        let length = self.length;
        self.update(&padding);
        self.length = length;
        let mut out = [0_u8; 32];
        for (slot, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *slot = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0_u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let at = |back: usize| w.get(i - back).copied().unwrap_or_default();
            let a = at(15);
            let b = at(2);
            let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            let next = at(16).wrapping_add(s0).wrapping_add(at(7)).wrapping_add(s1);
            if let Some(slot) = w.get_mut(i) {
                *slot = next;
            }
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (round, word) in ROUNDS.iter().zip(w) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(*round)
                .wrapping_add(word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, add) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(add);
        }
    }
}

#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finish()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Law {
    Rule { name: String },
    Law { name: String, canonical: String },
}

impl Law {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Rule { name } | Self::Law { name, .. } => name,
        }
    }

    #[must_use]
    pub const fn tag(&self) -> [u8; 4] {
        match self {
            Self::Rule { .. } => RULE,
            Self::Law { .. } => LAW,
        }
    }

    #[must_use]
    pub fn canonical(&self, params: usize) -> String {
        match self {
            Self::Rule { name } => wire::written(&wire::map(vec![
                (
                    "params",
                    Json::Int(i64::try_from(params).unwrap_or(i64::MAX)),
                ),
                ("rule", wire::text(name)),
            ])),
            Self::Law { canonical, .. } => canonical.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Dense(Vec<u32>),
    Sparse {
        capacity: u32,
        sides: [u32; 3],
        table: Vec<u32>,
        pool: Vec<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub law: Law,
    pub params: Vec<u32>,
    pub frame: u32,
    pub size: [u32; 3],
    pub state: State,
}

impl Image {
    #[must_use]
    pub fn law_hash(&self) -> [u8; 32] {
        sha256(self.law.canonical(self.params.len()).as_bytes())
    }

    #[must_use]
    pub fn cells(&self) -> u64 {
        self.size.iter().map(|&side| u64::from(side)).product()
    }

    #[must_use]
    pub fn channels(&self) -> u32 {
        let cells = self.cells().max(1);
        let planes = match &self.state {
            State::Dense(words) => words.len() as u64 / cells,
            State::Sparse { sides, table, .. } => {
                let bricks = (cells / brick_cells(*sides).max(1) as u64).max(1);
                (table.len() as u64 / bricks).saturating_sub(1)
            }
        };
        u32::try_from(planes.max(1)).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn unpacked(&self) -> Vec<u32> {
        match &self.state {
            State::Dense(cells) => cells.clone(),
            State::Sparse {
                sides, table, pool, ..
            } => {
                let size = self.size;
                let lattice = lattice_of(size, *sides);
                let per = brick_cells(*sides);
                let total = usize::try_from(self.cells()).unwrap_or(0);
                let planes = self.channels() as usize;
                let bricks = lattice.iter().map(|&side| side as usize).product::<usize>();
                let held = pool.len() / (per * planes).max(1);
                let mut cells = vec![0_u32; total * planes];
                for (index, cell) in cells.iter_mut().enumerate() {
                    let plane = index / total.max(1);
                    let at = index % total.max(1);
                    let point = point_of(size, at as u64);
                    let (brick, offset) = place(point, *sides, lattice);
                    let slot = table.get(brick * 2).copied().unwrap_or(UNIFORM);
                    let value = match plane {
                        0 => table.get(brick * 2 + 1),
                        _ => table.get(2 * bricks + (plane - 1) * bricks + brick),
                    }
                    .copied()
                    .unwrap_or_default();
                    *cell = if slot == UNIFORM {
                        value
                    } else {
                        pool.get(plane * held * per + slot as usize * per + offset)
                            .copied()
                            .unwrap_or_default()
                    };
                }
                cells
            }
        }
    }
}

#[must_use]
pub fn lattice_of(size: [u32; 3], sides: [u32; 3]) -> [u32; 3] {
    [
        size[0] / sides[0].max(1),
        size[1] / sides[1].max(1),
        size[2] / sides[2].max(1),
    ]
}

#[must_use]
pub fn brick_cells(sides: [u32; 3]) -> usize {
    sides.iter().map(|&side| side as usize).product()
}

#[must_use]
pub fn point_of(size: [u32; 3], index: u64) -> [u32; 3] {
    let wide = u64::from(size[0].max(1));
    let deep = u64::from(size[1].max(1));
    [
        u32::try_from(index % wide).unwrap_or_default(),
        u32::try_from((index / wide) % deep).unwrap_or_default(),
        u32::try_from(index / wide / deep).unwrap_or_default(),
    ]
}

#[must_use]
pub fn place(point: [u32; 3], sides: [u32; 3], lattice: [u32; 3]) -> (usize, usize) {
    let mut brick = 0_usize;
    let mut offset = 0_usize;
    for axis in (0..3).rev() {
        let side = sides.get(axis).copied().unwrap_or(1).max(1) as usize;
        let across = lattice.get(axis).copied().unwrap_or(1) as usize;
        let at = point.get(axis).copied().unwrap_or_default() as usize;
        brick = brick * across + at / side;
        offset = offset * side + at % side;
    }
    (brick, offset)
}

#[must_use]
pub fn bricked(cells: &[u32], size: [u32; 3], sides: [u32; 3], capacity: Option<u32>) -> State {
    bricked_planes(cells, 1, size, sides, capacity)
}

#[must_use]
pub fn bricked_planes(
    cells: &[u32],
    planes: u32,
    size: [u32; 3],
    sides: [u32; 3],
    capacity: Option<u32>,
) -> State {
    let lattice = lattice_of(size, sides);
    let per = brick_cells(sides);
    let count = lattice.iter().map(|&side| side as u64).product::<u64>();
    let total = size.iter().map(|&side| u64::from(side)).product::<u64>();
    let planes = u64::from(planes.max(1));
    let mut table = Vec::new();
    let mut values: Vec<Vec<u32>> = vec![Vec::new(); usize::try_from(planes - 1).unwrap_or(0)];
    let mut pools: Vec<Vec<u32>> = vec![Vec::new(); usize::try_from(planes).unwrap_or(1)];
    let mut held = 0_u32;
    let mut words = vec![vec![0_u32; per]; usize::try_from(planes).unwrap_or(1)];
    for brick in 0..count {
        let origin = point_of(lattice, brick);
        for (plane, plane_words) in (0_u64..).zip(words.iter_mut()) {
            for (offset, slot) in (0_u64..).zip(plane_words.iter_mut()) {
                let within = point_of(sides, offset);
                let point = [
                    origin[0] * sides[0] + within[0],
                    origin[1] * sides[1] + within[1],
                    origin[2] * sides[2] + within[2],
                ];
                let index = plane * total
                    + (u64::from(point[2]) * u64::from(size[1]) + u64::from(point[1]))
                        * u64::from(size[0])
                    + u64::from(point[0]);
                *slot = usize::try_from(index)
                    .ok()
                    .and_then(|index| cells.get(index))
                    .copied()
                    .unwrap_or_default();
            }
        }
        let uniform: Vec<Option<u32>> = words
            .iter()
            .map(|plane_words| {
                let first = plane_words.first().copied().unwrap_or_default();
                plane_words
                    .iter()
                    .all(|&word| word == first)
                    .then_some(first)
            })
            .collect();
        if uniform.iter().all(Option::is_some) {
            let mut given = uniform.into_iter().flatten();
            table.extend([UNIFORM, given.next().unwrap_or_default()]);
            for (plane_values, value) in values.iter_mut().zip(given) {
                plane_values.push(value);
            }
            continue;
        }
        table.extend([held, 0]);
        for plane_values in &mut values {
            plane_values.push(0);
        }
        for (pool, plane_words) in pools.iter_mut().zip(&words) {
            pool.extend_from_slice(plane_words);
        }
        held += 1;
    }
    table.extend(values.concat());
    State::Sparse {
        capacity: capacity.unwrap_or(held).max(held).max(1),
        sides,
        table,
        pool: pools.concat(),
    }
}

struct Sealing<W> {
    out: W,
    hash: Sha256,
    written: u64,
    bytes: Vec<u8>,
}

impl<W: Write> Sealing<W> {
    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.hash.update(bytes);
        self.out.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn word(&mut self, word: u32) -> io::Result<()> {
        self.put(&word.to_le_bytes())
    }

    fn words(&mut self, words: &[u32]) -> io::Result<()> {
        for chunk in words.chunks(CHUNK) {
            let mut bytes = std::mem::take(&mut self.bytes);
            bytes.clear();
            bytes.extend(chunk.iter().flat_map(|word| word.to_le_bytes()));
            let done = self.put(&bytes);
            self.bytes = bytes;
            done?;
        }
        Ok(())
    }

    fn section(&mut self, tag: [u8; 4], words: &[u32]) -> io::Result<()> {
        self.put(&tag)?;
        self.put(&(words.len() as u64).to_le_bytes())?;
        self.words(words)
    }
}

#[must_use]
pub fn text_words(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut words = vec![u32::try_from(bytes.len()).unwrap_or(u32::MAX)];
    for chunk in bytes.chunks(4) {
        let mut four = [0_u8; 4];
        if let Some(room) = four.get_mut(..chunk.len()) {
            room.copy_from_slice(chunk);
        }
        words.push(u32::from_le_bytes(four));
    }
    words
}

pub fn encode<W: Write>(image: &Image, out: W) -> io::Result<(W, u64)> {
    let mut sealing = Sealing {
        out,
        hash: Sha256::new(),
        written: 0,
        bytes: Vec::with_capacity(CHUNK * 4),
    };
    let (sparse, capacity, sides) = match &image.state {
        State::Dense(_) => (0, 0, [0; 3]),
        State::Sparse {
            capacity, sides, ..
        } => (1, *capacity, *sides),
    };
    sealing.put(&MAGIC)?;
    sealing.word(VERSION)?;
    sealing.put(&image.law_hash())?;
    sealing.word(u32::try_from(image.params.len()).unwrap_or(u32::MAX))?;
    sealing.words(&image.params)?;
    sealing.word(image.frame)?;
    sealing.words(&image.size)?;
    sealing.word(image.channels())?;
    sealing.word(sparse)?;
    sealing.word(capacity)?;
    sealing.words(&sides)?;
    sealing.section(
        image.law.tag(),
        &text_words(&image.law.canonical(image.params.len())),
    )?;
    match &image.state {
        State::Dense(cells) => sealing.section(DENSE, cells)?,
        State::Sparse { table, pool, .. } => {
            sealing.section(TABLE, table)?;
            sealing.section(POOL, pool)?;
        }
    }
    let digest = std::mem::take(&mut sealing.hash).finish();
    sealing.put(&SUM)?;
    sealing.put(&8_u64.to_le_bytes())?;
    sealing.put(&digest)?;
    Ok((sealing.out, sealing.written))
}

#[must_use]
pub fn encoded(image: &Image) -> Vec<u8> {
    match encode(image, Vec::new()) {
        Ok((bytes, _)) => bytes,
        Err(_) => Vec::new(),
    }
}

fn failed(why: &io::Error) -> Refused {
    Refused::Io {
        what: why.to_string(),
    }
}

struct Taking<R> {
    input: R,
    hash: Sha256,
    at: u64,
    length: u64,
    bytes: Vec<u8>,
}

impl<R: Read> Taking<R> {
    fn raw(&mut self, into: &mut [u8], reading: &'static str) -> Result<(), Refused> {
        let wanted = into.len() as u64;
        if self.length.saturating_sub(self.at) < wanted {
            return Err(Refused::Truncated {
                at: self.length,
                reading,
            });
        }
        match self.input.read_exact(into) {
            Ok(()) => {
                self.at += wanted;
                Ok(())
            }
            Err(why) if why.kind() == io::ErrorKind::UnexpectedEof => Err(Refused::Truncated {
                at: self.at,
                reading,
            }),
            Err(why) => Err(failed(&why)),
        }
    }

    fn take<const N: usize>(&mut self, reading: &'static str) -> Result<[u8; N], Refused> {
        let mut out = [0_u8; N];
        self.raw(&mut out, reading)?;
        self.hash.update(&out);
        Ok(out)
    }

    fn word(&mut self, reading: &'static str) -> Result<u32, Refused> {
        Ok(u32::from_le_bytes(self.take::<4>(reading)?))
    }

    fn words(&mut self, count: u64, reading: &'static str) -> Result<Vec<u32>, Refused> {
        let room = self.length.saturating_sub(self.at) / 4;
        if count > room {
            return Err(Refused::Truncated {
                at: self.length,
                reading,
            });
        }
        let count = usize::try_from(count).map_err(|_| Refused::Truncated {
            at: self.length,
            reading,
        })?;
        let mut out = Vec::with_capacity(count);
        let mut bytes = std::mem::take(&mut self.bytes);
        while out.len() < count {
            let run = CHUNK.min(count - out.len());
            bytes.resize(run * 4, 0);
            let done = self.raw(&mut bytes, reading);
            if let Err(why) = done {
                self.bytes = bytes;
                return Err(why);
            }
            self.hash.update(&bytes);
            out.extend(
                bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|four| u32::from_le_bytes(*four)),
            );
        }
        self.bytes = bytes;
        Ok(out)
    }
}

struct Header {
    law: [u8; 32],
    params: Vec<u32>,
    frame: u32,
    size: [u32; 3],
    channels: u32,
    sparse: u32,
    capacity: u32,
    sides: [u32; 3],
}

fn malformed(what: impl Into<String>) -> Refused {
    Refused::Malformed { what: what.into() }
}

fn sized(what: &'static str, header: u64, found: u64) -> Refused {
    Refused::Sizes {
        what,
        header,
        found,
    }
}

pub fn decode<R: Read>(input: R, length: u64) -> Result<Image, Refused> {
    let mut taking = Taking {
        input,
        hash: Sha256::new(),
        at: 0,
        length,
        bytes: Vec::new(),
    };
    let present = usize::try_from(length.min(MAGIC.len() as u64)).unwrap_or_default();
    let mut magic = vec![0_u8; present];
    taking.raw(&mut magic, "the magic")?;
    taking.hash.update(&magic);
    if MAGIC.get(..present) != Some(magic.as_slice()) {
        return Err(Refused::Magic);
    }
    if present < MAGIC.len() {
        return Err(Refused::Truncated {
            at: length,
            reading: "the magic",
        });
    }
    let version = taking.word("the version")?;
    if version != VERSION {
        return Err(Refused::Version { found: version });
    }
    let law = taking.take::<32>("the law hash")?;
    let count = taking.word("the parameter count")?;
    let params = taking.words(u64::from(count), "the parameters")?;
    let frame = taking.word("the frame")?;
    let size = [
        taking.word("the size")?,
        taking.word("the size")?,
        taking.word("the size")?,
    ];
    let channels = taking.word("the channels")?;
    let sparse = taking.word("the sparse flag")?;
    let capacity = taking.word("the capacity")?;
    let sides = [
        taking.word("the brick sides")?,
        taking.word("the brick sides")?,
        taking.word("the brick sides")?,
    ];
    let header = Header {
        law,
        params,
        frame,
        size,
        channels,
        sparse,
        capacity,
        sides,
    };
    let mut sections: Vec<([u8; 4], Vec<u32>)> = Vec::new();
    loop {
        let mut tag = [0_u8; 4];
        taking.raw(&mut tag, "a section's tag")?;
        if tag == SUM {
            break;
        }
        taking.hash.update(&tag);
        let words = u64::from_le_bytes(taking.take::<8>("a section's length")?);
        let payload = taking.words(words, "a section")?;
        sections.push((tag, payload));
    }
    let digest = std::mem::take(&mut taking.hash).finish();
    let mut length_bytes = [0_u8; 8];
    taking.raw(&mut length_bytes, "the checksum's length")?;
    let mut stored = [0_u8; 32];
    taking.raw(&mut stored, "the checksum")?;
    if taking.at < length {
        return Err(Refused::Trailing { at: taking.at });
    }
    if u64::from_le_bytes(length_bytes) != 8 || stored != digest {
        return Err(Refused::Checksum);
    }
    judged(header, sections)
}

fn judged(header: Header, sections: Vec<([u8; 4], Vec<u32>)>) -> Result<Image, Refused> {
    if let Some((tag, _)) = sections
        .iter()
        .find(|(tag, _)| ![RULE, LAW, TABLE, POOL, DENSE].contains(tag))
    {
        return Err(Refused::Section { tag: *tag });
    }
    if header.channels == 0 || header.channels > MOST_CHANNELS {
        return Err(malformed(format!(
            "the world has {} channels, and version {VERSION} holds 1 to {MOST_CHANNELS}",
            header.channels
        )));
    }
    let tags: Vec<[u8; 4]> = sections.iter().map(|(tag, _)| *tag).collect();
    let lawful = tags.first() == Some(&LAW);
    let wanted: &[[u8; 4]] = match (header.sparse, lawful) {
        (0, false) => &[RULE, DENSE],
        (1, false) => &[RULE, TABLE, POOL],
        (0, true) => &[LAW, DENSE],
        (1, true) => &[LAW, TABLE, POOL],
        (other, _) => {
            return Err(malformed(format!(
                "the sparse flag is {other}, which is not 0 or 1"
            )));
        }
    };
    if tags != wanted {
        return Err(malformed(format!(
            "the sections are {}, and a {} world has {}",
            named(&tags),
            if header.sparse == 1 {
                "sparse"
            } else {
                "dense"
            },
            named(wanted)
        )));
    }
    let mut payloads = sections.into_iter().map(|(_, payload)| payload);
    let rule = payloads.next().unwrap_or_default();
    let text = law_text(&rule)?;
    if sha256(text.as_bytes()) != header.law {
        return Err(Refused::LawHash);
    }
    let (law, count, channels) = if lawful {
        law_compiled(&text)?
    } else {
        let (name, count) = law_read(&text)?;
        (Law::Rule { name }, count, CHANNELS)
    };
    if count != header.params.len() as u64 {
        return Err(sized("params", header.params.len() as u64, count));
    }
    if law.canonical(header.params.len()) != text {
        return Err(malformed("the law section is not canonical JSON"));
    }
    if channels != header.channels {
        return Err(sized(
            "channels",
            u64::from(header.channels),
            u64::from(channels),
        ));
    }
    for side in header.size {
        if side < 2 || !side.is_power_of_two() {
            return Err(malformed(format!(
                "a side of {side} cells is not a power of two of at least 2"
            )));
        }
    }
    let cells = header
        .size
        .iter()
        .map(|&side| u64::from(side))
        .product::<u64>();
    let state = if header.sparse == 0 {
        if header.capacity != 0 || header.sides != [0; 3] {
            return Err(malformed(
                "a dense world has no capacity and no brick sides",
            ));
        }
        let dense = payloads.next().unwrap_or_default();
        let planes = cells * u64::from(header.channels);
        if dense.len() as u64 != planes {
            return Err(sized("dense", planes, dense.len() as u64));
        }
        State::Dense(dense)
    } else {
        sparse_state(&header, cells, payloads.next(), payloads.next())?
    };
    Ok(Image {
        law,
        params: header.params,
        frame: header.frame,
        size: header.size,
        state,
    })
}

fn sparse_state(
    header: &Header,
    cells: u64,
    table: Option<Vec<u32>>,
    pool: Option<Vec<u32>>,
) -> Result<State, Refused> {
    let table = table.unwrap_or_default();
    let pool = pool.unwrap_or_default();
    for (side, world) in header.sides.into_iter().zip(header.size) {
        if side < 2 || !side.is_power_of_two() || side > world {
            return Err(malformed(format!(
                "a brick side of {side} is not a power of two from 2 to the world's side of {world}"
            )));
        }
    }
    let per = header
        .sides
        .iter()
        .map(|&side| u64::from(side))
        .product::<u64>();
    let bricks = cells / per;
    let planes = u64::from(header.channels);
    if table.len() as u64 != bricks * (planes + 1) {
        return Err(sized("table", bricks * (planes + 1), table.len() as u64));
    }
    let heads = usize::try_from(bricks * 2).unwrap_or(usize::MAX);
    let (entries, upper) = table.split_at(heads.min(table.len()));
    let mut held = 0_u32;
    for (index, &[slot, value]) in entries.as_chunks::<2>().0.iter().enumerate() {
        if slot != UNIFORM
            && upper
                .iter()
                .skip(index)
                .step_by(usize::try_from(bricks).unwrap_or(1).max(1))
                .any(|&value| value != 0)
        {
            return Err(malformed(format!(
                "held brick {index} has a value in a plane past the first, where 0 is canonical"
            )));
        }
        if slot == UNIFORM {
            continue;
        }
        if slot != held || value != 0 {
            return Err(malformed(format!(
                "a held brick names slot {slot} with value {value}, where slot {held} with value 0 is next"
            )));
        }
        held += 1;
    }
    if header.capacity < held.max(1) || u64::from(header.capacity) > bricks {
        return Err(sized(
            "capacity",
            u64::from(header.capacity),
            u64::from(held),
        ));
    }
    if pool.len() as u64 != u64::from(held) * per * planes {
        return Err(sized(
            "pool",
            u64::from(held) * per * planes,
            pool.len() as u64,
        ));
    }
    Ok(State::Sparse {
        capacity: header.capacity,
        sides: header.sides,
        table,
        pool,
    })
}

fn named(tags: &[[u8; 4]]) -> String {
    tags.iter()
        .map(|tag| String::from_utf8_lossy(tag).trim().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

fn law_text(words: &[u32]) -> Result<String, Refused> {
    let (&length, rest) = words
        .split_first()
        .ok_or_else(|| malformed("the law section is empty"))?;
    let length = length as usize;
    if rest.len() != length.div_ceil(4) {
        return Err(sized("law", length.div_ceil(4) as u64, rest.len() as u64));
    }
    let bytes: Vec<u8> = rest.iter().flat_map(|word| word.to_le_bytes()).collect();
    let (text, padding) = bytes.split_at(length.min(bytes.len()));
    if padding.iter().any(|&byte| byte != 0) {
        return Err(malformed("the law section's padding is not zero"));
    }
    String::from_utf8(text.to_vec()).map_err(|_| malformed("the law section is not UTF-8"))
}

fn law_compiled(text: &str) -> Result<(Law, u64, u32), Refused> {
    let law = crate::law::Law::compile(text)
        .map_err(|why| malformed(format!("the law section: {why}")))?;
    let count = crate::rule3::Declared::params(&law) as u64;
    Ok((
        Law::Law {
            name: law.name().to_owned(),
            canonical: law.canonical().to_owned(),
        },
        count,
        law.channels(),
    ))
}

fn law_read(text: &str) -> Result<(String, u64), Refused> {
    let held = wire::read(text).map_err(|why| malformed(format!("the law section: {why}")))?;
    let name = held
        .get("rule")
        .and_then(Json::text)
        .ok_or_else(|| malformed("the law section names no rule"))?;
    let count = held
        .get("params")
        .and_then(Json::int)
        .and_then(|count| u64::try_from(count).ok())
        .ok_or_else(|| malformed("the law section gives no parameter count"))?;
    Ok((name.to_owned(), count))
}

pub fn decoded(bytes: &[u8]) -> Result<Image, Refused> {
    decode(bytes, bytes.len() as u64)
}

pub fn fetch(path: &Path) -> Result<Image, Refused> {
    let file = File::open(path).map_err(|why| failed(&why))?;
    let length = file.metadata().map_err(|why| failed(&why))?.len();
    decode(BufReader::with_capacity(1 << 20, file), length)
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn temporary(path: &Path) -> PathBuf {
    let name = path.file_name().map_or_else(
        || "world".into(),
        |name| name.to_string_lossy().into_owned(),
    );
    let count = TEMPORARY.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{count}.tmp", std::process::id()))
}

pub fn store(path: &Path, image: &Image) -> Result<u64, Refused> {
    store_with(path, image, |file| file)
}

pub fn store_with<W, F>(path: &Path, image: &Image, wrap: F) -> Result<u64, Refused>
where
    W: Write,
    F: FnOnce(File) -> W,
{
    let temporary = temporary(path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|why| failed(&why))?;
    let written = (|| -> io::Result<u64> {
        let out = BufWriter::with_capacity(1 << 20, wrap(file));
        let (out, written) = encode(image, out)?;
        let inner = out.into_inner().map_err(io::IntoInnerError::into_error)?;
        drop(inner);
        OpenOptions::new()
            .write(true)
            .open(&temporary)?
            .sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(written)
    })();
    match written {
        Ok(written) => {
            settled(path);
            Ok(written)
        }
        Err(why) => {
            let _ = fs::remove_file(&temporary);
            Err(failed(&why))
        }
    }
}

#[cfg(unix)]
fn settled(path: &Path) {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        let _ = File::open(folder).and_then(|held| held.sync_all());
    }
}

#[cfg(not(unix))]
const fn settled(_: &Path) {}
