pub mod fork;
pub mod format;

use std::fs::File;
use std::io::Write;
use std::path::Path;

use simdr::lanes::LaneError;

use crate::bricks::{self, Bricks, Content, ENTRY, Packed, UNIFORM};
use crate::compact::{Plan, TooBig};
use crate::host::{self, Host};
use crate::layout::{Dimension, PARAMS, Windows};
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

const GATHER_GAP: usize = 16;

const RUN_COST: usize = 64;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stepping {
    Listed,
    Whole,
}

pub(crate) enum Stepper {
    Named(Box<dyn Listed>),
    Law(Box<crate::law::Law>, Stepping),
}

impl Stepper {
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Named(rule) => rule.name(),
            Self::Law(law, _) => law.name(),
        }
    }

    fn tally_words(&self) -> usize {
        match self {
            Self::Named(rule) => rule.tally_words(),
            Self::Law(law, _) => crate::rule3::Declared::tally(&**law).words(),
        }
    }

    pub(crate) fn planes(&self) -> u32 {
        match self {
            Self::Named(_) => 1,
            Self::Law(law, _) => law.channels().max(1),
        }
    }

    fn wired(&self, subgroup: u32, grid: Grid, plan: &Plan) -> Result<live::Loop, LaneError> {
        match self {
            Self::Named(rule) => rule.wired(subgroup, grid, plan),
            Self::Law(law, _) => {
                live::wired3_switched_by(&**law, subgroup, grid, plan, switch_of(law, plan))
            }
        }
    }

    fn laid(&self, plan: &Plan) -> Vec<usize> {
        match self {
            Self::Named(rule) => plan.windows().laid(&rule.sizes(plan)),
            Self::Law(law, _) => {
                let mut sizes = live::sizes3(&**law, plan);
                if let Some(params) = sizes.get_mut(PARAMS as usize) {
                    *params += 1;
                }
                live::windows3(&**law, plan).laid(&sizes)
            }
        }
    }

    pub(crate) fn windows(&self, plan: &Plan) -> Windows {
        match self {
            Self::Named(_) => plan.windows(),
            Self::Law(law, _) => live::windows3(&**law, plan),
        }
    }

    fn bricked(&self, bricks: &Bricks, plan: &Plan) -> Result<live::Loop, LaneError> {
        match self {
            Self::Named(rule) => rule.bricked(bricks, plan),
            Self::Law(law, _) => bricks::arranged(&**law, bricks, plan, true),
        }
    }

    fn bricked_sizes(&self, bricks: &Bricks, plan: &Plan) -> Vec<usize> {
        match self {
            Self::Named(rule) => rule.bricked_sizes(bricks, plan),
            Self::Law(law, _) => bricks.sizes(&**law, plan),
        }
    }

    pub(crate) fn again(&self, host: &Host, params: &[u32]) -> Result<Self, Refused> {
        match self {
            Self::Named(rule) => Ok(Self::Named(ruled(host, rule.name(), params)?)),
            Self::Law(law, stepping) => Ok(Self::Law(law.clone(), *stepping)),
        }
    }

    fn law(&self) -> Law {
        match self {
            Self::Named(rule) => Law::Rule {
                name: rule.name().to_owned(),
            },
            Self::Law(law, _) => Law::Law {
                name: law.name().to_owned(),
                canonical: law.canonical().to_owned(),
            },
        }
    }
}

fn switch_of(law: &crate::law::Law, plan: &Plan) -> u32 {
    let words = live::sizes3(law, plan)
        .get(PARAMS as usize)
        .copied()
        .unwrap_or(FIRST_PARAM);
    u32::try_from(words).unwrap_or(u32::MAX)
}

fn lawful(law: &crate::law::Law, params: &[u32], stepping: Stepping) -> Result<Stepper, Refused> {
    let wanted = crate::rule3::Declared::params(law);
    if params.len() != wanted {
        return Err(Refused::Params {
            rule: law.name().to_owned(),
            given: params.len(),
            wanted,
        });
    }
    Ok(Stepper::Law(Box::new(law.clone()), stepping))
}

