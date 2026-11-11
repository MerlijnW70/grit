//! A reader of binary glTF 2.0: the triangles of every mesh the scene
//! places, each primitive with the name of its material.

use super::Error;
use super::json::{self, Value};

/// The first four bytes of a binary glTF file.
pub const MAGIC: [u8; 4] = *b"glTF";

/// The container version this reader takes.
pub const VERSION: u32 = 2;

const HEADER: usize = 12;
const CHUNK_HEADER: usize = 8;
const JSON_CHUNK: u32 = 0x4E4F_534A;
const BIN_CHUNK: u32 = 0x004E_4942;

const FLOAT: u64 = 5126;
const UNSIGNED_BYTE: u64 = 5121;
const UNSIGNED_SHORT: u64 = 5123;
const UNSIGNED_INT: u64 = 5125;

const TRIANGLES: u64 = 4;
const STRIP: u64 = 5;
const FAN: u64 = 6;

/// One primitive of a mesh as a node places it: its triangles in the
/// scene's frame, in glTF's axes (y up), and the name of its material,
/// `None` when it has no material or the material has no name.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub material: Option<String>,
    pub triangles: Vec<[[f64; 3]; 3]>,
}

/// Every part of a model, in the order the scene places them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub parts: Vec<Part>,
}

type Matrix = [[f64; 4]; 4];

const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn times(left: &Matrix, right: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for (column, out_column) in right.iter().zip(out.iter_mut()) {
        for (row, cell) in out_column.iter_mut().enumerate() {
            *cell = left
                .iter()
                .zip(column)
                .map(|(left_column, scale)| left_column.get(row).copied().unwrap_or(0.0) * scale)
                .sum();
        }
    }
    out
}

fn applied(matrix: &Matrix, point: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = point;
    let [c0, c1, c2, c3] = matrix;
    [
        c0[0] * x + c1[0] * y + c2[0] * z + c3[0],
        c0[1] * x + c1[1] * y + c2[1] * z + c3[1],
        c0[2] * x + c1[2] * y + c2[2] * z + c3[2],
    ]
}

fn malformed(what: String) -> Error {
    Error::Malformed { what }
}

fn u32_at(bytes: &[u8], at: usize, what: &'static str) -> Result<u32, Error> {
    let held = bytes
        .get(at..at + 4)
        .and_then(|four| <[u8; 4]>::try_from(four).ok())
        .ok_or(Error::Truncated {
            what: what.to_owned(),
            needed: (at + 4) as u64,
            held: bytes.len() as u64,
        })?;
    Ok(u32::from_le_bytes(held))
}

fn list<'held>(json: &'held Value, key: &str) -> Result<&'held [Value], Error> {
    match json.get(key) {
        None => Ok(&[]),
        Some(one) => one
            .list()
            .ok_or_else(|| malformed(format!("{key} is not a list"))),
    }
}

fn whole(held: &Value, key: &str, what: &str) -> Result<Option<u64>, Error> {
    held.get(key).map_or(Ok(None), |one| {
        one.whole()
            .map(Some)
            .ok_or_else(|| malformed(format!("{what} {key} is not a whole number")))
    })
}

fn needed(held: &Value, key: &str, what: &str) -> Result<u64, Error> {
    whole(held, key, what)?.ok_or_else(|| Error::Missing {
        what: format!("{what} {key}"),
    })
}

fn pick(items: &[Value], index: u64, what: impl Into<String>) -> Result<&Value, Error> {
    usize::try_from(index)
        .ok()
        .and_then(|at| items.get(at))
        .ok_or_else(|| Error::OutOfRange {
            what: what.into(),
            index,
            count: items.len() as u64,
        })
}

fn floats<const N: usize>(held: &Value, key: &str, what: &str) -> Result<Option<[f64; N]>, Error> {
    let Some(one) = held.get(key) else {
        return Ok(None);
    };
    let numbers: Option<Vec<f64>> = one
        .list()
        .map(|items| items.iter().map(Value::number).collect())
        .and_then(|got: Option<Vec<f64>>| got);
    numbers
        .and_then(|got| <[f64; N]>::try_from(got).ok())
        .map(Some)
        .ok_or_else(|| malformed(format!("{what} {key} is not {N} numbers")))
}

