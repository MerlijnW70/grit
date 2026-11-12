pub mod cpu;
pub mod kernel;
pub mod law;
pub mod reduce;

use crate::bricks::{Bricks, MOST_GROUPS};
use crate::layout::Windows;
use crate::ops::{Flag, Helpers, Ops};
use crate::resident::{Groups, Resident};
use crate::sand::Grid;

pub use reduce::{Answer, Measure, Reduce};

pub const MOST_BINS: u32 = 256;

pub const PLANES: usize = 1;

#[derive(Debug, Clone, Copy)]
pub struct Seen<'held, W> {
    pub planes: &'held [W],
    pub at: [W; 3],
}

impl<W: Copy> Seen<'_, W> {
    #[must_use]
    pub fn cell(&self) -> Option<W> {
        self.planes.first().copied()
    }
}

pub trait Map {
    fn map<O: Ops>(&self, ops: &mut O, seen: Seen<'_, O::Word>) -> O::Word;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub from: [u32; 3],
    pub to: [u32; 3],
}

impl Region {
    #[must_use]
    pub const fn new(from: [u32; 3], to: [u32; 3]) -> Self {
        Self { from, to }
    }

    #[must_use]
    pub const fn whole(size: [u32; 3]) -> Self {
        Self {
            from: [0; 3],
            to: size,
        }
    }

    #[must_use]
    pub fn sides(&self) -> [u32; 3] {
        [0, 1, 2].map(|axis| {
            let from = self.from.get(axis).copied().unwrap_or(0);
            let to = self.to.get(axis).copied().unwrap_or(0);
            to.saturating_sub(from)
        })
    }

    #[must_use]
    pub fn cells(&self) -> u128 {
        self.sides().iter().map(|&side| u128::from(side)).product()
    }

    #[must_use]
    pub fn holds(&self, point: [u32; 3]) -> bool {
        point
            .iter()
            .zip(self.from.iter().zip(self.to))
            .all(|(&at, (&from, to))| at >= from && at < to)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Dense { grid: Grid, windows: Windows },
    Sparse(Bricks),
}

impl Layout {
    #[must_use]
    pub const fn grid(&self) -> Grid {
        match self {
            Self::Dense { grid, .. } => *grid,
            Self::Sparse(bricks) => bricks.grid(),
        }
    }

    #[must_use]
    pub const fn size(&self) -> [u32; 3] {
        let grid = self.grid();
        [grid.wide, grid.deep, grid.tall]
    }