fn stepper_of(
    host: &Host,
    law: &Law,
    params: &[u32],
    stepping: Stepping,
) -> Result<Stepper, Failed> {
    match law {
        Law::Rule { name } => Ok(Stepper::Named(ruled(host, name, params)?)),
        Law::Law { canonical, .. } => {
            let compiled = crate::law::Law::compile(canonical).map_err(|why| {
                Box::new(Refused::Malformed {
                    what: format!("the law: {why}"),
                }) as Failed
            })?;
            Ok(lawful(&compiled, params, stepping)?)
        }
    }
}

fn listed_if_sparse(sparse: bool, stepping: Stepping) -> Result<(), Failed> {
    if sparse && stepping == Stepping::Whole {
        return Err(Box::new(Refused::Malformed {
            what: "a sparse world steps only the octs it lists".to_owned(),
        }));
    }
    Ok(())
}

fn pooled(bricks: Bricks, rule: &Stepper) -> Result<Bricks, Failed> {
    bricks
        .with_planes(rule.planes())
        .map_err(|why| Box::new(unfit(why)) as Failed)
}

fn pool_for(
    (grid, plan): &(Grid, Plan),
    made: &Made<'_>,
    rule: &Stepper,
) -> Result<Option<Bricks>, Failed> {
    if !made.sparse {
        return Ok(None);
    }
    let brick_cells = u64::from(plan.per_block) * 8;
    let least = u32::try_from(POOL_LEAST_CELLS.div_ceil(brick_cells)).unwrap_or(u32::MAX);
    let capacity = made
        .slots
        .unwrap_or((plan.blocks / 4).max(least))
        .clamp(1, most_slots(plan));
    Ok(Some(pooled(Bricks::new(*grid, plan, capacity)?, rule)?))
}

pub fn sizes_of(host: &Host, made: &Made<'_>) -> Result<Vec<usize>, Failed> {
    let rule = Stepper::Named(ruled(host, made.rule, made.params)?);
    sizes_by(host, made, &rule)
}

pub fn sizes_of_law(
    host: &Host,
    law: &crate::law::Law,
    made: &Made<'_>,
) -> Result<Vec<usize>, Failed> {
    let rule = lawful(law, made.params, Stepping::Listed)?;
    sizes_by(host, made, &rule)
}

fn sizes_by(host: &Host, made: &Made<'_>, rule: &Stepper) -> Result<Vec<usize>, Failed> {
    let shape = planned(host.width(), made.size, made.sparse)?;
    Ok(match pool_for(&shape, made, rule)? {
        Some(bricks) => rule.bricked_sizes(&bricks, &shape.1),
        None => rule.laid(&shape.1),
    })
}