struct Chunks<'held> {
    json: &'held [u8],
    bin: Option<&'held [u8]>,
}

fn chunks(bytes: &[u8]) -> Result<Chunks<'_>, Error> {
    let magic = bytes.get(..4).ok_or(Error::Truncated {
        what: "the header".to_owned(),
        needed: HEADER as u64,
        held: bytes.len() as u64,
    })?;
    if magic != MAGIC {
        let mut found = [0; 4];
        found.copy_from_slice(magic);
        return Err(Error::BadMagic { found });
    }
    let version = u32_at(bytes, 4, "the header")?;
    if version != VERSION {
        return Err(Error::Version { found: version });
    }
    let length = u32_at(bytes, 8, "the header")? as usize;
    let whole = bytes.get(..length).ok_or(Error::Truncated {
        what: "the file".to_owned(),
        needed: length as u64,
        held: bytes.len() as u64,
    })?;
    let mut at = HEADER;
    let mut json = None;
    let mut bin = None;
    while at < whole.len() {
        let size = u32_at(whole, at, "a chunk header")? as usize;
        let kind = u32_at(whole, at + 4, "a chunk header")?;
        let from = at + CHUNK_HEADER;
        let to = from.saturating_add(size);
        let body = whole.get(from..to).ok_or(Error::Truncated {
            what: "a chunk".to_owned(),
            needed: to as u64,
            held: whole.len() as u64,
        })?;
        match (json.is_none(), kind) {
            (true, JSON_CHUNK) => json = Some(body),
            (true, _) => return Err(Error::NoJson),
            (false, BIN_CHUNK) if bin.is_none() => bin = Some(body),
            _ => {}
        }
        at = to;
    }
    Ok(Chunks {
        json: json.ok_or(Error::NoJson)?,
        bin,
    })
}

struct File<'held> {
    json: Value,
    bin: Option<&'held [u8]>,
}

enum Wanted {
    Positions,
    Indices,
}

enum Read {
    Positions(Vec<[f64; 3]>),
    Indices(Vec<u32>),
}

