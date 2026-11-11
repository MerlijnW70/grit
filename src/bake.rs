//! The baker: a model built in a modelling tool, as binary glTF 2.0, turned
//! into a box of cell words of a 3D rule that a scene places into a world.
//!
//! [`read_glb`] reads the model, [`Map::read`] the material map, [`bake`]
//! voxelises the model into an [`Asset`], [`Asset::write`] and
//! [`Asset::read`] keep it in a file, and [`place`] writes it into a
//! world of the listed engine through [`crate::live::put3`], which wakes
//! the octs it touches.
//!
//! # Axes
//!
//! glTF's y axis points up and the grid's points down, as grains fall, so
//! a model point `(x, y, z)` lands at grid `(x, -y, z)`, times the scale.
//! A camera of [`crate::render3`] then shows the model as the modelling
//! tool did, not mirrored.
//!
//! # The inside test
//!
//! Solid interiors are filled: ray parity along each of the three axes,
//! decided by a majority vote of the three, with points on an edge or a
//! vertex settled by a symbolic perturbation. Each primitive is tested on
//! its own, and a voxel any primitive fills takes that primitive's word;
//! where primitives overlap, the later one in the scene's order wins.
//!
//! # The lattice
//!
//! The box starts at the least corner of the model's bounds. Voxel
//! `(i, j, k)` is filled when its centre, the corner plus
//! `(i + 0.5, j + 0.5, k + 0.5)` voxel sides, is inside. A side holds
//! every centre within the model's bounds: the extent along it in voxel
//! sides less one half, rounded down, plus one. A model exactly `n` voxels
//! wide is `n` voxels wide however its extent rounds.
//!
//! # The asset file
//!
//! All words are little-endian `u32`:
//!
//! | words | field |
//! |---|---|
//! | 1 | the magic [`ASSET_MAGIC`], the bytes `GRVX` |
//! | 1 | the version, [`ASSET_VERSION`] |
//! | 3 | the box: wide (x), deep (y, down) and tall (z), each at least 1 |
//! | 1 | how many runs follow |
//! | ... | the runs |
//!
//! The runs cover the box's cells in the order of [`crate::sand3::Grid3`],
//! x fastest, then y, then z, and add up to exactly its cells. A run is a
//! head word, whose low 31 bits are its length, at least 1, and whose top
//! bit is [`FILLED`]; a filled run is followed by the one word all its
//! cells hold, an empty run by nothing. Empty cells are not written when
//! the asset is placed, so the world keeps what it had there.
//!
//! # Placing
//!
//! The anchor is the centre of the box's base, the cell
//! `(wide / 2, deep - 1, tall / 2)`, rounded down. [`place`] puts the
//! anchor on a world cell, turned about the vertical axis by quarter
//! turns. One turn takes the offset `(dx, dz)` from the anchor to
//! `(dz, -dx)`: +x goes to -z, a counter-clockwise turn seen from above,
//! as a positive turn about glTF's y axis.

mod glb;
pub mod json;
mod voxel;

pub use glb::{MAGIC, Model, Part, VERSION};
pub use voxel::{Lattice, MAJORITY, votes};

use crate::compact::Plan;
use crate::rule3::nature;
use crate::sand::Grid;

/// The first word of an asset file, the bytes `GRVX`.
pub const ASSET_MAGIC: [u8; 4] = *b"GRVX";

/// The asset file version this baker writes and reads.
pub const ASSET_VERSION: u32 = 1;

/// The top bit of a run's head: the run is filled and a word follows.
pub const FILLED: u32 = 1 << 31;

/// The most cells a baked box may hold.
pub const MOST_CELLS: u64 = 1 << 24;

