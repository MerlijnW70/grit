use crate::branch::World;
use crate::wire::{Json, counted, map, text};

use super::Failed;
use super::errors::{Code, Refusal};
use super::tools::{sized, words_of};

pub const PAGE_CHARS: usize = super::PAGE_BYTES;

pub const PAGE_WORDS: u64 = (PAGE_CHARS as u64 / 4) * 3 / 4;

pub const PAGE_BITS: u64 = PAGE_WORDS * 32;

pub const PAGE: &str = "a page is one box of one plane, corners included, cells x fastest, then y, then z. format words: one 32-bit word a cell. format bits: one bit a cell, cell i at bit i % 32 of word i / 32, each cell 0 or 1; the unused bits of the last word are 0. data is the words, each 4 bytes little-endian, in standard base64 with padding. A page holds at most page_words words, so a request or an answer with one stays under line_bytes and answer_bytes. A page write is the cell list of every cell of its box, written, priced (its cells, as write_cells' cells are) and journaled as that list; a page read answers the words a dense read of the box would. A request line past line_bytes is answered line_too_long with id null, and nothing in it runs";

pub const DATA: &str = "grit_world_create with planes and neither rule nor law makes a data-only world: planes planes of words, every cell 0, under the data law of that many planes, whose step keeps every cell. It holds examples for grit_law_learn and starts and targets for grit_law_check; it reads, writes, slices, forks, saves, opens and restores as any world. grit_world_step, grit_world_back, grit_world_params and a world start of grit_law_gradient, grit_law_descend, grit_law_score or grit_learn_params refuse it with no_law";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Bits,
    Words,
}

impl Format {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bits => "bits",
            Self::Words => "words",
        }
    }

    #[must_use]
    pub const fn most(self) -> u64 {
        match self {
            Self::Bits => PAGE_BITS,
            Self::Words => PAGE_WORDS,
        }
    }

    #[must_use]
    pub const fn words(self, cells: u64) -> u64 {
        match self {
            Self::Bits => cells.div_ceil(32),
            Self::Words => cells,
        }
    }
}