impl File<'_> {
    fn buffer(&self, index: u64) -> Result<&[u8], Error> {
        let buffers = list(&self.json, "buffers")?;
        let buffer = pick(buffers, index, "buffer")?;
        let what = format!("buffer {index}");
        if buffer.get("uri").is_some() || index != 0 {
            return Err(Error::Unsupported {
                what: format!("{what}, which lies outside the file"),
            });
        }
        let length = needed(buffer, "byteLength", &what)?;
        let bin = self.bin.unwrap_or(&[]);
        usize::try_from(length)
            .ok()
            .and_then(|length| bin.get(..length))
            .ok_or(Error::Truncated {
                what: format!("{what} in the binary chunk"),
                needed: length,
                held: bin.len() as u64,
            })
    }

    fn view(&self, index: u64) -> Result<(&[u8], Option<u64>), Error> {
        let views = list(&self.json, "bufferViews")?;
        let view = pick(views, index, "buffer view")?;
        let what = format!("buffer view {index}");
        let buffer = self.buffer(needed(view, "buffer", &what)?)?;
        let offset = whole(view, "byteOffset", &what)?.unwrap_or(0);
        let length = needed(view, "byteLength", &what)?;
        let end = offset.saturating_add(length);
        let bytes = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(end).ok())
            .and_then(|(from, to)| buffer.get(from..to))
            .ok_or(Error::Truncated {
                what,
                needed: end,
                held: buffer.len() as u64,
            })?;
        Ok((bytes, whole(view, "byteStride", "a buffer view")?))
    }

    fn accessor(&self, index: u64, wanted: &Wanted) -> Result<Read, Error> {
        let accessors = list(&self.json, "accessors")?;
        let accessor = pick(accessors, index, "accessor")?;
        let what = format!("accessor {index}");
        if accessor.get("sparse").is_some() {
            return Err(Error::Unsupported {
                what: format!("{what}, which is sparse"),
            });
        }
        let Some(view) = whole(accessor, "bufferView", &what)? else {
            return Err(Error::Unsupported {
                what: format!("{what}, which has no buffer view"),
            });
        };
        let component = needed(accessor, "componentType", &what)?;
        let count = needed(accessor, "count", &what)?;
        let offset = whole(accessor, "byteOffset", &what)?.unwrap_or(0);
        let shape = accessor.get("type").and_then(Value::text).unwrap_or("");
        let (size, element) = match (wanted, component, shape) {
            (Wanted::Positions, FLOAT, "VEC3") => (4, 12),
            (Wanted::Indices, UNSIGNED_BYTE, "SCALAR") => (1, 1),
            (Wanted::Indices, UNSIGNED_SHORT, "SCALAR") => (2, 2),
            (Wanted::Indices, UNSIGNED_INT, "SCALAR") => (4, 4),
            _ => {
                return Err(Error::Unsupported {
                    what: format!("{what} of component type {component} and type {shape:?}"),
                });
            }
        };
        let (bytes, stride) = self.view(view)?;
        let stride = stride.unwrap_or(element);
        if stride < element {
            return Err(malformed(format!(
                "{what} has a stride of {stride} bytes, less than its element of {element}"
            )));
        }
        let span = match count {
            0 => 0,
            _ => stride
                .checked_mul(count - 1)
                .and_then(|span| span.checked_add(element))
                .and_then(|span| span.checked_add(offset))
                .unwrap_or(u64::MAX),
        };
        if span > bytes.len() as u64 {
            return Err(Error::Truncated {
                what,
                needed: span,
                held: bytes.len() as u64,
            });
        }
        let stride = stride as usize;
        let offset = offset as usize;
        let element_at = |at: usize| -> Option<&[u8]> {
            let from = offset + at * stride;
            bytes.get(from..from + element as usize)
        };
        let word = |held: &[u8], at: usize| -> u32 {
            match size {
                1 => held.get(at).copied().map_or(0, u32::from),
                2 => held
                    .get(at * 2..at * 2 + 2)
                    .and_then(|two| <[u8; 2]>::try_from(two).ok())
                    .map_or(0, |two| u32::from(u16::from_le_bytes(two))),
                _ => held
                    .get(at * 4..at * 4 + 4)
                    .and_then(|four| <[u8; 4]>::try_from(four).ok())
                    .map_or(0, u32::from_le_bytes),
            }
        };
        let count = count as usize;
        let torn = || Error::Truncated {
            what: format!("accessor {index}"),
            needed: span,
            held: bytes.len() as u64,
        };
        match wanted {
            Wanted::Positions => {
                let mut points = Vec::with_capacity(count);
                for at in 0..count {
                    let held = element_at(at).ok_or_else(torn)?;
                    points.push([0, 1, 2].map(|axis| f64::from(f32::from_bits(word(held, axis)))));
                }
                Ok(Read::Positions(points))
            }
            Wanted::Indices => {
                let mut indices = Vec::with_capacity(count);
                for at in 0..count {
                    indices.push(word(element_at(at).ok_or_else(torn)?, 0));
                }
                Ok(Read::Indices(indices))
            }
        }
    }

    fn positions(&self, index: u64) -> Result<Vec<[f64; 3]>, Error> {
        match self.accessor(index, &Wanted::Positions)? {
            Read::Positions(points) => Ok(points),
            Read::Indices(_) => Ok(Vec::new()),
        }
    }

    fn indices(&self, index: u64) -> Result<Vec<u32>, Error> {
        match self.accessor(index, &Wanted::Indices)? {
            Read::Indices(indices) => Ok(indices),
            Read::Positions(_) => Ok(Vec::new()),
        }
    }

    fn material(&self, index: Option<u64>) -> Result<Option<String>, Error> {
        let Some(index) = index else {
            return Ok(None);
        };
        let materials = list(&self.json, "materials")?;
        let material = pick(materials, index, "material")?;
        Ok(material
            .get("name")
            .and_then(Value::text)
            .map(str::to_owned))
    }

    fn parts(&self, mesh: u64, matrix: &Matrix, into: &mut Vec<Part>) -> Result<(), Error> {
        let meshes = list(&self.json, "meshes")?;
        let held = pick(meshes, mesh, "mesh")?;
        for (number, primitive) in list(held, "primitives")?.iter().enumerate() {
            let what = format!("mesh {mesh} primitive {number}");
            let mode = whole(primitive, "mode", &what)?.unwrap_or(TRIANGLES);
            if mode < TRIANGLES {
                continue;
            }
            let position = primitive
                .get("attributes")
                .and_then(|attributes| attributes.get("POSITION"))
                .ok_or_else(|| Error::Missing {
                    what: format!("{what} POSITION"),
                })?
                .whole()
                .ok_or_else(|| malformed(format!("{what} POSITION is not a whole number")))?;
            let points = self.positions(position)?;
            let indices = match whole(primitive, "indices", &what)? {
                Some(accessor) => self.indices(accessor)?,
                None => (0..points.len())
                    .map(|at| u32::try_from(at).unwrap_or(u32::MAX))
                    .collect(),
            };
            let corners = corners(&indices, mode, &what)?;
            let mut triangles = Vec::with_capacity(corners.len());
            for corner in corners {
                let mut triangle = [[0.0; 3]; 3];
                for (slot, index) in triangle.iter_mut().zip(corner) {
                    let point = points
                        .get(index as usize)
                        .ok_or_else(|| Error::OutOfRange {
                            what: format!("vertex index of {what}"),
                            index: u64::from(index),
                            count: points.len() as u64,
                        })?;
                    *slot = applied(matrix, *point);
                }
                triangles.push(triangle);
            }
            let material = self.material(whole(primitive, "material", &what)?)?;
            into.push(Part {
                material,
                triangles,
            });
        }
        Ok(())
    }
}

