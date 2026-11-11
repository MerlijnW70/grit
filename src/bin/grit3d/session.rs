use std::time::Duration;

use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{DENSE, Dimension, GRAND, PARAMS, PIXELS, STATE, TALLY};
use grit::render3::{
    self, Build, Camera, Frame, HIT_WORDS, INSIDE, Light, MISS, Screen, Steps, framed_words, stage,
};
use grit::rule3::code::Code;
use grit::rule3::heat::Heat;
use grit::rule3::nature::Nature;
use grit::rule3::sand::Sand;
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::Grid;
use grit::view3::Kinds;
use grit::{Groups, Host, Resident, live, sweep};

use super::life::{Act, Cycle, Phase};
use super::scenes::{Example, Tool, Which};
use super::valley::FEED_EVERY;

/// How many engine steps apart a burning world's fire and smoke are
/// counted.
pub const COUNT_EVERY: u32 = 4;

pub type Failed = Box<dyn std::error::Error>;

/// How widely the sun rays of the faces and of the floor spread.
pub const SPREAD: f32 = 0.02;
pub const FLOOR_SPREAD: f32 = 0.07;

/// The points of each pixel the camera samples, one a frame, in pixels
/// from its centre.
const SHIFTS: [[f32; 2]; 8] = [
    [0.0, -0.1667],
    [-0.25, 0.1667],
    [0.25, -0.3889],
    [-0.375, -0.0556],
    [0.125, 0.2778],
    [-0.125, -0.2778],
    [0.375, 0.0556],
    [-0.4375, 0.3889],
];

/// What a session draws with: the scene, the grid side, the screen, and the
/// sun rays of a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    pub which: Which,
    pub side: u32,
    pub screen: Screen,
    pub suns: u32,
}

/// A running scene on the device with its picture's passes.
pub struct Session<'h> {
    pub held: Resident<'h>,
    pub build: Build,
    pub plan: Plan,
    pub example: Example,
    pub setup: Setup,
    stages: Vec<Vec<Groups>>,
    still: Vec<Vec<Groups>>,
    pub watcher: usize,
    pub engine_stages: usize,
    drawn: u32,
    before: Option<Camera>,
    pub dusk: f32,
    pub steps: u32,
    pub life: Option<Cycle>,
    pub feed: Vec<(u32, u32)>,
    /// The bytes of the buffers the session holds on the device.
    pub bytes: usize,
    start: Option<Vec<u32>>,
    pub restarted: bool,
}

impl<'h> Session<'h> {
    /// The session of `setup`, its world written and warmed up. The scene
    /// words are written before the warm-up, so the bricks the engine's
    /// frames rebuild know which kinds are seen, gas or glowing.
    pub fn open(host: &'h Host, setup: Setup) -> Result<Self, Failed> {
        let mut session = Self::cold(host, setup)?;
        session.warm(session.example.warm)?;
        if session.life.is_some() {
            session.start = Some(session.state()?);
        }
        Ok(session)
    }

    /// The session of `setup`, its world written, with no warm-up.
    pub fn cold(host: &'h Host, setup: Setup) -> Result<Self, Failed> {
        let example = setup.which.example();
        let mut session = match setup.which {
            Which::Valley => Self::with(host, &Nature, example, setup)?,
            Which::Sand => Self::with(host, &Sand, example, setup)?,
            Which::Fire => Self::with(host, &Heat, example, setup)?,
            Which::Code => Self::with(host, &Code, example, setup)?,
        };
        let side = setup.side as f32;
        let camera = Camera::looking(
            [side * 1.5, -side * 0.8, side * 1.5],
            [side * 0.5, side * 0.7, side * 0.5],
            VIEW,
        );
        session.aim(&camera, false, true)?;
        session.before = None;
        Ok(session)
    }