pub struct World<'host> {
    host: &'host Host,
    rule: Stepper,
    passes: Vec<Vec<u32>>,
    stages: Vec<Vec<Groups>>,
    sizes: Vec<usize>,
    grid: Grid,
    plan: Plan,
    pool: Option<Bricks>,
    params: Vec<u32>,
    frame: u32,
    live: Resident<'host>,
    words_read: u64,
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
        rule: Stepper,
        params: Vec<u32>,
        (grid, plan): (Grid, Plan),
        pool: Option<Bricks>,
    ) -> Result<Self, Failed> {
        let ((passes, stages), sizes) = match pool.as_ref() {
            Some(bricks) => (
                rule.bricked(bricks, &plan)?,
                rule.bricked_sizes(bricks, &plan),
            ),
            None => (rule.wired(host.width(), grid, &plan)?, rule.laid(&plan)),
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
            words_read: 0,
        })
    }

    pub fn create(host: &'host Host, made: &Made<'_>) -> Result<Self, Failed> {
        let rule = Stepper::Named(ruled(host, made.rule, made.params)?);
        Self::made(host, rule, made)
    }

    pub fn create_law(
        host: &'host Host,
        law: &crate::law::Law,
        made: &Made<'_>,
    ) -> Result<Self, Failed> {
        if made.rule != law.name() {
            return Err(Box::new(Refused::Rule {
                name: made.rule.to_owned(),
            }));
        }
        Self::create_law_stepped(host, law, made, Stepping::Listed)
    }

    pub fn create_law_stepped(
        host: &'host Host,
        law: &crate::law::Law,
        made: &Made<'_>,
        stepping: Stepping,
    ) -> Result<Self, Failed> {
        if made.rule != law.name() {
            return Err(Box::new(Refused::Rule {
                name: made.rule.to_owned(),
            }));
        }
        listed_if_sparse(made.sparse, stepping)?;
        let rule = lawful(law, made.params, stepping)?;
        Self::made(host, rule, made)
    }

    #[cfg(feature = "faults")]
    pub fn create_split(host: &'host Host, made: &Made<'_>, bits: u32) -> Result<Self, Failed> {
        let rule = Stepper::Named(ruled(host, made.rule, made.params)?);
        let (grid, plan) = planned(host.width(), made.size, made.sparse)?;
        let plan = plan.split_at(bits);
        let pool = match pool_for(&(grid, plan), made, &rule)? {
            Some(bricks) => Some(pooled(
                Bricks::split_at(grid, &plan, bricks.capacity(), bits)?,
                &rule,
            )?),
            None => None,
        };
        Self::made_from(host, rule, made, (grid, plan), pool)
    }

    fn made(host: &'host Host, rule: Stepper, made: &Made<'_>) -> Result<Self, Failed> {
        let shape = planned(host.width(), made.size, made.sparse)?;
        let pool = pool_for(&shape, made, &rule)?;
        Self::made_from(host, rule, made, shape, pool)
    }

    fn made_from(
        host: &'host Host,
        rule: Stepper,
        made: &Made<'_>,
        shape: (Grid, Plan),
        pool: Option<Bricks>,
    ) -> Result<Self, Failed> {
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
        if let (Stepper::Law(law, stepping), None) = (&self.rule, self.pool) {
            let whole = u32::from(*stepping == Stepping::Whole);
            let switch = switch_of(law, &self.plan) as usize;
            self.live.write_at(PARAMS as usize, switch, &[whole])?;
        }
        if self.params.is_empty() {
            return Ok(());
        }
        self.live
            .write_at(PARAMS as usize, FIRST_PARAM, &self.params)
    }

    pub fn open(host: &'host Host, image: &Image) -> Result<Self, Failed> {
        Self::open_stepped(host, image, Stepping::Listed)
    }

    pub fn open_stepped(
        host: &'host Host,
        image: &Image,
        stepping: Stepping,
    ) -> Result<Self, Failed> {
        listed_if_sparse(matches!(image.state, State::Sparse { .. }), stepping)?;
        let rule = stepper_of(host, &image.law, &image.params, stepping)?;
        if rule.planes() != image.channels() {
            return Err(Box::new(Refused::Sizes {
                what: "channels",
                header: u64::from(image.channels()),
                found: u64::from(rule.planes()),
            }));
        }
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
                let windows = world.rule.windows(&world.plan);
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
                let (bricks, packed) = repacked(
                    image,
                    (&plan, grid, rule.planes()),
                    *capacity,
                    *sides,
                    table,
                    pool,
                )?;
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
    pub fn law(&self) -> Option<&crate::law::Law> {
        match &self.rule {
            Stepper::Named(_) => None,
            Stepper::Law(law, _) => Some(law),
        }
    }

    #[must_use]
    pub const fn stepping(&self) -> Stepping {
        match &self.rule {
            Stepper::Named(_) => Stepping::Listed,
            Stepper::Law(_, stepping) => *stepping,
        }
    }

    pub fn set_params(&mut self, params: &[u32], stepping: Stepping) -> Result<(), Failed> {
        let stepping = match &self.rule {
            Stepper::Named(rule) => {
                if params.len() != rule.params() {
                    return Err(Box::new(Refused::Params {
                        rule: rule.name().to_owned(),
                        given: params.len(),
                        wanted: rule.params(),
                    }));
                }
                Stepping::Listed
            }
            Stepper::Law(law, _) => {
                let wanted = crate::rule3::Declared::params(&**law);
                if params.len() != wanted {
                    return Err(Box::new(Refused::Params {
                        rule: law.name().to_owned(),
                        given: params.len(),
                        wanted,
                    }));
                }
                stepping
            }
        };
        listed_if_sparse(self.pool.is_some(), stepping)?;
        if let Stepper::Law(_, held) = &mut self.rule {
            *held = stepping;
        }
        self.params = params.to_vec();
        self.parameters()?;
        self.reseed()?;
        self.live
            .write(PARAMS as usize, &[self.frame.wrapping_sub(1)])?;
        Ok(())
    }

    pub fn uniforms(&mut self) -> Result<Vec<Vec<u32>>, Failed> {
        let Some(bricks) = self.pool else {
            return Ok(Vec::new());
        };
        let table = self
            .live
            .read(bricks.table() as usize, bricks.table_words())?;
        let mut found = std::collections::BTreeSet::new();
        for brick in 0..bricks.count() {
            let slot = table
                .get(brick as usize * ENTRY)
                .copied()
                .unwrap_or(UNIFORM);
            if slot != UNIFORM {
                continue;
            }
            let value: Vec<u32> = (0..bricks.planes())
                .map(|plane| {
                    table
                        .get(bricks.value_at(plane, brick))
                        .copied()
                        .unwrap_or_default()
                })
                .collect();
            found.insert(value);
        }
        Ok(found.into_iter().collect())
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
        self.write_plane(0, cells)
    }

    #[must_use]
    pub fn planes(&self) -> u32 {
        self.rule.planes()
    }

    pub fn write_plane(&mut self, plane: u32, cells: &[([u32; 3], u32)]) -> Result<(), Failed> {
        if plane >= self.planes() {
            return Err(Box::new(Refused::Malformed {
                what: format!("plane {plane} is past the world's {} planes", self.planes()),
            }));
        }
        let size = self.size();
        if let Some(&(at, _)) = cells
            .iter()
            .find(|(at, _)| at.iter().zip(size).any(|(one, side)| *one >= side))
        {
            return Err(Box::new(Refused::Outside { at, size }));
        }
        match self.pool {
            Some(bricks) => bricks.put_plane_at(&mut self.live, plane as usize, cells)?,
            None => {
                let mut indexed = Vec::with_capacity(cells.len());
                for &(at, word) in cells {
                    let index = (u64::from(at[2]) * u64::from(size[1]) + u64::from(at[1]))
                        * u64::from(size[0])
                        + u64::from(at[0]);
                    indexed.push((u32::try_from(index)?, word));
                }
                let placed = live::Placed3 {
                    windows: Some(self.rule.windows(&self.plan)),
                    plane,
                };
                live::put3_in(&mut self.live, self.grid, &self.plan, placed, &indexed)?;
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

    #[must_use]
    pub fn sizes(&self) -> &[usize] {
        &self.sizes
    }

    pub fn set_patience(&mut self, patience: std::time::Duration) {
        self.live.set_patience(patience);
    }

    pub fn exact_tally(&mut self) -> Result<Vec<u128>, Failed> {
        Ok(live::read_tally(&mut self.live, self.rule.tally_words())?)
    }

    pub fn tally(&mut self) -> Result<Vec<u128>, Failed> {
        self.exact_tally()
    }

    pub fn cells_at(&mut self, points: &[[u32; 3]]) -> Result<Vec<u32>, Failed> {
        self.cells_at_plane(points, 0)
    }

    #[must_use]
    pub const fn words_read(&self) -> u64 {
        self.words_read
    }

    #[must_use]
    pub const fn read_waits(&self) -> u64 {
        self.live.copying().1
    }

    fn gathered(&mut self, wanted: &[(u32, usize)]) -> Result<Vec<u32>, Failed> {
        let mut order: Vec<usize> = (0..wanted.len()).collect();
        order.sort_unstable_by_key(|&at| wanted.get(at).copied());
        let mut runs: Vec<(u32, usize, usize)> = Vec::new();
        for &at in &order {
            let Some(&(binding, index)) = wanted.get(at) else {
                continue;
            };
            match runs.last_mut() {
                Some((held, _, last))
                    if *held == binding && index <= last.saturating_add(GATHER_GAP) =>
                {
                    *last = (*last).max(index);
                }
                _ => runs.push((binding, index, index)),
            }
        }
        let mut bindings: Vec<u32> = runs.iter().map(|&(binding, _, _)| binding).collect();
        bindings.dedup();
        let mut reads: Vec<(usize, usize, usize)> = Vec::new();
        for binding in bindings {
            let mine: Vec<(usize, usize)> = runs
                .iter()
                .filter(|&&(held, _, _)| held == binding)
                .map(|&(_, from, last)| (from, last - from + 1))
                .collect();
            let whole = self.sizes.get(binding as usize).copied().unwrap_or(0);
            let cost: usize = mine
                .iter()
                .map(|&(_, count)| count.saturating_add(RUN_COST))
                .fold(0, usize::saturating_add);
            if cost >= whole && whole > 0 {
                reads.push((binding as usize, 0, whole));
            } else {
                reads.extend(
                    mine.iter()
                        .map(|&(from, count)| (binding as usize, from, count)),
                );
            }
        }
        let got = self.live.read_runs(&reads)?;
        let read: usize = got.iter().map(Vec::len).sum();
        self.words_read = self.words_read.saturating_add(read as u64);
        let mut out = Vec::with_capacity(wanted.len());
        for &(binding, index) in wanted {
            let key = (binding as usize, index);
            let place = reads
                .partition_point(|&(held, from, _)| (held, from) <= key)
                .checked_sub(1);
            let word = place
                .and_then(|place| Some((reads.get(place)?, got.get(place)?)))
                .filter(|((held, _, _), _)| *held == binding as usize)
                .and_then(|((_, from, _), words)| words.get(index.checked_sub(*from)?))
                .copied()
                .ok_or("the world read back short")?;
            out.push(word);
        }
        Ok(out)
    }

    pub fn cells_at_plane(&mut self, points: &[[u32; 3]], plane: u32) -> Result<Vec<u32>, Failed> {
        if plane >= self.planes() {
            return Err(Box::new(Refused::Malformed {
                what: format!("plane {plane} is past the world's {} planes", self.planes()),
            }));
        }
        let size = self.size();
        if let Some(&at) = points
            .iter()
            .find(|at| at.iter().zip(size).any(|(one, side)| *one >= side))
        {
            return Err(Box::new(Refused::Outside { at, size }));
        }
        if points.is_empty() {
            return Ok(Vec::new());
        }
        let Some(bricks) = self.pool else {
            let windows = self.rule.windows(&self.plan);
            let cells = self.grid.cells();
            let wanted: Vec<(u32, usize)> = points
                .iter()
                .map(|at| {
                    let index = (at[2] as usize * size[1] as usize + at[1] as usize)
                        * size[0] as usize
                        + at[0] as usize;
                    windows.place(plane as usize * cells + index)
                })
                .collect();
            return self.gathered(&wanted);
        };
        let places: Vec<bricks::Cell<u32>> = points.iter().map(|&at| bricks.cell_of(at)).collect();
        let table = bricks.table();
        let mut first: Vec<(u32, usize)> =
            (0..bricks::HEAD).map(|at| (bricks.slots(), at)).collect();
        first.extend(
            places
                .iter()
                .map(|place| (table, place.brick as usize * ENTRY)),
        );
        first.extend(
            places
                .iter()
                .map(|place| (table, bricks.value_at(plane as usize, place.brick))),
        );
        let first = self.gathered(&first)?;
        let (head, entries) = first.split_at(bricks::HEAD.min(first.len()));
        if head.get(bricks::OVERFLOW).copied().unwrap_or_default() != 0 {
            return Err(Box::new(host::Error::PoolFull {
                slots: bricks.capacity(),
            }));
        }
        if head.get(bricks::LOST).copied().unwrap_or_default() != 0 {
            return Err(Box::new(host::Error::LostWrite));
        }
        let (slots, values) = entries.split_at(places.len().min(entries.len()));
        let windows = bricks.windows();
        let start = bricks.plane_start(plane as usize) as usize;
        let held: Vec<(usize, (u32, usize))> = places
            .iter()
            .zip(slots)
            .enumerate()
            .filter(|&(_, (_, &slot))| slot != UNIFORM)
            .map(|(at, (place, &slot))| {
                let index = start + slot as usize * bricks.cells() + place.offset as usize;
                (at, windows.place(index))
            })
            .collect();
        let mut out = values.to_vec();
        if held.is_empty() {
            return Ok(out);
        }
        let wanted: Vec<(u32, usize)> = held.iter().map(|&(_, place)| place).collect();
        let words = self.gathered(&wanted)?;
        for (&(at, _), word) in held.iter().zip(words) {
            if let Some(slot) = out.get_mut(at) {
                *slot = word;
            }
        }
        Ok(out)
    }

    pub fn cells(&mut self) -> Result<Vec<u32>, Failed> {
        match self.pool {
            Some(bricks) => Ok(bricks.read(&mut self.live)?),
            None => Ok(live::read_state(
                &mut self.live,
                self.rule.windows(&self.plan),
                self.grid.cells() * self.rule.planes() as usize,
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
        Ok(Some((bricks.windows(), planes_reach(&bricks, &table))))
    }

    pub fn image(&mut self) -> Result<Image, Failed> {
        let state = match self.pool {
            None => State::Dense(live::read_state(
                &mut self.live,
                self.rule.windows(&self.plan),
                self.grid.cells() * self.rule.planes() as usize,
            )?),
            Some(bricks) => {
                bricks.check(&mut self.live)?;
                let table = self
                    .live
                    .read(bricks.table() as usize, bricks.table_words())?;
                let reach = planes_reach(&bricks, &table);
                let raw = live::read_state(&mut self.live, bricks.windows(), reach)?;
                let per = bricks.cells();
                let count = bricks.count() as usize;
                let planes = bricks.planes();
                let mut canonical = Vec::with_capacity(count * (planes + 1));
                let mut values: Vec<Vec<u32>> = vec![Vec::with_capacity(count); planes - 1];
                let mut pools: Vec<Vec<u32>> = vec![Vec::new(); planes];
                let mut held = 0_u32;
                for (brick, entry) in (0_u32..).zip(table.as_chunks::<ENTRY>().0.iter().take(count))
                {
                    let slot = entry.first().copied().unwrap_or(UNIFORM);
                    if slot == UNIFORM {
                        canonical.extend([UNIFORM, entry.get(1).copied().unwrap_or_default()]);
                        for (plane, plane_values) in (1..).zip(values.iter_mut()) {
                            plane_values.push(
                                table
                                    .get(bricks.value_at(plane, brick))
                                    .copied()
                                    .unwrap_or_default(),
                            );
                        }
                        continue;
                    }
                    let mut planes_words = Vec::with_capacity(planes);
                    for plane in 0..planes {
                        let from = plane * bricks.plane_words() + slot as usize * per;
                        planes_words.push(raw.get(from..from + per).ok_or_else(|| {
                            Refused::Malformed {
                                what: format!("held slot {slot} lies past the pool read back"),
                            }
                        })?);
                    }
                    let uniform: Option<Vec<u32>> = planes_words
                        .iter()
                        .map(|words| {
                            let first = words.first().copied().unwrap_or_default();
                            words.iter().all(|&word| word == first).then_some(first)
                        })
                        .collect();
                    if let Some(uniform) = uniform {
                        canonical.extend([UNIFORM, uniform.first().copied().unwrap_or_default()]);
                        for (plane_values, &value) in values.iter_mut().zip(uniform.iter().skip(1))
                        {
                            plane_values.push(value);
                        }
                        continue;
                    }
                    for (pool, words) in pools.iter_mut().zip(planes_words) {
                        pool.extend_from_slice(words);
                    }
                    for plane_values in &mut values {
                        plane_values.push(0);
                    }
                    canonical.extend([held, 0]);
                    held += 1;
                }
                canonical.extend(values.concat());
                State::Sparse {
                    capacity: bricks.capacity(),
                    sides: bricks.sides(),
                    table: canonical,
                    pool: pools.concat(),
                }
            }
        };
        Ok(Image {
            law: self.rule.law(),
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

fn planes_reach(bricks: &Bricks, table: &[u32]) -> usize {
    let reach = reach_of(bricks, table);
    if bricks.planes() <= 1 || reach == 0 {
        return reach;
    }
    (bricks.planes() - 1) * bricks.plane_words() + reach
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
    (plan, grid, planes): (&Plan, Grid, u32),
    capacity: u32,
    sides: [u32; 3],
    table: &[u32],
    pool: &[u32],
) -> Result<(Bricks, Packed), Failed> {
    let refused = |why: LaneError| Box::new(unfit(why)) as Failed;
    let probe = Bricks::new(grid, plan, most_slots(plan).max(1))
        .and_then(|probe| probe.with_planes(planes))
        .map_err(refused)?;
    let same = probe.sides() == sides;
    let wanted = if same {
        u64::from(capacity)
    } else {
        (u64::from(capacity) * format::brick_cells(sides) as u64).div_ceil(probe.cells() as u64)
    };
    let fits = |wanted: u64| {
        let capacity = u32::try_from(wanted.clamp(1, u64::from(plan.blocks))).unwrap_or(u32::MAX);
        Bricks::new(grid, plan, capacity)
            .and_then(|bricks| bricks.with_planes(planes))
            .map_err(refused)
    };
    let bricks = fits(wanted)?;
    let count = probe.count() as usize;
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
        for plane in 1..probe.planes() {
            let from = 2 * count + (plane - 1) * count;
            let base = probe.plane_values(plane) as usize;
            if let (Some(into), Some(given)) = (
                entries.get_mut(base..base + count),
                table.get(from..from + count),
            ) {
                into.copy_from_slice(given);
            }
        }
        Packed {
            table: entries,
            pool: pool.to_vec(),
            slots: Vec::new(),
            held: u32::try_from(pool.len() / (probe.cells() * probe.planes())).unwrap_or(u32::MAX),
        }
    } else {
        let lattice = format::lattice_of(image.size, sides);
        let per = format::brick_cells(sides);
        let source = Source {
            size: image.size,
            sides,
            lattice,
            per,
            bricks: lattice.iter().map(|&side| side as usize).product(),
            held: pool.len() / (per * planes as usize).max(1),
            table,
            pool,
        };
        probe
            .compose_planes(|brick| source.contents(&probe, brick))
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
    bricks: usize,
    held: usize,
    table: &'image [u32],
    pool: &'image [u32],
}

impl Source<'_> {
    fn entry(&self, brick: usize, plane: usize) -> (u32, u32) {
        let value = match plane {
            0 => self.table.get(brick * 2 + 1),
            _ => self
                .table
                .get(2 * self.bricks + (plane - 1) * self.bricks + brick),
        };
        (
            self.table.get(brick * 2).copied().unwrap_or(UNIFORM),
            value.copied().unwrap_or_default(),
        )
    }

    fn word(&self, point: [u32; 3], plane: usize) -> u32 {
        let (brick, offset) = format::place(point, self.sides, self.lattice);
        let (slot, value) = self.entry(brick, plane);
        if slot == UNIFORM {
            return value;
        }
        self.pool
            .get(plane * self.held * self.per + slot as usize * self.per + offset)
            .copied()
            .unwrap_or_default()
    }

    fn uniform(&self, from: [u32; 3], sides: [u32; 3], plane: usize) -> Option<u32> {
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
                    let (slot, value) = self.entry(brick, plane);
                    if slot != UNIFORM || found.is_some_and(|held| held != value) {
                        return None;
                    }
                    found = Some(value);
                }
            }
        }
        found
    }

    fn contents(&self, target: &Bricks, brick: u32) -> Vec<Content> {
        let from = target.point_of(brick, 0);
        (0..target.planes())
            .map(|plane| {
                if let Some(value) = self.uniform(from, target.sides(), plane) {
                    return Content::Uniform(value);
                }
                let words = (0..u32::try_from(target.cells()).unwrap_or(u32::MAX))
                    .map(|offset| self.word(target.point_of(brick, offset), plane))
                    .collect();
                Content::Mixed(words)
            })
            .collect()
    }
}
