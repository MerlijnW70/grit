pub mod fork;
pub mod format;

use std::fs::File;
use std::io::Write;
use std::path::Path;

use simdr::lanes::LaneError;

use crate::bricks::{self, Bricks, Content, ENTRY, Packed, UNIFORM};
use crate::compact::{Plan, TooBig};
use crate::host::{self, Host};
use crate::layout::{Dimension, PARAMS, TALLY, Windows};
use crate::live;
use crate::resident::{Groups, Resident};
use crate::rule3::FIRST_PARAM;
use crate::sand::Grid;
use crate::serve3::{Listed, POOL_LEAST_CELLS, rules_on};
use crate::sweep;

pub use format::{Image, Law, Sha256, State, sha256};

pub type Failed = Box<dyn std::error::Error>;

const BATCH: u32 = 64;

const ZEROS: usize = 1 << 22;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    Magic,
    Version {
        found: u32,
    },
    Checksum,
    Truncated {
        at: u64,
        reading: &'static str,
    },
    Sizes {
        what: &'static str,
        header: u64,
        found: u64,
    },
    LawHash,
    Unfit {
        why: String,
    },
    Section {
        tag: [u8; 4],
    },
    Trailing {
        at: u64,
    },
    Malformed {
        what: String,
    },
    Rule {
        name: String,
    },
    Params {
        rule: String,
        given: usize,
        wanted: usize,
    },
    Unlike {
        what: &'static str,
    },
    Outside {
        at: [u32; 3],
        size: [u32; 3],
    },
    Io {
        what: String,
    },
}

impl Refused {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Magic => "bad_magic",
            Self::Version { .. } => "unknown_version",
            Self::Checksum => "bad_checksum",
            Self::Truncated { .. } => "truncated",
            Self::Sizes { .. } => "sizes",
            Self::LawHash => "law_hash",
            Self::Unfit { .. } => "unfit",
            Self::Section { .. } => "unknown_section",
            Self::Trailing { .. } => "trailing",
            Self::Malformed { .. } => "malformed",
            Self::Rule { .. } => "unknown_rule",
            Self::Params { .. } => "params",
            Self::Unlike { .. } => "unlike",
            Self::Outside { .. } => "outside",
            Self::Io { .. } => "io",
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Magic => write!(out, "the file does not start with {:?}", "GRITWLD1"),
            Self::Version { found } => write!(
                out,
                "the file is version {found}, and this reader knows version {}",
                format::VERSION
            ),
            Self::Checksum => write!(out, "the file's checksum does not match its bytes"),
            Self::Truncated { at, reading } => {
                write!(out, "the file ends at byte {at} while reading {reading}")
            }
            Self::Sizes {
                what,
                header,
                found,
            } => write!(
                out,
                "the {what} disagrees with the header: {header} against {found}"
            ),
            Self::LawHash => write!(
                out,
                "the header's law hash is not the sha256 of the law section"
            ),
            Self::Unfit { why } => write!(out, "the world does not fit the device: {why}"),
            Self::Section { tag } => write!(
                out,
                "the file has a section {:?} this reader does not know",
                String::from_utf8_lossy(tag)
            ),
            Self::Trailing { at } => {
                write!(out, "the file runs on past its checksum, from byte {at}")
            }
            Self::Malformed { what } => write!(out, "{what}"),
            Self::Rule { name } => write!(out, "no rule is named {name:?}"),
            Self::Params {
                rule,
                given,
                wanted,
            } => write!(
                out,
                "rule {rule} reads {wanted} parameters, and {given} were given"
            ),
            Self::Unlike { what } => {
                write!(
                    out,
                    "the worlds differ in their {what}, so one cannot take the other's state"
                )
            }
            Self::Outside { at, size } => write!(
                out,
                "cell {at:?} is outside the world of {} by {} by {} cells",
                size[0], size[1], size[2]
            ),
            Self::Io { what } => write!(out, "{what}"),
        }
    }
}