/// Why a model, a map or an asset was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The file does not start with [`MAGIC`].
    BadMagic { found: [u8; 4] },
    /// A container version other than [`VERSION`].
    Version { found: u32 },
    /// Something ends before the bytes it needs: the header, a chunk, a
    /// buffer, a buffer view, an accessor or the asset's runs.
    Truncated {
        what: String,
        needed: u64,
        held: u64,
    },
    /// The first chunk is not JSON, or there is none.
    NoJson,
    /// The JSON does not read.
    Json(json::Torn),
    /// A field that must be there is not.
    Missing { what: String },
    /// A field of the wrong shape.
    Malformed { what: String },
    /// An index past the items it indexes: a node, a mesh, a material, an
    /// accessor, a buffer view, a buffer or a vertex.
    OutOfRange {
        what: String,
        index: u64,
        count: u64,
    },
    /// Something valid glTF this baker does not read.
    Unsupported { what: String },
    /// A vertex that is not finite once placed.
    NotFinite,
    /// A model with no triangles.
    Empty,
    /// A voxel side or a scale that is not a positive finite number.
    Size { what: &'static str, value: f64 },
    /// A box over [`MOST_CELLS`] cells.
    TooBig { size: [u64; 3], most: u64 },
    /// The map names a rule the world tools do not know.
    UnknownRule { asked: String, known: Vec<String> },
    /// The map names a word its rule does not have.
    UnknownWord { word: String, rule: String },
    /// The map has no default word.
    NoDefault,
    /// The asset file does not start with [`ASSET_MAGIC`].
    BadAssetMagic { found: [u8; 4] },
    /// An asset version other than [`ASSET_VERSION`].
    AssetVersion { found: u32 },
    /// The asset's runs do not cover its box exactly, or a run is empty.
    Runs { covered: u64, cells: u64 },
    /// Bytes after the last run.
    Trailing { at: u64 },
    /// A placed cell falls outside the world.
    Outside { at: [i64; 3], size: [u32; 3] },
    /// The engine refused the write.
    Engine(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic { found } => {
                write!(out, "not binary glTF: the file starts with {found:?}")
            }
            Self::Version { found } => {
                write!(out, "binary glTF version {found}; only {VERSION} is read")
            }
            Self::Truncated { what, needed, held } => write!(
                out,
                "{what} is truncated: it needs {needed} bytes and has {held}"
            ),
            Self::NoJson => write!(out, "the file has no JSON chunk first"),
            Self::Json(torn) => write!(out, "{torn}"),
            Self::Missing { what } => write!(out, "{what} is missing"),
            Self::Malformed { what } => write!(out, "malformed: {what}"),
            Self::OutOfRange { what, index, count } => {
                write!(out, "{what} {index} is out of range: there are {count}")
            }
            Self::Unsupported { what } => write!(out, "not supported: {what}"),
            Self::NotFinite => write!(out, "a vertex is not finite"),
            Self::Empty => write!(out, "the model has no triangles"),
            Self::Size { what, value } => {
                write!(out, "the {what} {value} is not a positive finite number")
            }
            Self::TooBig { size, most } => write!(
                out,
                "a box of {} by {} by {} cells is more than {most}",
                size[0], size[1], size[2]
            ),
            Self::UnknownRule { asked, known } => {
                write!(out, "no rule {asked:?}; the rules are {known:?}")
            }
            Self::UnknownWord { word, rule } => {
                write!(out, "the rule {rule} has no word {word:?}")
            }
            Self::NoDefault => write!(out, "the map has no default"),
            Self::BadAssetMagic { found } => {
                write!(out, "not an asset: the file starts with {found:?}")
            }
            Self::AssetVersion { found } => {
                write!(out, "asset version {found}; only {ASSET_VERSION} is read")
            }
            Self::Runs { covered, cells } => write!(
                out,
                "the runs cover {covered} cells of a box of {cells}, or one is empty"
            ),
            Self::Trailing { at } => write!(out, "the asset goes on past its runs at byte {at}"),
            Self::Outside { at, size } => write!(
                out,
                "a placed cell at {at:?} is outside the world of {size:?}"
            ),
            Self::Engine(why) => write!(out, "the engine refused the write: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads a binary glTF 2.0 file: every primitive with triangles of every
/// mesh the default scene places (scene 0 when the file names none, or
/// every root node when it has no scenes), moved by its nodes' transforms.
/// Points and lines are passed over. Every index, offset and length is
/// checked before it is used, so a malformed file is refused with an
/// [`Error`] naming what is wrong, never a panic.
pub fn read_glb(bytes: &[u8]) -> Result<Model, Error> {
    glb::read(bytes)
}

/// How a model is voxelised: the side of a voxel, in the model's units
/// after scaling, and the uniform scale applied to the model first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub size: f64,
    pub scale: f64,
}

/// The names of the nature rule's kinds, in the order of their values.
pub const NATURE_KINDS: [&str; 13] = [
    "AIR", "ROCK", "EARTH", "SAND", "SNOW", "ICE", "WATER", "WOOD", "LEAVES", "FIRE", "ASH",
    "SMOKE", "STEAM",
];

/// A material map: the word each glTF material name bakes to, and the
/// word for a primitive whose material has no name, is missing, or is not
/// in the map.
///
/// It is read from JSON:
///
/// ```json
/// {"rule": "nature", "heat": 300, "default": "ROCK",
///  "materials": {"bark": "WOOD", "needles": "LEAVES", "stone": "ROCK"}}
/// ```
///
/// `rule` names a rule of [`crate::serve3::rules`] and defaults to
/// `nature`. A word is a whole number, the cell word itself, or for the
/// nature rule the name of a kind of [`NATURE_KINDS`], in any case, which
/// bakes to that kind at the temperature `heat`, 0 when it is not given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map {
    pub rule: String,
    pub words: Vec<(String, u32)>,
    pub default: u32,
}

fn worded(value: &json::Value, rule: &str, heat: u32) -> Result<u32, Error> {
    if let Some(whole) = value.whole() {
        return u32::try_from(whole).map_err(|_| Error::UnknownWord {
            word: whole.to_string(),
            rule: rule.to_owned(),
        });
    }
    let named = value.text().ok_or_else(|| Error::Malformed {
        what: "a word that is neither a whole number nor a name".to_owned(),
    })?;
    let kind = (rule == "nature")
        .then(|| {
            NATURE_KINDS
                .iter()
                .position(|kind| kind.eq_ignore_ascii_case(named))
        })
        .flatten()
        .ok_or_else(|| Error::UnknownWord {
            word: named.to_owned(),
            rule: rule.to_owned(),
        })?;
    Ok(nature::cell(u32::try_from(kind).unwrap_or(0), heat))
}

impl Map {
    /// Reads a map from its JSON. See [`Map`] for the format.
    pub fn read(text: &[u8]) -> Result<Self, Error> {
        let held = json::read(text).map_err(Error::Json)?;
        if held.entries().is_none() {
            return Err(Error::Malformed {
                what: "the map is not a JSON map".to_owned(),
            });
        }
        let known: Vec<String> = crate::serve3::rules()
            .iter()
            .map(|rule| rule.name().to_owned())
            .collect();
        let rule = match held.get("rule") {
            None => "nature".to_owned(),
            Some(one) => one
                .text()
                .ok_or_else(|| Error::Malformed {
                    what: "the map's rule is not a name".to_owned(),
                })?
                .to_owned(),
        };
        if !known.contains(&rule) {
            return Err(Error::UnknownRule { asked: rule, known });
        }
        let heat = match held.get("heat") {
            None => 0,
            Some(one) => one
                .whole()
                .and_then(|heat| u32::try_from(heat).ok())
                .filter(|&heat| heat <= nature::TOP)
                .ok_or_else(|| Error::Malformed {
                    what: format!("the map's heat is not a whole number up to {}", nature::TOP),
                })?,
        };
        let default = worded(held.get("default").ok_or(Error::NoDefault)?, &rule, heat)?;
        let mut words: Vec<(String, u32)> = Vec::new();
        if let Some(materials) = held.get("materials") {
            let entries = materials.entries().ok_or_else(|| Error::Malformed {
                what: "the map's materials are not a JSON map".to_owned(),
            })?;
            for (name, value) in entries {
                if words.iter().any(|(known, _)| known == name) {
                    return Err(Error::Malformed {
                        what: format!("the material {name:?} is mapped twice"),
                    });
                }
                words.push((name.clone(), worded(value, &rule, heat)?));
            }
        }
        Ok(Self {
            rule,
            words,
            default,
        })
    }

    /// The word the material `name` bakes to.
    #[must_use]
    pub fn word(&self, name: Option<&str>) -> u32 {
        name.and_then(|name| {
            self.words
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, word)| *word)
        })
        .unwrap_or(self.default)
    }
}