    #[must_use]
    pub const fn windows(&self) -> Windows {
        match self {
            Self::Dense { windows, .. } => *windows,
            Self::Sparse(bricks) => bricks.windows(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Outside {
        from: [u32; 3],
        to: [u32; 3],
        size: [u32; 3],
    },
    Empty {
        from: [u32; 3],
        to: [u32; 3],
    },
    TooLarge {
        cells: u128,
        most: u128,
        what: &'static str,
    },
    Bins {
        asked: u32,
    },
    NoTarget,
    TargetShort {
        cells: u128,
        held: usize,
    },
    Unasked {
        asked: usize,
        held: usize,
    },
    AimSize {
        cells: u128,
        given: usize,
    },
    Unaimed {
        region: Region,
        aimed: Option<Region>,
    },
    Width {
        subgroup: u32,
    },
    Flat,
}

impl Refused {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Outside { .. } => "outside",
            Self::Empty { .. } => "empty",
            Self::TooLarge { .. } => "too_large",
            Self::Bins { .. } => "bins",
            Self::NoTarget => "no_target",
            Self::TargetShort { .. } => "target_short",
            Self::Unasked { .. } => "unasked",
            Self::AimSize { .. } => "aim_size",
            Self::Unaimed { .. } => "unaimed",
            Self::Width { .. } => "width",
            Self::Flat => "flat",
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outside { from, to, size } => write!(
                out,
                "the box from {from:?} to {to:?} leaves the world of {size:?} cells"
            ),
            Self::Empty { from, to } => {
                write!(out, "the box from {from:?} to {to:?} holds no cell")
            }
            Self::TooLarge { cells, most, what } => write!(
                out,
                "the box holds {cells} cells and {what} fits at most {most}"
            ),
            Self::Bins { asked } => write!(
                out,
                "a histogram of {asked} bins is not one of 1 to {MOST_BINS}"
            ),
            Self::NoTarget => write!(out, "a difference needs a target plane, and none was made"),
            Self::TargetShort { cells, held } => write!(
                out,
                "the box holds {cells} cells and the target plane holds {held}"
            ),
            Self::Unasked { asked, held } => write!(
                out,
                "question {asked} was never built; {held} questions are"
            ),
            Self::AimSize { cells, given } => write!(
                out,
                "the box holds {cells} cells and {given} target words were given"
            ),
            Self::Unaimed { region, aimed } => write!(
                out,
                "a difference over {region:?} needs a target uploaded for that box; the target holds {aimed:?}"
            ),
            Self::Width { subgroup } => write!(
                out,
                "no question kernel is built at subgroup width {subgroup}"
            ),
            Self::Flat => write!(out, "questions are asked of solid worlds"),
        }
    }
}

impl std::error::Error for Refused {}

#[derive(Debug)]
pub enum Failure {
    Refused(Refused),
    Host(crate::Error),
}

impl Failure {
    #[must_use]
    pub const fn refused(&self) -> Option<&Refused> {
        match self {
            Self::Refused(why) => Some(why),
            Self::Host(_) => None,
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => write!(out, "{why}"),
            Self::Host(why) => write!(out, "{why}"),
        }
    }
}

impl std::error::Error for Failure {}

impl From<Refused> for Failure {
    fn from(why: Refused) -> Self {
        Self::Refused(why)
    }
}

impl From<crate::Error> for Failure {
    fn from(why: crate::Error) -> Self {
        Self::Host(why)
    }
}

impl From<simdr::lanes::LaneError> for Failure {
    fn from(why: simdr::lanes::LaneError) -> Self {
        Self::Host(why.into())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Taint {
    carried: Option<bool>,
}

impl Taint {
    fn flagged(&mut self, holds: bool) -> Flag<bool> {
        self.carried = Some(holds);
        let flag = Helpers::truth(self, true);
        self.carried = None;
        flag
    }
}

impl Ops for Taint {
    type Word = bool;

    fn constant(&mut self, _: u32) -> bool {
        self.carried.take().unwrap_or(false)
    }

    fn and(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn or(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn xor(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn not(&mut self, value: bool) -> bool {
        value
    }

    fn shift_left(&mut self, value: bool, _: u32) -> bool {
        value
    }

    fn shift_right(&mut self, value: bool, _: u32) -> bool {
        value
    }

    fn add(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn sub(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn mul(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn div(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn dot_signed(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn dot_unsigned(&mut self, left: bool, right: bool) -> bool {
        left || right
    }

    fn equal(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn not_equal(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn less(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn less_equal(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn greater(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn greater_equal(&mut self, left: bool, right: bool) -> Flag<bool> {
        self.flagged(left || right)
    }

    fn select(&mut self, flag: Flag<bool>, then: bool, otherwise: bool) -> bool {
        flag.word() || then || otherwise
    }
}

#[must_use]
pub fn reads_point<M: Map>(map: &M) -> bool {
    let planes = [false; PLANES];
    map.map(
        &mut Taint::default(),
        Seen {
            planes: &planes,
            at: [true; 3],
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked(usize);

impl Asked {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone)]
struct Built {
    reduce: Reduce,
    words: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Questions {
    layout: Layout,
    subgroup: u32,
    first: u32,
    target: usize,
    counting: bool,
    built: Vec<Built>,
    aimed: Option<Region>,
}

pub const ASK: usize = 0;
pub const TARGET: usize = 1;
pub const BINDINGS: usize = 2;

impl Questions {
    pub fn new(
        layout: Layout,
        subgroup: u32,
        first: usize,
        target: usize,
    ) -> Result<Self, Refused> {
        if !matches!(subgroup, 4 | 8 | 16 | 32 | 64) {
            return Err(Refused::Width { subgroup });
        }
        if layout.grid().tall < 2 {
            return Err(Refused::Flat);
        }
        let first = u32::try_from(first).map_err(|_| Refused::Width { subgroup })?;
        Ok(Self {
            layout,
            subgroup,
            first,
            target,
            counting: false,
            built: Vec::new(),
            aimed: None,
        })
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub const fn counting(mut self) -> Self {
        self.counting = true;
        self
    }

    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    #[must_use]
    pub const fn first(&self) -> usize {
        self.first as usize
    }

    #[must_use]
    pub const fn target_words(&self) -> usize {
        self.target
    }

    pub fn add<M: Map>(&mut self, map: &M, reduce: Reduce) -> Result<Asked, Failure> {
        reduce.check()?;
        if matches!(reduce, Reduce::Differ(_)) && self.target == 0 {
            return Err(Refused::NoTarget.into());
        }
        let shape = kernel::Shape {
            layout: self.layout,
            subgroup: self.subgroup,
            ask: self.first + ASK as u32,
            target: self.first + TARGET as u32,
            reduce,
            counting: self.counting,
            placed: reads_point(map),
        };
        let words = kernel::built(map, &shape)?;
        self.built.push(Built { reduce, words });
        Ok(Asked(self.built.len() - 1))
    }

    #[must_use]
    pub fn ask_words(&self) -> usize {
        let most = self
            .built
            .iter()
            .map(|one| one.reduce.words())
            .max()
            .unwrap_or(0);
        reduce::RESULT + most + reduce::COUNTER
    }

    #[must_use]
    pub fn sizes(&self) -> Vec<usize> {
        vec![self.ask_words(), self.target.max(1)]
    }

    #[must_use]
    pub fn resting(&self) -> Vec<Groups> {
        vec![Groups::Fixed(0); self.built.len()]
    }

    #[must_use]
    pub fn modules(&self) -> Vec<&[u32]> {
        self.built.iter().map(|one| one.words.as_slice()).collect()
    }

    #[must_use]
    pub fn reduce(&self, asked: Asked) -> Option<Reduce> {
        self.built.get(asked.0).map(|one| one.reduce)
    }

    pub fn aim(
        &mut self,
        held: &mut Resident<'_>,
        region: Region,
        words: &[u32],
    ) -> Result<(), Failure> {
        if self.target == 0 {
            return Err(Refused::NoTarget.into());
        }
        let cells = self.placed(region)?;
        if cells > self.target as u128 {
            return Err(Refused::TargetShort {
                cells,
                held: self.target,
            }
            .into());
        }
        if words.len() as u128 != cells {
            return Err(Refused::AimSize {
                cells,
                given: words.len(),
            }
            .into());
        }
        self.aimed = None;
        held.write((self.first as usize) + TARGET, words)?;
        self.aimed = Some(region);
        Ok(())
    }

    fn placed(&self, region: Region) -> Result<u128, Refused> {
        let size = self.layout.size();
        let Region { from, to } = region;
        if to.iter().zip(size).any(|(&high, side)| high > side) {
            return Err(Refused::Outside { from, to, size });
        }
        if from.iter().zip(to).any(|(&low, high)| high <= low) {
            return Err(Refused::Empty { from, to });
        }
        Ok(region.cells())
    }

    pub fn check(&self, asked: Asked, region: Region) -> Result<Reduce, Refused> {
        let reduce = self.reduce(asked).ok_or(Refused::Unasked {
            asked: asked.0,
            held: self.built.len(),
        })?;
        let cells = self.placed(region)?;
        let (most, what) = reduce.most_cells(region);
        if cells > most {
            return Err(Refused::TooLarge { cells, most, what });
        }
        if let Reduce::Differ(_) = reduce {
            if cells > self.target as u128 {
                return Err(Refused::TargetShort {
                    cells,
                    held: self.target,
                });
            }
            if self.aimed != Some(region) {
                return Err(Refused::Unaimed {
                    region,
                    aimed: self.aimed,
                });
            }
        }
        Ok(reduce)
    }

    pub fn ask(
        &self,
        held: &mut Resident<'_>,
        first_pass: usize,
        asked: Asked,
        region: Region,
    ) -> Result<Answer, Failure> {
        self.spread(held, first_pass, asked, region, MOST_GROUPS)
            .map(|(answer, _)| answer)
    }

    #[cfg(feature = "faults")]
    pub fn ask_spread(
        &self,
        held: &mut Resident<'_>,
        first_pass: usize,
        asked: Asked,
        region: Region,
        most: u32,
    ) -> Result<(Answer, Option<reduce::Reads>), Failure> {
        self.spread(held, first_pass, asked, region, most.clamp(1, MOST_GROUPS))
    }

    fn spread(
        &self,
        held: &mut Resident<'_>,
        first_pass: usize,
        asked: Asked,
        region: Region,
        most: u32,
    ) -> Result<(Answer, Option<reduce::Reads>), Failure> {
        let reduce = self.check(asked, region)?;
        let walk = kernel::Walk::of(&self.layout, self.subgroup, region, most);
        let mut words = vec![0_u32; reduce::RESULT];
        walk.write(&mut words, region, reduce);
        words.extend(reduce.initial());
        words.extend([0; reduce::COUNTER]);
        let ask = self.first as usize + ASK;
        held.write(ask, &words)?;
        let mut groups = vec![Groups::Fixed(0); first_pass + asked.0];
        groups.push(Groups::Fixed(walk.groups));
        groups.resize(first_pass + self.built.len(), Groups::Fixed(0));
        let (_, read) = held.frame_reading(&groups, ask, words.len())?;
        let result = read.get(reduce::RESULT..).unwrap_or_default();
        let answer = reduce.answer(result);
        let reads = self
            .counting
            .then(|| reduce::Reads::of(result.get(reduce.words()..).unwrap_or_default()));
        Ok((answer, reads))
    }
}