pub fn format_of(held: &Json, field: &str) -> Result<Format, Failed> {
    match held.get("format").and_then(Json::text) {
        None | Some("words") => Ok(Format::Words),
        Some("bits") => Ok(Format::Bits),
        Some(other) => Err(Box::new(Refusal::new(
            Code::PageFormat,
            field,
            format!(
                "format {} is neither bits nor words",
                super::errors::shown(other)
            ),
        ))),
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[must_use]
pub fn encoded(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut held = chunk.iter().copied().map(u32::from);
        let a = held.next().unwrap_or(0);
        let b = held.next().unwrap_or(0);
        let c = held.next().unwrap_or(0);
        let joined = (a << 16) | (b << 8) | c;
        for at in 0..4 {
            if at <= chunk.len() {
                let six = (joined >> (18 - 6 * at)) & 63;
                out.push(char::from(
                    ALPHABET.get(six as usize).copied().unwrap_or(b'A'),
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn six_of(byte: u8) -> Option<u32> {
    let value = match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(value))
}

pub fn decoded(held: &str) -> Result<Vec<u8>, String> {
    let bytes = held.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "the text holds {} characters; base64 with padding holds a multiple of 4",
            bytes.len()
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let groups = bytes.len() / 4;
    for (index, group) in bytes.chunks(4).enumerate() {
        let last = index + 1 == groups;
        let pads = group.iter().rev().take_while(|one| **one == b'=').count();
        if pads > 2 || (pads > 0 && !last) {
            return Err(format!("padding stands inside the text, at group {index}"));
        }
        let mut joined = 0_u32;
        for (at, byte) in group.iter().enumerate() {
            let six = if at >= 4 - pads {
                0
            } else {
                six_of(*byte).ok_or_else(|| {
                    format!("character {} is not in the base64 alphabet", index * 4 + at)
                })?
            };
            joined = (joined << 6) | six;
        }
        let keep = 3 - pads;
        let spare = joined & ((1 << (8 * pads as u32)) - 1);
        if spare != 0 {
            return Err(format!("group {index} sets bits its padding drops"));
        }
        for at in 0..keep {
            out.push(((joined >> (16 - 8 * at)) & 0xFF) as u8);
        }
    }
    Ok(out)
}

pub fn packed(format: Format, words: &[u32]) -> Result<Vec<u8>, usize> {
    let held = match format {
        Format::Words => words.to_vec(),
        Format::Bits => {
            let mut held = vec![0_u32; words.len().div_ceil(32)];
            for (at, word) in words.iter().enumerate() {
                if *word > 1 {
                    return Err(at);
                }
                if let Some(slot) = held.get_mut(at / 32) {
                    *slot |= word << (at % 32);
                }
            }
            held
        }
    };
    Ok(held.iter().flat_map(|word| word.to_le_bytes()).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unpacked {
    Length { wanted: u64, got: u64 },
    Past { cell: u64 },
}

pub fn unpacked(format: Format, bytes: &[u8], cells: u64) -> Result<Vec<u32>, Unpacked> {
    let wanted = format.words(cells).saturating_mul(4);
    let got = bytes.len() as u64;
    if got != wanted {
        return Err(Unpacked::Length { wanted, got });
    }
    let words: Vec<u32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|four| u32::from_le_bytes(*four))
        .collect();
    match format {
        Format::Words => Ok(words),
        Format::Bits => {
            let total = usize::try_from(cells).map_err(|_| Unpacked::Length { wanted, got })?;
            let tail = cells % 32;
            if tail != 0
                && let Some(last) = words.last()
                && last >> tail != 0
            {
                let spare = last >> tail;
                return Err(Unpacked::Past {
                    cell: cells + u64::from(spare.trailing_zeros()),
                });
            }
            Ok((0..total)
                .map(|at| words.get(at / 32).map_or(0, |word| (word >> (at % 32)) & 1))
                .collect())
        }
    }
}

#[must_use]
pub fn box_points(from: [u32; 3], to: [u32; 3]) -> Vec<[u32; 3]> {
    let mut points = Vec::new();
    for z in from[2]..=to[2] {
        for y in from[1]..=to[1] {
            for x in from[0]..=to[0] {
                points.push([x, y, z]);
            }
        }
    }
    points
}

#[must_use]
pub fn box_cells(from: [u32; 3], to: [u32; 3]) -> u64 {
    from.iter()
        .zip(to)
        .map(|(one, other)| u64::from(other.saturating_sub(*one)) + 1)
        .product()
}

pub fn limit_checked(format: Format, cells: u64, field: &str) -> Result<(), Failed> {
    if cells <= format.most() {
        return Ok(());
    }
    Err(Box::new(
        Refusal::new(
            Code::PageLimit,
            field,
            format!(
                "the box holds {cells} cells, more than the {} one {} page holds within the line and answer limits; split it into boxes of at most that many",
                format.most(),
                format.name()
            ),
        )
        .limited(Json::Int(i64::try_from(format.most()).unwrap_or(i64::MAX))),
    ))
}

pub fn words_from(
    held: &Json,
    format: Format,
    cells: u64,
    field: &str,
) -> Result<Vec<u32>, Failed> {
    let data = words_of(held, "data")?;
    let bytes = decoded(data)
        .map_err(|why| Box::new(Refusal::new(Code::Malformed, field, why)) as Failed)?;
    unpacked(format, &bytes, cells).map_err(|why| {
        let refusal = match why {
            Unpacked::Length { wanted, got } => Refusal::new(
                Code::PageLength,
                field,
                format!(
                    "the page holds {got} bytes, and the box's {cells} cells as {} need exactly {wanted}; nothing was written",
                    format.name()
                ),
            )
            .limited(Json::Int(i64::try_from(wanted).unwrap_or(i64::MAX))),
            Unpacked::Past { cell } => Refusal::new(
                Code::PageLength,
                field,
                format!(
                    "the last word sets bit {}, cell {cell}, past the box's {cells} cells; the unused bits of the last word must be 0, and nothing was written",
                    cell % 32
                ),
            )
            .limited(Json::Int(i64::try_from(cells).unwrap_or(i64::MAX))),
        };
        Box::new(refusal) as Failed
    })
}

#[must_use]
pub fn data_law(planes: u32) -> String {
    format!(
        "{{\"law\":1,\"name\":\"data\",\"params\":0,\"channels\":{planes},\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",\"cells\",[\"vec\",0]]}}"
    )
}

#[must_use]
pub fn data_only(world: &World<'_>) -> bool {
    world.law().is_some_and(|law| {
        crate::law::Law::compile(&data_law(world.planes()))
            .is_ok_and(|data| data.canonical() == law.canonical())
    })
}

pub fn lawful_checked(world: &World<'_>, number: u32, field: &str) -> Result<(), Failed> {
    if !data_only(world) {
        return Ok(());
    }
    Err(Box::new(Refusal::new(
        Code::NoLaw,
        field,
        format!(
            "world {number} is a data-only world: it holds {} planes of data and no law, so nothing runs on it; create a world with a law and write these cells to it, or use it as examples, starts or targets",
            world.planes()
        ),
    )))
}

#[must_use]
pub fn limits_shown() -> Json {
    map(vec![
        (
            "words",
            Json::Int(i64::try_from(PAGE_WORDS).unwrap_or(i64::MAX)),
        ),
        (
            "bits",
            Json::Int(i64::try_from(PAGE_BITS).unwrap_or(i64::MAX)),
        ),
        (
            "data_chars",
            Json::Int(i64::try_from(PAGE_CHARS).unwrap_or(i64::MAX)),
        ),
        ("means", text(PAGE)),
    ])
}

pub fn page_answer(
    world: &mut World<'_>,
    held: &Json,
    (from, to): ([u32; 3], [u32; 3]),
    plane: u32,
) -> Result<Vec<(&'static str, Json)>, Failed> {
    let format = format_of(held, "$.format")?;
    let cells = box_cells(from, to);
    limit_checked(format, cells, "$.to")?;
    let words = world.cells_at_plane(&box_points(from, to), plane)?;
    let bytes = packed(format, &words).map_err(|at| {
        let width = u64::from(to[0] - from[0]) + 1;
        let deep = u64::from(to[1] - from[1]) + 1;
        let index = at as u64;
        let spot = [
            from[0] as u64 + index % width,
            from[1] as u64 + index / width % deep,
            from[2] as u64 + index / width / deep,
        ];
        Box::new(
            Refusal::new(
                Code::PageFormat,
                "$.format",
                format!(
                    "the cell at {spot:?} holds {}, which one bit cannot hold; read it with format words",
                    words.get(at).copied().unwrap_or_default()
                ),
            )
            .limited(counted(1)),
        ) as Failed
    })?;
    let count = format.words(cells);
    Ok(vec![
        ("from", sized(from)),
        ("to", sized(to)),
        ("form", text("page")),
        ("format", text(format.name())),
        ("cells", Json::Int(i64::try_from(cells).unwrap_or(i64::MAX))),
        ("words", Json::Int(i64::try_from(count).unwrap_or(i64::MAX))),
        ("data", text(&encoded(&bytes))),
    ])
}