/// A baked box of cells, each a word or empty, in the order of
/// [`crate::sand3::Grid3`]: x fastest, then y (down), then z.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub size: [u32; 3],
    pub cells: Vec<Option<u32>>,
}

fn cells_of(size: [u64; 3]) -> Result<u64, Error> {
    let cells = size
        .iter()
        .try_fold(1_u64, |cells, &side| cells.checked_mul(side))
        .unwrap_or(u64::MAX);
    if cells > MOST_CELLS || size.contains(&0) {
        return Err(Error::TooBig {
            size,
            most: MOST_CELLS,
        });
    }
    Ok(cells)
}

fn word_at(bytes: &[u8], at: usize) -> Result<u32, Error> {
    bytes
        .get(at..at + 4)
        .and_then(|four| <[u8; 4]>::try_from(four).ok())
        .map(u32::from_le_bytes)
        .ok_or(Error::Truncated {
            what: "the asset".to_owned(),
            needed: (at + 4) as u64,
            held: bytes.len() as u64,
        })
}

impl Asset {
    /// An asset of the box `size` from its cells, refused unless they fill
    /// the box exactly.
    pub fn new(size: [u32; 3], cells: Vec<Option<u32>>) -> Result<Self, Error> {
        let wanted = cells_of(size.map(u64::from))?;
        if cells.len() as u64 != wanted {
            return Err(Error::Runs {
                covered: cells.len() as u64,
                cells: wanted,
            });
        }
        Ok(Self { size, cells })
    }