fn corners(indices: &[u32], mode: u64, what: &str) -> Result<Vec<[u32; 3]>, Error> {
    match mode {
        TRIANGLES => {
            if !indices.len().is_multiple_of(3) {
                return Err(malformed(format!(
                    "{what} has {} indices, not a whole number of triangles",
                    indices.len()
                )));
            }
            Ok(indices.as_chunks::<3>().0.to_vec())
        }
        STRIP => Ok(indices
            .windows(3)
            .map(|three| [0, 1, 2].map(|at| three.get(at).copied().unwrap_or(0)))
            .collect()),
        FAN => {
            let Some((&first, rest)) = indices.split_first() else {
                return Ok(Vec::new());
            };
            Ok(rest
                .windows(2)
                .map(|two| {
                    [
                        first,
                        two.first().copied().unwrap_or(0),
                        two.get(1).copied().unwrap_or(0),
                    ]
                })
                .collect())
        }
        _ => Err(malformed(format!("{what} has mode {mode}"))),
    }
}

fn local(node: &Value, what: &str) -> Result<Matrix, Error> {
    if let Some(flat) = floats::<16>(node, "matrix", what)? {
        let mut matrix = [[0.0; 4]; 4];
        for (at, value) in flat.iter().enumerate() {
            if let Some(cell) = matrix
                .get_mut(at / 4)
                .and_then(|column| column.get_mut(at % 4))
            {
                *cell = *value;
            }
        }
        return Ok(matrix);
    }
    let [tx, ty, tz] = floats::<3>(node, "translation", what)?.unwrap_or([0.0; 3]);
    let [x, y, z, w] = floats::<4>(node, "rotation", what)?.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let [sx, sy, sz] = floats::<3>(node, "scale", what)?.unwrap_or([1.0; 3]);
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if length.is_nan() || length == 0.0 {
        return Err(malformed(format!("{what} rotation is not a rotation")));
    }
    let (x, y, z, w) = (x / length, y / length, z / length, w / length);
    let rotation = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let [r0, r1, r2] = rotation;
    Ok([
        [r0[0] * sx, r0[1] * sx, r0[2] * sx, 0.0],
        [r1[0] * sy, r1[1] * sy, r1[2] * sy, 0.0],
        [r2[0] * sz, r2[1] * sz, r2[2] * sz, 0.0],
        [tx, ty, tz, 1.0],
    ])
}