    fn with<R: Rule>(
        host: &'h Host,
        rule: &R,
        example: Example,
        setup: Setup,
    ) -> Result<Self, Failed> {
        let width = host.width();
        let grid = Grid::solid(setup.side, setup.side, setup.side)?;
        if !render3::fits(grid, setup.screen) {
            return Err(render3::UNFIT.into());
        }
        let kinds = Kinds::named(example.rule).ok_or("no kinds for the rule")?;
        let build = Build {
            grid,
            kinds,
            screen: setup.screen,
            subgroup: width,
            suns: setup.suns,
        };
        let plan = Plan::over_in(
            width,
            grid.blocks().div_ceil(sweep::cells(width)),
            Dimension::Three,
        )?;
        let engine = live::wired3_by(rule, width, grid, &plan)?;
        let listed = Plan::listed();
        let watcher = engine
            .1
            .iter()
            .position(|stage| stage == &vec![listed])
            .ok_or("the engine has no listed step")?
            + 1;
        let (passes, stages) =
            render3::watching(engine, build.passes()?).ok_or("the engine has no listed step")?;
        let layout = build.layout();
        let sizes = layout.sized(live::sizes3(rule, &plan));
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let bytes = sizes.iter().sum::<usize>() * 4;
        let mut held = host.resident(&sizes, &modules)?;
        for (binding, words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; *words])?;
        }
        let world = (example.scene)(grid);
        held.write(STATE as usize, &world)?;
        let life = example.life.map(|made| made(grid, &world));
        let feed = example
            .feed
            .map_or_else(Vec::new, |made| made(grid, &world));
        drop(world);
        if !example.params.is_empty() {
            held.write_at(PARAMS as usize, FIRST_PARAM, &example.params)?;
        }
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
        live::seed(&mut held, grid, &plan)?;
        held.write_at(PIXELS as usize, layout.table(), &example.table.words())?;
        let still = stages
            .iter()
            .map(|stage| stage.iter().map(|_| Groups::Fixed(0)).collect())
            .collect();
        let engine_stages = stages.len();
        let dusk = example.dusk;
        Ok(Self {
            held,
            build,
            plan,
            example,
            setup,
            stages,
            still,
            watcher,
            engine_stages,
            drawn: 0,
            before: None,
            dusk,
            steps: 0,
            life,
            feed,
            bytes,
            start: None,
            restarted: false,
        })
    }

    pub fn grid(&self) -> Grid {
        self.build.grid
    }

    pub fn light(&self) -> Light {
        (self.example.light)(Light::at(self.build.screen.high, VIEW, self.dusk))
    }

    /// Writes the scene words of the next frame, seen from `camera`, with
    /// the overlay of active octs when `active`. The frame keeps nothing of
    /// the frames before when `fresh` or when it is the first.
    pub fn aim(&mut self, camera: &Camera, active: bool, fresh: bool) -> Result<(), Failed> {
        let grid = self.build.grid;
        let screen = self.build.screen;
        let light = self.light();
        let shift = SHIFTS
            .get(self.drawn as usize % SHIFTS.len())
            .copied()
            .unwrap_or([0.0, 0.0]);
        let rays = camera.rays_shifted(grid, screen, shift);
        let reset = fresh || self.before.is_none();
        let before = self.before.unwrap_or(*camera).before(grid, screen);
        let mut flags = 0;
        if reset {
            flags |= render3::flags::RESET;
        }
        if active {
            flags |= render3::flags::ACTIVE;
        }
        let frame = Frame {
            suns: light.sun_rays(self.build.suns as usize, SPREAD),
            floor: FLOOR_SPREAD,
            before,
            flags,
            drawn: self.drawn,
        };
        let words = framed_words(&rays, &self.example.table, &frame, &light);
        self.held
            .write_at(PIXELS as usize, self.build.layout().scene(), &words)?;
        self.before = Some(*camera);
        Ok(())
    }

    /// The stages of a frame: the engine's when `moving`, then the picture's
    /// with the optional ones `steps` asks for.
    pub fn stages(&self, moving: bool, steps: Steps) -> Vec<Vec<Groups>> {
        let mut all = if moving {
            self.stages.clone()
        } else {
            self.still.clone()
        };
        all.extend(self.build.stages(steps));
        all
    }

    /// Runs a frame and gives its time and, when `reading`, its colours.
    pub fn frame(
        &mut self,
        moving: bool,
        steps: Steps,
        reading: bool,
    ) -> Result<(Duration, Vec<u32>), Failed> {
        if moving {
            self.fed()?;
        }
        let all = self.stages(moving, steps);
        let groups: Vec<&[Groups]> = all.iter().map(Vec::as_slice).collect();
        let out = if reading {
            self.held
                .staged_reading(&groups, PIXELS as usize, self.build.screen.pixels())?
        } else {
            (self.held.staged(&groups)?, Vec::new())
        };
        self.drawn = self.drawn.wrapping_add(1);
        if moving {
            self.steps = self.steps.wrapping_add(1);
            self.lived()?;
        }
        Ok(out)
    }

    /// The fire and smoke cells of the world after the last step, as the
    /// rule's tally counts them.
    pub fn counts(&mut self) -> Result<(u32, u32), Failed> {
        let words = self.held.read(TALLY as usize, 3)?;
        Ok((
            words.get(1).copied().unwrap_or(0),
            words.get(2).copied().unwrap_or(0),
        ))
    }

    /// Takes the scene's own cycle one step on, and does what it asks.
    fn lived(&mut self) -> Result<(), Failed> {
        let burning = self
            .life
            .as_ref()
            .is_some_and(|life| life.phase == Phase::Burning);
        let counted = if burning && self.steps.is_multiple_of(COUNT_EVERY) {
            Some(self.counts()?)
        } else {
            None
        };
        let Some(life) = self.life.as_mut() else {
            return Ok(());
        };
        match life.stepped(counted) {
            Act::Nothing => {}
            Act::Light(cells) => {
                live::put3(&mut self.held, self.build.grid, &self.plan, &cells)?;
            }
            Act::Restart => self.restart()?,
        }
        Ok(())
    }

    /// Puts the world back as it was after its warm-up, at the frame it
    /// was then.
    pub fn restart(&mut self) -> Result<(), Failed> {
        let Some(start) = self.start.as_ref() else {
            return Ok(());
        };
        self.held.write(STATE as usize, start)?;
        live::seed(&mut self.held, self.build.grid, &self.plan)?;
        let warm = self.example.warm;
        self.held
            .write_at(PARAMS as usize, 0, &[warm.wrapping_sub(1)])?;
        self.steps = warm;
        self.restarted = true;
        Ok(())
    }

    /// Runs `frames` frames of the engine, and the scene's own cycle, with
    /// no picture.
    pub fn advance(&mut self, frames: u32) -> Result<(), Failed> {
        let mut all = self.stages.clone();
        all.extend(
            self.build
                .stages(Steps::ALL)
                .into_iter()
                .map(|stage| stage.into_iter().map(|_| Groups::Fixed(0)).collect()),
        );
        for _ in 0..frames {
            self.fed()?;
            let groups: Vec<&[Groups]> = all.iter().map(Vec::as_slice).collect();
            self.held.staged(&groups)?;
            self.steps = self.steps.wrapping_add(1);
            self.lived()?;
        }
        Ok(())
    }

    /// Runs `frames` frames of the engine alone, with no picture.
    pub fn warm(&mut self, frames: u32) -> Result<(), Failed> {
        let mut all = self.stages.clone();
        all.extend(
            self.build
                .stages(Steps::ALL)
                .into_iter()
                .map(|stage| stage.into_iter().map(|_| Groups::Fixed(0)).collect()),
        );
        let groups: Vec<&[Groups]> = all.iter().map(Vec::as_slice).collect();
        for _ in 0..frames {
            self.fed()?;
            self.held.staged(&groups)?;
            self.steps = self.steps.wrapping_add(1);
        }
        Ok(())
    }

    /// Writes the scene's feed, the cells it writes before every step.
    fn fed(&mut self) -> Result<(), Failed> {
        if !self.feed.is_empty() && self.steps.is_multiple_of(FEED_EVERY) {
            let feed = std::mem::take(&mut self.feed);
            let done = self.put_runs(&feed);
            self.feed = feed;
            done?;
        }
        Ok(())
    }

    /// Writes `cells`, each a cell index and its word, and wakes them as
    /// [`live::put3`] does: the oct of either phase that holds each cell is
    /// marked awake and hot and is on the list once, added after the octs
    /// it held when it is not there yet. The words go in runs of
    /// neighbouring indices, a write a run, so a feed of many cells costs a
    /// few writes.
    pub fn put_runs(&mut self, cells: &[(u32, u32)]) -> Result<(), Failed> {
        use grit::layout::{AWAKE, HOT, INDIRECT};
        use std::collections::BTreeSet;

        let grid = self.build.grid;
        let mut sorted = cells.to_vec();
        sorted.sort_unstable_by_key(|&(cell, _)| cell);
        sorted.dedup_by_key(|held| held.0);
        if let Some(&(last, _)) = sorted.last()
            && last as usize >= grid.cells()
        {
            return Err("a fed cell lies past the grid".into());
        }
        let grand = self
            .held
            .read(GRAND as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0);
        for (start, words) in runs(sorted.iter().copied()) {
            self.held.write_at(STATE as usize, start as usize, &words)?;
        }
        let woken: BTreeSet<u32> = sorted
            .iter()
            .flat_map(|&(cell, _)| (0..2).map(move |phase| Solid::held(grid, cell, phase).index))
            .collect();
        for (start, words) in runs(woken.iter().map(|&oct| (oct, 2))) {
            self.held.write_at(AWAKE as usize, start as usize, &words)?;
        }
        let by = self.plan.per_block.trailing_zeros();
        let hot: BTreeSet<u32> = woken.iter().map(|&oct| oct >> by).collect();
        for (start, words) in runs(hot.iter().map(|&block| (block, 2))) {
            self.held.write_at(HOT as usize, start as usize, &words)?;
        }
        let mut missing = woken;
        for oct in self.held.read(DENSE as usize, grand as usize)? {
            missing.remove(&oct);
            if missing.is_empty() {
                break;
            }
        }
        if !missing.is_empty() {
            let added: Vec<u32> = missing.into_iter().collect();
            let count = grand.saturating_add(u32::try_from(added.len()).unwrap_or(u32::MAX));
            self.held.write_at(DENSE as usize, grand as usize, &added)?;
            self.held.write(GRAND as usize, &[count])?;
            self.held.write(
                INDIRECT as usize,
                &[count.div_ceil(self.plan.subgroup), 1, 1],
            )?;
        }
        Ok(())
    }

    /// The cell a pixel sees, and the face it sees it by.
    pub fn under(&mut self, (x, y): (u32, u32)) -> Result<Option<(u32, u32)>, Failed> {
        let layout = self.build.layout();
        let at =
            layout.hits() + (y as usize * self.build.screen.wide as usize + x as usize) * HIT_WORDS;
        let words = self.held.read(PIXELS as usize, at + HIT_WORDS)?;
        let cell = words.get(at).copied().unwrap_or(MISS);
        let info = words.get(at + 1).copied().unwrap_or(0);
        Ok((cell != MISS).then_some((cell, info & 7)))
    }

    /// Writes a ball of the tool's word around the cell in front of `face`
    /// of `cell`, through [`live::put3`], and gives the cells it wrote.
    pub fn stroke(
        &mut self,
        (cell, face): (u32, u32),
        tool: Tool,
    ) -> Result<Vec<(u32, u32)>, Failed> {
        let writes = stroked(self.build.grid, (cell, face), tool);
        live::put3(&mut self.held, self.build.grid, &self.plan, &writes)?;
        Ok(writes)
    }

    /// The octs the engine steps next frame: its list, read back.
    pub fn listed(&mut self) -> Result<Vec<u32>, Failed> {
        let grand = self
            .held
            .read(GRAND as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0);
        let dump = self.plan.dump() as usize;
        let dense = self.held.read(DENSE as usize, dump + 1)?;
        let claimed = dense.get(dump).copied().unwrap_or(0);
        let total = (grand + claimed) as usize;
        let list = self.held.read(DENSE as usize, total.max(dump + 1))?;
        Ok(list.get(..total).unwrap_or(&[]).to_vec())
    }

    /// How many octs the engine stepped in the last frame.
    pub fn active(&mut self) -> Result<u32, Failed> {
        let layout = self.build.layout();
        let frame = self
            .held
            .read(PARAMS as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0);
        let words = self.held.read(PIXELS as usize, layout.changed())?;
        let stamp = frame.wrapping_add(1);
        Ok(words
            .get(layout.active()..layout.changed())
            .unwrap_or(&[])
            .iter()
            .filter(|&&held| held == stamp)
            .count() as u32)
    }

    /// The octs whose stamp shows the engine stepped them in the frame the
    /// first word of `PARAMS` holds.
    #[allow(dead_code)]
    pub fn marked(&mut self) -> Result<Vec<u32>, Failed> {
        let layout = self.build.layout();
        let frame = self
            .held
            .read(PARAMS as usize, 1)?
            .first()
            .copied()
            .unwrap_or(0);
        let words = self.held.read(PIXELS as usize, layout.changed())?;
        let stamp = frame.wrapping_add(1);
        Ok((0_u32..)
            .zip(words.get(layout.active()..layout.changed()).unwrap_or(&[]))
            .filter(|&(_, &held)| held == stamp)
            .map(|(oct, _)| oct)
            .collect())
    }

    #[allow(dead_code)]
    pub fn state(&mut self) -> Result<Vec<u32>, Failed> {
        Ok(self.held.read(STATE as usize, self.build.grid.cells())?)
    }

    /// The stage times of the last frame, by stage, with the engine's summed
    /// into the first.
    pub fn times(&self, moving: bool, steps: Steps) -> Result<Vec<Duration>, Failed> {
        let spans = self.held.stage_times().to_vec();
        let all = self.stages(moving, steps);
        let ran: Vec<usize> = (0..all.len())
            .filter(|&at| {
                all.get(at)
                    .is_some_and(|stage| stage.iter().any(|groups| *groups != Groups::Fixed(0)))
            })
            .collect();
        if spans.len() != ran.len() {
            return Err(format!("{} stage times, {} stages ran", spans.len(), ran.len()).into());
        }
        let time_of = |stage: usize| {
            ran.iter()
                .position(|&at| at == stage)
                .and_then(|at| spans.get(at).copied())
                .unwrap_or_default()
        };
        let mut out = vec![(0..self.engine_stages).map(time_of).sum::<Duration>()];
        out.push(time_of(self.watcher));
        for at in 0..stage::COUNT {
            out.push(time_of(self.engine_stages + at));
        }
        Ok(out)
    }
}

