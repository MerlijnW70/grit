use grit::geometry::Solid;
use grit::rule3::nature::{FIRE, WOOD_LIFE, alive};
use grit::sand::Grid;

use super::valley::mixed;

/// Where a scene's own cycle stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Calm,
    Burning,
    After,
}

/// What the cycle asks of the session after a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Nothing,
    Light(Vec<(u32, u32)>),
    Restart,
}

/// The heat of the fire the cycle lights.
pub const KINDLING: u32 = 900;

/// A world that lives on its own: a calm, then a fire lit at one of
/// `sites`, a different one each round, then, once no fire or smoke is
/// left, a while to see the ash, then the world as it started. A fire that
/// dies out within a calm of being lit did not take, and another is
/// lit at the next site a short while later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    pub grid: Grid,
    pub sites: Vec<[u32; 3]>,
    pub calm: u32,
    pub after: u32,
    pub round: u32,
    pub phase: Phase,
    pub since: u32,
    pub lit: Option<[u32; 3]>,
}

impl Cycle {
    pub fn new(grid: Grid, sites: Vec<[u32; 3]>, calm: u32, after: u32) -> Self {
        Self {
            grid,
            sites,
            calm,
            after,
            round: 0,
            phase: Phase::Calm,
            since: 0,
            lit: None,
        }
    }

    /// The site of the fire of `round`.
    pub fn site(&self, round: u32) -> Option<[u32; 3]> {
        let count = u32::try_from(self.sites.len()).ok()?;
        if count == 0 {
            return None;
        }
        self.sites
            .get((mixed(round.wrapping_mul(0x9E37_79B9) ^ 0xF12E) % count) as usize)
            .copied()
    }

    /// The cells that light the fire at `site`: a small ball of fire.
    pub fn kindling(&self, site: [u32; 3]) -> Vec<(u32, u32)> {
        let sides = [self.grid.wide, self.grid.deep, self.grid.tall];
        let mut cells = Vec::new();
        for dz in -2_i64..=2 {
            for dy in -2_i64..=2 {
                for dx in -2_i64..=2 {
                    if dx * dx + dy * dy + dz * dz > 4 {
                        continue;
                    }
                    let at = [
                        i64::from(site[0]) + dx,
                        i64::from(site[1]) + dy,
                        i64::from(site[2]) + dz,
                    ];
                    if at
                        .iter()
                        .zip(sides)
                        .all(|(&value, side)| value >= 0 && value < i64::from(side))
                    {
                        let at = at.map(|value| value as u32);
                        cells.push((Solid::cell(self.grid, at), alive(FIRE, KINDLING, WOOD_LIFE)));
                    }
                }
            }
        }
        cells.sort_unstable();
        cells
    }

    /// One engine step later, with the world's fire and smoke counted when
    /// `counted` holds them.
    pub fn stepped(&mut self, counted: Option<(u32, u32)>) -> Act {
        self.since = self.since.saturating_add(1);
        match self.phase {
            Phase::Calm => {
                if self.since < self.calm {
                    return Act::Nothing;
                }
                self.phase = Phase::Burning;
                self.since = 0;
                self.lit = self.site(self.round);
                self.lit
                    .map_or(Act::Nothing, |site| Act::Light(self.kindling(site)))
            }
            Phase::Burning => {
                if counted == Some((0, 0)) && self.since > 2 {
                    if self.since < self.calm {
                        self.phase = Phase::Calm;
                        self.since = self.calm - self.calm / 8;
                        self.round = self.round.wrapping_add(1);
                    } else {
                        self.phase = Phase::After;
                        self.since = 0;
                    }
                }
                Act::Nothing
            }
            Phase::After => {
                if self.since < self.after {
                    return Act::Nothing;
                }
                self.phase = Phase::Calm;
                self.since = 0;
                self.round = self.round.wrapping_add(1);
                self.lit = None;
                Act::Restart
            }
        }
    }
}