impl std::error::Error for Refused {}

fn unfit(why: impl std::fmt::Display) -> Refused {
    Refused::Unfit {
        why: why.to_string(),
    }
}

pub(crate) fn fitted(why: host::Error) -> Failed {
    match why {
        host::Error::TooLarge { .. }
        | host::Error::TooMuchMemory { .. }
        | host::Error::NoMemory { .. }
        | host::Error::PoolFull { .. } => Box::new(unfit(why)),
        other => Box::new(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Made<'made> {
    pub rule: &'made str,
    pub params: &'made [u32],
    pub size: [u32; 3],
    pub sparse: bool,
    pub slots: Option<u32>,
}

pub struct World<'host> {
    host: &'host Host,
    rule: Box<dyn Listed>,
    passes: Vec<Vec<u32>>,
    stages: Vec<Vec<Groups>>,
    sizes: Vec<usize>,
    grid: Grid,
    plan: Plan,
    pool: Option<Bricks>,
    params: Vec<u32>,
    frame: u32,
    live: Resident<'host>,
}

pub(crate) fn ruled(host: &Host, name: &str, params: &[u32]) -> Result<Box<dyn Listed>, Refused> {
    let mut known = rules_on(Some(host));
    let at = known
        .iter()
        .position(|one| one.name() == name)
        .ok_or_else(|| Refused::Rule {
            name: name.to_owned(),
        })?;
    let rule = known.swap_remove(at);
    if params.len() != rule.params() {
        return Err(Refused::Params {
            rule: name.to_owned(),
            given: params.len(),
            wanted: rule.params(),
        });
    }
    Ok(rule)
}

fn planned(width: u32, size: [u32; 3], sparse: bool) -> Result<(Grid, Plan), Failed> {
    let grid = Grid::solid(size[0], size[1], size[2])?;
    if grid.tall < 2 {
        return Err(Box::new(Refused::Malformed {
            what: format!(
                "a world of {} by {} by {} cells is not solid",
                size[0], size[1], size[2]
            ),
        }));
    }
    let blocks = grid.blocks().div_ceil(sweep::cells(width));
    let plan = if sparse {
        Plan::bricked(width, blocks)?
    } else {
        Plan::over_in(width, blocks, Dimension::Three)?
    };
    Ok((grid, plan))
}

fn most_slots(plan: &Plan) -> u32 {
    u32::try_from(((1_u64 << 32) - 1) / (u64::from(plan.per_block) * 8))
        .unwrap_or(u32::MAX)
        .min(plan.blocks)
}

pub(crate) fn zeroed(
    live: &mut Resident<'_>,
    sizes: &[usize],
    state: Option<(Windows, usize)>,
) -> Result<(), host::Error> {
    let rest = state.map_or(0, |(windows, from)| windows.words().saturating_sub(from));
    let widest = sizes
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .max(rest)
        .clamp(1, ZEROS);
    let zeros = vec![0_u32; widest];
    let windows = state.map(|(windows, _)| windows);
    for (binding, &words) in sizes.iter().enumerate() {
        let skipped = windows.is_some_and(|windows| {
            (0..windows.count()).any(|window| Windows::binding(window) as usize == binding)
        });
        if skipped {
            continue;
        }
        let mut at = 0;
        while at < words {
            let run = zeros.len().min(words - at);
            live.write_at(binding, at, zeros.get(..run).unwrap_or_default())?;
            at += run;
        }
    }
    if let Some((windows, from)) = state {
        let mut at = from;
        while at < windows.words() {
            let run = zeros.len().min(windows.words() - at);
            live::write_state(live, windows, at, zeros.get(..run).unwrap_or_default())?;
            at += run;
        }
    }
    Ok(())
}

impl<'host> World<'host> {
    fn built(
        host: &'host Host,
        rule: Box<dyn Listed>,
        params: Vec<u32>,
        (grid, plan): (Grid, Plan),
        pool: Option<Bricks>,
    ) -> Result<Self, Failed> {
        let ((passes, stages), sizes) = match pool.as_ref() {
            Some(bricks) => (
                rule.bricked(bricks, &plan)?,
                rule.bricked_sizes(bricks, &plan),
            ),
            None => (
                rule.wired(host.width(), grid, &plan)?,
                plan.windows().laid(&rule.sizes(&plan)),
            ),
        };
        host.limits()
            .admits_on(&sizes, None, host.heap())
            .map_err(fitted)?;
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let live = host.resident(&sizes, &modules).map_err(fitted)?;
        Ok(Self {
            host,
            rule,
            passes,
            stages,
            sizes,
            grid,
            plan,
            pool,
            params,
            frame: 0,
            live,
        })
    }

    pub fn create(host: &'host Host, made: &Made<'_>) -> Result<Self, Failed> {
        let rule = ruled(host, made.rule, made.params)?;
        let shape = planned(host.width(), made.size, made.sparse)?;
        let plan = shape.1;
        let pool = if made.sparse {
            let brick_cells = u64::from(plan.per_block) * 8;
            let least = u32::try_from(POOL_LEAST_CELLS.div_ceil(brick_cells)).unwrap_or(u32::MAX);
            let capacity = made
                .slots
                .unwrap_or((plan.blocks / 4).max(least))
                .clamp(1, most_slots(&plan));
            Some(Bricks::new(shape.0, &plan, capacity)?)
        } else {
            None
        };
        let mut world = Self::built(host, rule, made.params.to_vec(), shape, pool)?;
        zeroed(&mut world.live, &world.sizes, None)?;
        world.parameters()?;
        match world.pool {
            Some(bricks) => {
                world.live.write(PARAMS as usize, &[u32::MAX])?;
                let packed = bricks.compose(|_| Content::Uniform(0))?;
                bricks.write_packed(&mut world.live, &world.plan, &packed)?;
                bricks.seed(&mut world.live, &world.plan)?;
            }
            None => live::seed(&mut world.live, world.grid, &world.plan)?,
        }
        Ok(world)
    }

    fn parameters(&mut self) -> Result<(), host::Error> {
        if self.params.is_empty() {
            return Ok(());
        }
        self.live
            .write_at(PARAMS as usize, FIRST_PARAM, &self.params)
    }

    pub fn open(host: &'host Host, image: &Image) -> Result<Self, Failed> {
        let rule = ruled(host, image.law.name(), &image.params)?;
        let sparse = matches!(image.state, State::Sparse { .. });
        let shape = planned(host.width(), image.size, sparse).map_err(|why| {
            if why.downcast_ref::<TooBig>().is_some()
                || why.downcast_ref::<crate::sand::Unfit>().is_some()
            {
                Box::new(unfit(why)) as Failed
            } else {
                why
            }
        })?;
        let (grid, plan) = shape;
        if [grid.wide, grid.deep, grid.tall] != image.size {
            return Err(Box::new(Refused::Malformed {
                what: format!("a world of {:?} cells is not solid", image.size),
            }));
        }
        let mut world = match &image.state {
            State::Dense(cells) => {
                let mut world = Self::built(host, rule, image.params.clone(), shape, None)?;
                let windows = world.plan.windows();
                zeroed(&mut world.live, &world.sizes, Some((windows, cells.len())))?;
                live::write_state(&mut world.live, windows, 0, cells)?;
                world.parameters()?;
                world
            }
            State::Sparse {
                capacity,
                sides,
                table,
                pool,
            } => {
                let (bricks, packed) =
                    repacked(image, &plan, grid, *capacity, *sides, table, pool)?;
                let mut world = Self::built(host, rule, image.params.clone(), shape, Some(bricks))?;
                let reach = packed.pool.len();
                zeroed(
                    &mut world.live,
                    &world.sizes,
                    Some((bricks.windows(), reach)),
                )?;
                world.parameters()?;
                bricks
                    .write_packed(&mut world.live, &world.plan, &packed)
                    .map_err(fitted)?;
                world
            }
        };
        world.reseed()?;
        world.frame = image.frame;
        world
            .live
            .write(PARAMS as usize, &[image.frame.wrapping_sub(1)])?;
        Ok(world)
    }

    fn reseed(&mut self) -> Result<(), host::Error> {
        match self.pool {
            Some(bricks) => bricks.seed(&mut self.live, &self.plan),
            None => live::seed(&mut self.live, self.grid, &self.plan),
        }
    }

    #[must_use]
    pub fn rule(&self) -> &str {
        self.rule.name()
    }

    #[must_use]
    pub fn params(&self) -> &[u32] {
        &self.params
    }

    #[must_use]
    pub const fn frame(&self) -> u32 {
        self.frame
    }

    #[must_use]
    pub const fn size(&self) -> [u32; 3] {
        [self.grid.wide, self.grid.deep, self.grid.tall]
    }

    #[must_use]
    pub const fn pool(&self) -> Option<Bricks> {
        self.pool
    }

    pub fn write(&mut self, cells: &[([u32; 3], u32)]) -> Result<(), Failed> {
        let size = self.size();
        if let Some(&(at, _)) = cells
            .iter()
            .find(|(at, _)| at.iter().zip(size).any(|(one, side)| *one >= side))
        {
            return Err(Box::new(Refused::Outside { at, size }));
        }
        match self.pool {
            Some(bricks) => bricks.put_at(&mut self.live, cells)?,
            None => {
                let mut indexed = Vec::with_capacity(cells.len());
                for &(at, word) in cells {
                    let index = (u64::from(at[2]) * u64::from(size[1]) + u64::from(at[1]))
                        * u64::from(size[0])
                        + u64::from(at[0]);
                    indexed.push((u32::try_from(index)?, word));
                }
                live::put3(&mut self.live, self.grid, &self.plan, &indexed)?;
            }
        }
        Ok(())
    }

    pub fn step(&mut self, frames: u32) -> Result<(), Failed> {
        let groups: Vec<&[Groups]> = self.stages.iter().map(Vec::as_slice).collect();
        let mut done = 0_u32;
        while done < frames {
            let step = BATCH.min(frames - done);
            self.live.staged_frames(&groups, step)?;
            done += step;
            self.frame = self.frame.wrapping_add(step);
        }
        Ok(())
    }

    pub fn tally(&mut self) -> Result<Vec<u32>, Failed> {
        Ok(self.live.read(TALLY as usize, self.rule.tally_words())?)
    }

    pub fn cells(&mut self) -> Result<Vec<u32>, Failed> {
        match self.pool {
            Some(bricks) => Ok(bricks.read(&mut self.live)?),
            None => Ok(live::read_state(
                &mut self.live,
                self.plan.windows(),
                self.grid.cells(),
            )?),
        }
    }

    pub fn state_hash(&mut self) -> Result<[u8; 32], Failed> {
        let cells = self.cells()?;
        let mut hash = Sha256::new();
        hash.update(self.rule.name().as_bytes());
        hash.update(&[0]);
        for word in self.params.iter().chain(&[self.frame]).chain(&self.size()) {
            hash.update(&word.to_le_bytes());
        }
        for chunk in cells.chunks(1 << 16) {
            let bytes: Vec<u8> = chunk.iter().flat_map(|word| word.to_le_bytes()).collect();
            hash.update(&bytes);
        }
        Ok(hash.finish())
    }

    fn reach(&mut self) -> Result<Option<(Windows, usize)>, Failed> {
        let Some(bricks) = self.pool else {
            return Ok(None);
        };
        let table = self
            .live
            .read(bricks.table() as usize, bricks.table_words())?;
        Ok(Some((bricks.windows(), reach_of(&bricks, &table))))
    }

    pub fn image(&mut self) -> Result<Image, Failed> {
        let state = match self.pool {
            None => State::Dense(live::read_state(
                &mut self.live,
                self.plan.windows(),
                self.grid.cells(),
            )?),
            Some(bricks) => {
                bricks.check(&mut self.live)?;
                let table = self
                    .live
                    .read(bricks.table() as usize, bricks.table_words())?;
                let reach = reach_of(&bricks, &table);
                let raw = live::read_state(&mut self.live, bricks.windows(), reach)?;
                let per = bricks.cells();
                let mut canonical = Vec::with_capacity(bricks.count() as usize * 2);
                let mut pool = Vec::new();
                let mut held = 0_u32;
                for entry in table
                    .as_chunks::<ENTRY>()
                    .0
                    .iter()
                    .take(bricks.count() as usize)
                {
                    let slot = entry.first().copied().unwrap_or(UNIFORM);
                    if slot == UNIFORM {
                        canonical.extend([UNIFORM, entry.get(1).copied().unwrap_or_default()]);
                        continue;
                    }
                    let from = slot as usize * per;
                    let words = raw
                        .get(from..from + per)
                        .ok_or_else(|| Refused::Malformed {
                            what: format!("held slot {slot} lies past the pool read back"),
                        })?;
                    pool.extend_from_slice(words);
                    canonical.extend([held, 0]);
                    held += 1;
                }
                State::Sparse {
                    capacity: bricks.capacity(),
                    sides: bricks.sides(),
                    table: canonical,
                    pool,
                }
            }
        };
        Ok(Image {
            law: Law::Rule {
                name: self.rule.name().to_owned(),
            },
            params: self.params.clone(),
            frame: self.frame,
            size: self.size(),
            state,
        })
    }

    pub fn save(&mut self, path: &Path) -> Result<u64, Failed> {
        let image = self.image()?;
        Ok(format::store(path, &image)?)
    }

    pub fn save_with<W, F>(&mut self, path: &Path, wrap: F) -> Result<u64, Failed>
    where
        W: Write,
        F: FnOnce(File) -> W,
    {
        let image = self.image()?;
        Ok(format::store_with(path, &image, wrap)?)
    }

    pub fn listed(&mut self) -> Result<u32, Failed> {
        match self.pool {
            Some(bricks) => {
                let (octs, whole) = bricks.listed(&mut self.live)?;
                Ok(octs.saturating_add(whole.saturating_mul(bricks.per_block())))
            }
            None => Ok(self
                .live
                .read(crate::layout::GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0)),
        }
    }

    pub fn pool_use(&mut self) -> Result<Option<bricks::PoolUse>, Failed> {
        match self.pool {
            Some(bricks) => Ok(Some(bricks.pool_use(&mut self.live)?)),
            None => Ok(None),
        }
    }

    pub fn load(host: &'host Host, path: &Path) -> Result<Self, Failed> {
        let image = format::fetch(path)?;
        Self::open(host, &image)
    }
}

fn reach_of(bricks: &Bricks, table: &[u32]) -> usize {
    table
        .as_chunks::<ENTRY>()
        .0
        .iter()
        .take(bricks.count() as usize)
        .map(|entry| entry[0])
        .filter(|&slot| slot != UNIFORM)
        .map(|slot| (slot as usize + 1) * bricks.cells())
        .max()
        .unwrap_or(0)
}

fn repacked(
    image: &Image,
    plan: &Plan,
    grid: Grid,
    capacity: u32,
    sides: [u32; 3],
    table: &[u32],
    pool: &[u32],
) -> Result<(Bricks, Packed), Failed> {
    let refused = |why: LaneError| Box::new(unfit(why)) as Failed;
    let probe = Bricks::new(grid, plan, most_slots(plan).max(1)).map_err(refused)?;
    let same = probe.sides() == sides;
    let wanted = if same {
        u64::from(capacity)
    } else {
        (u64::from(capacity) * format::brick_cells(sides) as u64).div_ceil(probe.cells() as u64)
    };
    let fits = |wanted: u64| {
        let capacity = u32::try_from(wanted.clamp(1, u64::from(plan.blocks))).unwrap_or(u32::MAX);
        Bricks::new(grid, plan, capacity).map_err(refused)
    };
    let bricks = fits(wanted)?;
    let packed = if same {
        let mut entries = vec![0_u32; probe.table_words()];
        for (entry, pair) in entries
            .as_chunks_mut::<ENTRY>()
            .0
            .iter_mut()
            .zip(table.as_chunks::<2>().0)
        {
            if let Some(head) = entry.get_mut(..2) {
                head.copy_from_slice(pair);
            }
        }
        Packed {
            table: entries,
            pool: pool.to_vec(),
            slots: Vec::new(),
            held: u32::try_from(pool.len() / probe.cells()).unwrap_or(u32::MAX),
        }
    } else {
        let source = Source {
            size: image.size,
            sides,
            lattice: format::lattice_of(image.size, sides),
            per: format::brick_cells(sides),
            table,
            pool,
        };
        probe
            .compose(|brick| source.content(&probe, brick))
            .map_err(fitted)?
    };
    let bricks = if packed.held > bricks.capacity() {
        fits(u64::from(packed.held))?
    } else {
        bricks
    };
    let packed = bricks.restacked(packed).map_err(fitted)?;
    Ok((bricks, packed))
}

struct Source<'image> {
    size: [u32; 3],
    sides: [u32; 3],
    lattice: [u32; 3],
    per: usize,
    table: &'image [u32],
    pool: &'image [u32],
}