/// `pairs`, each an index and its word in rising order, gathered into runs
/// of neighbouring indices: each run's first index and its words.
pub fn runs(pairs: impl IntoIterator<Item = (u32, u32)>) -> Vec<(u32, Vec<u32>)> {
    let mut out: Vec<(u32, Vec<u32>)> = Vec::new();
    for (at, word) in pairs {
        if let Some((start, words)) = out.last_mut()
            && u64::from(*start) + words.len() as u64 == u64::from(at)
        {
            words.push(word);
            continue;
        }
        out.push((at, vec![word]));
    }
    out
}

/// The view of every camera, in radians.
pub const VIEW: f32 = 0.62;

/// The cells a stroke of `tool` writes at the cell in front of `face` of
/// `cell`: a ball of its word, in cell order.
pub fn stroked(grid: Grid, (cell, face): (u32, u32), tool: Tool) -> Vec<(u32, u32)> {
    let mut point = Solid::point(grid, cell).map(|value| value as i32);
    if face != INSIDE {
        let axis = (face / 2) as usize;
        let sign = if face % 2 == 1 { 1 } else { -1 };
        if let Some(slot) = point.get_mut(axis) {
            *slot += sign;
        }
    }
    let r = tool.radius.max(0);
    let mut cells = Vec::new();
    for dz in -r..=r {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy + dz * dz > r * r + r {
                    continue;
                }
                let at = [point[0] + dx, point[1] + dy, point[2] + dz];
                let sides = [grid.wide, grid.deep, grid.tall];
                if at
                    .iter()
                    .zip(sides)
                    .all(|(&value, side)| value >= 0 && (value as u32) < side)
                {
                    let at = at.map(|value| value as u32);
                    cells.push((Solid::cell(grid, at), tool.word));
                }
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
}