    /// The cell of the box at `(x, y, z)`, or `None` past the box.
    #[must_use]
    pub fn at(&self, [x, y, z]: [u32; 3]) -> Option<Option<u32>> {
        let [wide, deep, tall] = self.size;
        if x >= wide || y >= deep || z >= tall {
            return None;
        }
        let index = (z as usize * deep as usize + y as usize) * wide as usize + x as usize;
        self.cells.get(index).copied()
    }

    /// The anchor, the centre of the base: `(wide / 2, deep - 1, tall / 2)`.
    #[must_use]
    pub const fn anchor(&self) -> [u32; 3] {
        let [wide, deep, tall] = self.size;
        [wide / 2, deep.saturating_sub(1), tall / 2]
    }

    /// How many cells hold a word.
    #[must_use]
    pub fn filled(&self) -> usize {
        self.cells.iter().filter(|cell| cell.is_some()).count()
    }

    /// The asset file, as the [module](self) documents it.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut runs: Vec<(u32, Option<u32>)> = Vec::new();
        for &cell in &self.cells {
            match runs.last_mut() {
                Some((length, word)) if *word == cell && *length < FILLED - 1 => *length += 1,
                _ => runs.push((1, cell)),
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&ASSET_MAGIC);
        let mut words = vec![ASSET_VERSION];
        words.extend(self.size);
        words.push(u32::try_from(runs.len()).unwrap_or(u32::MAX));
        for (length, word) in runs {
            match word {
                Some(word) => words.extend([length | FILLED, word]),
                None => words.push(length),
            }
        }
        for word in words {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// Reads an asset file, refusing a bad magic or version, a box of no
    /// cells or past [`MOST_CELLS`], runs that do not cover the box
    /// exactly, an empty run, a file cut short and bytes past the runs.
    pub fn read(bytes: &[u8]) -> Result<Self, Error> {
        let magic = bytes.get(..4).ok_or(Error::Truncated {
            what: "the asset".to_owned(),
            needed: 4,
            held: bytes.len() as u64,
        })?;
        if magic != ASSET_MAGIC {
            let mut found = [0; 4];
            found.copy_from_slice(magic);
            return Err(Error::BadAssetMagic { found });
        }
        let version = word_at(bytes, 4)?;
        if version != ASSET_VERSION {
            return Err(Error::AssetVersion { found: version });
        }
        let size = [word_at(bytes, 8)?, word_at(bytes, 12)?, word_at(bytes, 16)?];
        let wanted = cells_of(size.map(u64::from))?;
        let runs = word_at(bytes, 20)?;
        let mut at = 24;
        let mut cells: Vec<Option<u32>> = Vec::new();
        for _ in 0..runs {
            let head = word_at(bytes, at)?;
            at += 4;
            let length = u64::from(head & !FILLED);
            let covered = cells.len() as u64 + length;
            if length == 0 || covered > wanted {
                return Err(Error::Runs {
                    covered,
                    cells: wanted,
                });
            }
            let word = if head & FILLED == 0 {
                None
            } else {
                at += 4;
                Some(word_at(bytes, at - 4)?)
            };
            cells.resize(covered as usize, word);
        }
        if cells.len() as u64 != wanted {
            return Err(Error::Runs {
                covered: cells.len() as u64,
                cells: wanted,
            });
        }
        if at < bytes.len() {
            return Err(Error::Trailing { at: at as u64 });
        }
        Ok(Self { size, cells })
    }

    /// The world cells of this asset placed with its anchor on `at`,
    /// turned `turns` quarter turns, in a solid world of `size` cells (x,
    /// y, z), each as a cell index of [`crate::sand3::Grid3`] and its word.
    /// Empty cells are left out. A cell outside the world is refused with
    /// [`Error::Outside`]; the world does not wrap.
    pub fn placed(
        &self,
        size: [u32; 3],
        at: [u32; 3],
        turns: u32,
    ) -> Result<Vec<(u32, u32)>, Error> {
        let anchor = self.anchor().map(i64::from);
        let at = at.map(i64::from);
        let [wide, deep, tall] = self.size;
        let mut out = Vec::with_capacity(self.filled());
        let mut cells = self.cells.iter();
        for z in 0..tall {
            for y in 0..deep {
                for x in 0..wide {
                    let Some(Some(word)) = cells.next() else {
                        continue;
                    };
                    let mut dx = i64::from(x) - anchor[0];
                    let mut dz = i64::from(z) - anchor[2];
                    for _ in 0..turns % 4 {
                        (dx, dz) = (dz, -dx);
                    }
                    let world = [at[0] + dx, at[1] + i64::from(y) - anchor[1], at[2] + dz];
                    let inside = world
                        .iter()
                        .zip(size)
                        .all(|(&one, side)| (0..i64::from(side)).contains(&one));
                    if !inside {
                        return Err(Error::Outside { at: world, size });
                    }
                    let [wx, wy, wz] = world.map(|one| u64::try_from(one).unwrap_or(0));
                    let index = (wz * u64::from(size[1]) + wy) * u64::from(size[0]) + wx;
                    let index =
                        u32::try_from(index).map_err(|_| Error::Outside { at: world, size })?;
                    out.push((index, *word));
                }
            }
        }
        Ok(out)
    }
}

fn checked(what: &'static str, value: f64) -> Result<f64, Error> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(Error::Size { what, value })
    }
}

/// Voxelises `model` into an asset: each primitive's voxels take the word
/// `map` gives its material. See the [module](self) for the axes, the
/// lattice and the inside test.
pub fn bake(model: &Model, map: &Map, options: Options) -> Result<Asset, Error> {
    let size = checked("voxel side", options.size)?;
    let scale = checked("scale", options.scale)?;
    let parts: Vec<Vec<[[f64; 3]; 3]>> = model
        .parts
        .iter()
        .map(|part| {
            part.triangles
                .iter()
                .map(|triangle| triangle.map(|[x, y, z]| [x * scale, -y * scale, z * scale]))
                .collect()
        })
        .collect();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for point in parts.iter().flatten().flatten() {
        if point.iter().any(|value| !value.is_finite()) {
            return Err(Error::NotFinite);
        }
        for ((low, high), value) in low.iter_mut().zip(high.iter_mut()).zip(point) {
            *low = low.min(*value);
            *high = high.max(*value);
        }
    }
    if parts.iter().all(Vec::is_empty) {
        return Err(Error::Empty);
    }
    let mut count = [0_u64; 3];
    for ((side, low), high) in count.iter_mut().zip(low).zip(high) {
        let voxels = ((high - low) / size - 0.5).floor().max(0.0) + 1.0;
        *side = if voxels.is_finite() && voxels < u64::MAX as f64 {
            voxels as u64
        } else {
            u64::MAX
        };
    }
    let cells = cells_of(count)?;
    let lattice = Lattice {
        origin: low,
        size,
        count: count.map(|side| side as usize),
    };
    let mut asset = Asset::new(
        count.map(|side| u32::try_from(side).unwrap_or(u32::MAX)),
        vec![None; cells as usize],
    )?;
    for (part, triangles) in model.parts.iter().zip(&parts) {
        let word = map.word(part.material.as_deref());
        for (cell, votes) in asset.cells.iter_mut().zip(votes(&lattice, triangles)) {
            if votes >= MAJORITY {
                *cell = Some(word);
            }
        }
    }
    Ok(asset)
}

/// Writes `asset` into the world of a listed engine on the solid grid
/// `grid`, its anchor on `at`, turned `turns` quarter turns, through
/// [`crate::live::put3`], which wakes every oct it writes into. Returns
/// the cells written. Nothing is written when a cell falls outside the
/// world. No frame may be in flight.
pub fn place(
    held: &mut crate::Resident<'_>,
    grid: Grid,
    plan: &Plan,
    asset: &Asset,
    at: [u32; 3],
    turns: u32,
) -> Result<Vec<(u32, u32)>, Error> {
    let cells = asset.placed([grid.wide, grid.deep, grid.tall], at, turns)?;
    crate::live::put3(held, grid, plan, &cells).map_err(|why| Error::Engine(why.to_string()))?;
    Ok(cells)
}