impl Source<'_> {
    fn entry(&self, brick: usize) -> (u32, u32) {
        (
            self.table.get(brick * 2).copied().unwrap_or(UNIFORM),
            self.table.get(brick * 2 + 1).copied().unwrap_or_default(),
        )
    }

    fn word(&self, point: [u32; 3]) -> u32 {
        let (brick, offset) = format::place(point, self.sides, self.lattice);
        let (slot, value) = self.entry(brick);
        if slot == UNIFORM {
            return value;
        }
        self.pool
            .get(slot as usize * self.per + offset)
            .copied()
            .unwrap_or_default()
    }

    fn uniform(&self, from: [u32; 3], sides: [u32; 3]) -> Option<u32> {
        let mut low = [0_u32; 3];
        let mut high = [0_u32; 3];
        for axis in 0..3 {
            let side = self.sides.get(axis).copied().unwrap_or(1).max(1);
            let start = from.get(axis).copied().unwrap_or_default();
            let span = sides.get(axis).copied().unwrap_or(1).max(1);
            let world = self.size.get(axis).copied().unwrap_or(1);
            if let (Some(lo), Some(hi)) = (low.get_mut(axis), high.get_mut(axis)) {
                *lo = start / side;
                *hi = (start + span - 1).min(world - 1) / side;
            }
        }
        let mut found = None;
        for z in low[2]..=high[2] {
            for y in low[1]..=high[1] {
                for x in low[0]..=high[0] {
                    let brick = ((z as usize * self.lattice[1] as usize) + y as usize)
                        * self.lattice[0] as usize
                        + x as usize;
                    let (slot, value) = self.entry(brick);
                    if slot != UNIFORM || found.is_some_and(|held| held != value) {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
        found
    }

    fn content(&self, target: &Bricks, brick: u32) -> Content {
        let from = target.point_of(brick, 0);
        if let Some(value) = self.uniform(from, target.sides()) {
            return Content::Uniform(value);
        }
        let words = (0..u32::try_from(target.cells()).unwrap_or(u32::MAX))
            .map(|offset| self.word(target.point_of(brick, offset)))
            .collect();
        Content::Mixed(words)
    }
}
