use std::collections::HashMap;

use crate::law::Law;
use crate::wire::{Json, counted};

use super::Failed;
use super::errors::{Code, Refusal};
use super::page::{self, Format, Unpacked};
use super::tools::{int, ints, limited, refused, sized, unread};

pub const DRIVEN: &str = "driven names cells of one plane and their words at each frame the call runs: plane, or current true for an iterating law's current plane at each frame; cells as [x, y, z] each named once, or every cell of the box from to to, x fastest; values, one word list a frame in the order of the cells, or data, one page of the cells of every frame in turn (format words or bits). Before each frame the engine writes that frame's words to those cells, as grit_world_write would, then steps the frame, so the law sees them as that frame's state; the cells times the frames are at most one page";

pub const DRIVEN_STEP: &str = "before each frame that ran, that frame's words were written to the driven cells, priced and journaled as grit_world_write of those cells, and the frame then ran on its own; words counts the words written. After the step the plane driven at the last frame holds its words";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Plane(u32),
    Current,
}

#[derive(Clone, Debug)]
pub struct Driven {
    pub side: Option<Side>,
    pub cells: Vec<[u32; 3]>,
    pub values: Vec<Vec<u32>>,
}

impl Driven {
    #[must_use]
    pub fn points(&self, at: usize, origin: [u32; 3]) -> Vec<([u32; 3], u32)> {
        let row = self.values.get(at).map_or(&[][..], Vec::as_slice);
        self.cells
            .iter()
            .zip(row)
            .map(|(cell, word)| {
                (
                    [
                        cell[0] + origin[0],
                        cell[1] + origin[1],
                        cell[2] + origin[2],
                    ],
                    *word,
                )
            })
            .collect()
    }

    #[must_use]
    pub fn indices(&self, size: [u32; 3]) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .cells
            .iter()
            .map(|cell| {
                (cell[2] as usize * size[1] as usize + cell[1] as usize) * size[0] as usize
                    + cell[0] as usize
            })
            .collect();
        out.sort_unstable();
        out
    }

    #[must_use]
    pub fn words(&self) -> u64 {
        (self.cells.len() as u64).saturating_mul(self.values.len() as u64)
    }
}

fn corner_of(held: &Json, key: &str, field: &str, size: [u32; 3]) -> Result<[u32; 3], Failed> {
    let got = ints(held.get(key).ok_or_else(|| unread(key))?, key)?;
    let at = <[u32; 3]>::try_from(got.as_slice()).map_err(|_| unread(key))?;
    inside_of(at, size, field)?;
    Ok(at)
}

fn inside_of(at: [u32; 3], size: [u32; 3], field: &str) -> Result<(), Failed> {
    if at.iter().zip(size).all(|(one, side)| *one < side) {
        return Ok(());
    }
    Err(limited(
        Code::Outside,
        field,
        format!("a driven cell at {at:?} is outside the {size:?} cells it drives; nothing ran"),
        sized(size.map(|side| side.saturating_sub(1))),
    ))
}

fn side_of(given: &Json, field: &str) -> Result<Option<Side>, Failed> {
    match (given.get("plane"), given.get("current")) {
        (Some(_), Some(_)) => Err(refused(
            Code::PlaneOrCurrent,
            &format!("{field}.plane"),
            "give plane or current true, not both".to_owned(),
        )),
        (Some(_), None) => Ok(Some(Side::Plane(int(given, "plane")?))),
        (None, Some(Json::Bool(true))) => Ok(Some(Side::Current)),
        (None, Some(Json::Bool(false)) | None) => Ok(None),
        (None, Some(_)) => Err(unread("current")),
    }
}