fn children(node: &Value, count: usize, what: &str) -> Result<Vec<u64>, Error> {
    list(node, "children")?
        .iter()
        .map(|child| {
            let index = child.whole().ok_or_else(|| {
                malformed(format!("{what} has a child that is not a whole number"))
            })?;
            if index >= count as u64 {
                return Err(Error::OutOfRange {
                    what: format!("child of {what}"),
                    index,
                    count: count as u64,
                });
            }
            Ok(index)
        })
        .collect()
}

fn roots(json: &Value, nodes: &[Value]) -> Result<Vec<u64>, Error> {
    let scenes = list(json, "scenes")?;
    if json.get("scenes").is_some() {
        let index = whole(json, "scene", "the file")?.unwrap_or(0);
        let scene = pick(scenes, index, "scene")?;
        return list(scene, "nodes")?
            .iter()
            .map(|node| {
                node.whole().ok_or_else(|| {
                    malformed(format!(
                        "scene {index} has a node that is not a whole number"
                    ))
                })
            })
            .collect();
    }
    if json.get("nodes").is_none() {
        return Err(Error::Missing {
            what: "a scene or nodes".to_owned(),
        });
    }
    let mut child = vec![false; nodes.len()];
    for (at, node) in nodes.iter().enumerate() {
        for index in children(node, nodes.len(), &format!("node {at}"))? {
            if let Some(flag) = child.get_mut(index as usize) {
                *flag = true;
            }
        }
    }
    Ok((0..nodes.len() as u64)
        .filter(|&at| !child.get(at as usize).copied().unwrap_or(true))
        .collect())
}

/// Reads a binary glTF 2.0 file: every primitive with triangles of every
/// mesh the default scene places (scene 0 when the file names none, or
/// every root node when it has no scenes), moved by its nodes' transforms.
/// Points and lines are passed over. Every index, offset and length is
/// checked before it is used, so a malformed file is refused with an
/// [`Error`] naming what is wrong.
pub fn read(bytes: &[u8]) -> Result<Model, Error> {
    let chunks = chunks(bytes)?;
    let file = File {
        json: json::read(chunks.json).map_err(Error::Json)?,
        bin: chunks.bin,
    };
    if file.json.entries().is_none() {
        return Err(malformed("the JSON chunk is not a map".to_owned()));
    }
    let nodes = list(&file.json, "nodes")?;
    let mut seen = vec![false; nodes.len()];
    let mut pending: Vec<(u64, Matrix)> = roots(&file.json, nodes)?
        .into_iter()
        .rev()
        .map(|root| (root, IDENTITY))
        .collect();
    let mut model = Model::default();
    while let Some((index, parent)) = pending.pop() {
        let node = pick(nodes, index, "node")?;
        let what = format!("node {index}");
        let flag = seen
            .get_mut(index as usize)
            .ok_or_else(|| malformed(format!("{what} is past the nodes")))?;
        if *flag {
            return Err(malformed(format!("{what} is reached twice")));
        }
        *flag = true;
        let matrix = times(&parent, &local(node, &what)?);
        if let Some(mesh) = whole(node, "mesh", &what)? {
            file.parts(mesh, &matrix, &mut model.parts)?;
        }
        for child in children(node, nodes.len(), &what)?.into_iter().rev() {
            pending.push((child, matrix));
        }
    }
    Ok(model)
}