fn cells_of(given: &Json, field: &str, size: [u32; 3]) -> Result<Vec<[u32; 3]>, Failed> {
    let cells = match (given.get("cells"), given.get("from"), given.get("to")) {
        (Some(listed), None, None) => {
            let list = listed.list().ok_or_else(|| unread("cells"))?;
            let mut seen: HashMap<[u32; 3], usize> = HashMap::with_capacity(list.len());
            let mut cells = Vec::with_capacity(list.len());
            for (at, one) in list.iter().enumerate() {
                let got = ints(one, "cells")?;
                let cell = <[u32; 3]>::try_from(got.as_slice()).map_err(|_| unread("cells"))?;
                let path = format!("{field}.cells[{at}]");
                inside_of(cell, size, &path)?;
                if let Some(before) = seen.insert(cell, at) {
                    return Err(refused(
                        Code::Malformed,
                        &path,
                        format!(
                            "cell {at} at {cell:?} is named twice, first as cell {before}; each driven cell takes one word a frame"
                        ),
                    ));
                }
                cells.push(cell);
            }
            cells
        }
        (None, Some(_), Some(_)) => {
            let from = corner_of(given, "from", &format!("{field}.from"), size)?;
            let to = corner_of(given, "to", &format!("{field}.to"), size)?;
            if from.iter().zip(to).any(|(one, other)| *one > other) {
                return Err(refused(
                    Code::Reversed,
                    &format!("{field}.to"),
                    format!("the box from {from:?} to {to:?} has its first corner past its last"),
                ));
            }
            page::box_points(from, to)
        }
        _ => {
            return Err(refused(
                Code::Malformed,
                field,
                "driven names its cells as cells, or as a box with from and to; give one of them"
                    .to_owned(),
            ));
        }
    };
    if cells.is_empty() {
        return Err(refused(
            Code::Malformed,
            &format!("{field}.cells"),
            "driven names no cell; give at least one".to_owned(),
        ));
    }
    Ok(cells)
}

fn listed_values(
    list: &[Json],
    field: &str,
    cells: usize,
    count: u32,
) -> Result<Vec<Vec<u32>>, Failed> {
    if list.len() != count as usize {
        return Err(limited(
            Code::DrivenFrames,
            &format!("{field}.values"),
            format!(
                "values holds {} word lists, and the call runs {count} frames, one list each; nothing ran",
                list.len()
            ),
            counted(count),
        ));
    }
    let mut out = Vec::with_capacity(list.len());
    for (at, one) in list.iter().enumerate() {
        let words = ints(one, "values")?;
        if words.len() != cells {
            return Err(limited(
                Code::DrivenLength,
                &format!("{field}.values[{at}]"),
                format!(
                    "frame {at} holds {} words, and driven names {cells} cells, one word each; nothing ran",
                    words.len()
                ),
                Json::Int(i64::try_from(cells).unwrap_or(i64::MAX)),
            ));
        }
        out.push(words);
    }
    Ok(out)
}

fn paged_values(
    given: &Json,
    field: &str,
    format: Format,
    cells: usize,
    count: u32,
) -> Result<Vec<Vec<u32>>, Failed> {
    let path = format!("{field}.data");
    let data = given
        .get("data")
        .and_then(Json::text)
        .ok_or_else(|| unread("data"))?;
    let bytes = page::decoded(data)
        .map_err(|why| Box::new(Refusal::new(Code::Malformed, &path, why)) as Failed)?;
    let total = (cells as u64).saturating_mul(u64::from(count));
    let row = (cells as u64).saturating_mul(4);
    let words = page::unpacked(format, &bytes, total).map_err(|why| {
        let refusal = match why {
            Unpacked::Length { wanted, got }
                if format == Format::Words && row > 0 && got % row == 0 =>
            {
                Refusal::new(
                    Code::DrivenFrames,
                    &path,
                    format!(
                        "data holds the words of {} frames of the {cells} driven cells, and the call runs {count} frames; nothing ran",
                        got / row
                    ),
                )
                .limited(Json::Int(i64::try_from(wanted).unwrap_or(i64::MAX)))
            }
            Unpacked::Length { wanted, got } => Refusal::new(
                Code::DrivenLength,
                &path,
                format!(
                    "data holds {got} bytes, and {cells} driven cells for {count} frames as {} need exactly {wanted}; nothing ran",
                    format.name()
                ),
            )
            .limited(Json::Int(i64::try_from(wanted).unwrap_or(i64::MAX))),
            Unpacked::Past { cell } => Refusal::new(
                Code::DrivenLength,
                &path,
                format!(
                    "the last word sets cell {cell}, past the {total} cells of every frame; the unused bits of the last word must be 0, and nothing ran"
                ),
            )
            .limited(Json::Int(i64::try_from(total).unwrap_or(i64::MAX))),
        };
        Box::new(refusal) as Failed
    })?;
    Ok(words.chunks(cells.max(1)).map(<[u32]>::to_vec).collect())
}

pub fn driven_of(
    held: &Json,
    at: &str,
    size: [u32; 3],
    count: u32,
) -> Result<Option<Driven>, Failed> {
    let Some(given) = held.get("driven") else {
        return Ok(None);
    };
    let field = format!("{at}.driven");
    if !matches!(given, Json::Map(_)) {
        return Err(refused(
            Code::Malformed,
            &field,
            "driven is an object: plane or current, cells or from and to, and values or data"
                .to_owned(),
        ));
    }
    let side = side_of(given, &field)?;
    let cells = cells_of(given, &field, size)?;
    let format = page::format_of(given, &format!("{field}.format"))?;
    let total = (cells.len() as u64).saturating_mul(u64::from(count));
    if total > format.most() {
        return Err(limited(
            Code::PageLimit,
            &field,
            format!(
                "driven holds {} cells for {count} frames, {total} words, more than the {} one {} page holds; drive fewer cells or run fewer frames a call",
                cells.len(),
                format.most(),
                format.name()
            ),
            Json::Int(i64::try_from(format.most()).unwrap_or(i64::MAX)),
        ));
    }
    let values = match (given.get("values"), given.get("data")) {
        (Some(listed), None) => {
            if given.get("format").is_some() {
                return Err(refused(
                    Code::Malformed,
                    &format!("{field}.format"),
                    "format goes with data; values holds whole words".to_owned(),
                ));
            }
            let list = listed.list().ok_or_else(|| unread("values"))?;
            listed_values(list, &field, cells.len(), count)?
        }
        (None, Some(_)) => paged_values(given, &field, format, cells.len(), count)?,
        _ => {
            return Err(refused(
                Code::Malformed,
                &field,
                "driven gives its words as values, one list a frame, or as data, one page of every frame; give one of them".to_owned(),
            ));
        }
    };
    Ok(Some(Driven {
        side,
        cells,
        values,
    }))
}

fn swapped(law: &Law, plane: u32) -> u32 {
    law.pairs().iter().fold(plane, |held, &[one, other]| {
        if held == one {
            other
        } else if held == other {
            one
        } else {
            held
        }
    })
}

#[must_use]
pub fn written_by(law: &Law, plane: u32) -> bool {
    let dag = law.dag();
    crate::law::alternation::written(dag, plane)
        || (!law.pairs().is_empty() && crate::law::alternation::written(dag, swapped(law, plane)))
}

pub fn side_checked(side: Side, law: Option<&Law>, planes: u32, field: &str) -> Result<(), Failed> {
    let Some(law) = law else {
        return Err(refused(
            Code::DrivenWritten,
            field,
            "a rule's step names no plane it leaves unwritten, so its writes could replace the driven words; drive a law world".to_owned(),
        ));
    };
    match side {
        Side::Current => {
            if law.alternate().is_none() {
                return Err(refused(
                    Code::NotIterating,
                    &format!("{field}.current"),
                    "the law was not given alternate, so it has no current plane".to_owned(),
                ));
            }
            Ok(())
        }
        Side::Plane(plane) => {
            if plane >= planes {
                return Err(limited(
                    Code::Outside,
                    &format!("{field}.plane"),
                    format!("plane {plane} is past the law's {planes} planes"),
                    counted(planes.saturating_sub(1)),
                ));
            }
            if written_by(law, plane) {
                return Err(refused(
                    Code::DrivenWritten,
                    &format!("{field}.plane"),
                    format!(
                        "the law writes plane {plane}, so each step would replace the driven words there; drive a plane the law only reads, or give current true on an iterating law"
                    ),
                ));
            }
            Ok(())
        }
    }
}
