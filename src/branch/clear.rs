use crate::border::{self, Box3, CAP, COUNT, HEAD, NEEDS, WORDS};
use crate::bricks::{ENTRY, VALUE};
use crate::geometry::Solid;
use crate::live;
use crate::resident::Groups;

use super::{Failed, Refused, World};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cleared {
    pub words: u64,
    pub units: u64,
    pub lanes: u64,
    pub passes: u32,
}

struct Pass {
    window: Box3,
    start: [u32; 3],
    span: [u32; 3],
    planes: u32,
    keep: u32,
}

impl World<'_> {
    pub fn clear(
        &mut self,
        from: [u32; 3],
        to: [u32; 3],
        planes: u32,
        bits: u32,
    ) -> Result<Cleared, Failed> {
        let size = self.size();
        for at in [from, to] {
            if at.iter().zip(size).any(|(one, side)| *one >= side) {
                return Err(Box::new(Refused::Outside { at, size }));
            }
        }
        let every = (1_u64 << self.planes()) - 1;
        if planes == 0 || u64::from(planes) > every {
            return Err(Box::new(Refused::Malformed {
                what: format!(
                    "the planes {planes:#b} name none or one past the world's {}",
                    self.planes()
                ),
            }));
        }
        let window = Box3::spanning(from, to, size);
        let unit = self.pool.map_or([2, 2, 2], |bricks| bricks.sides());
        let (start, span) = window.units(size, unit);
        let pass = Pass {
            window,
            start,
            span,
            planes,
            keep: !bits,
        };
        self.seen = None;
        let mut out = Cleared::default();
        let mut changed = Vec::new();
        if self.pool.is_some() {
            let mut surveyed = Vec::new();
            let needs = self.cleared(&pass, false, &mut surveyed, &mut out)?;
            out.words += self.slotted(&pass, &needs, bits)?;
        }
        let late = self.cleared(&pass, true, &mut changed, &mut out)?;
        if !late.is_empty() {
            return Err(Box::new(Refused::Malformed {
                what: format!(
                    "{} bricks still had no slot after the clear took one for each",
                    late.len()
                ),
            }));
        }
        changed.sort_unstable();
        changed.dedup();
        out.units = changed.len() as u64;
        match self.pool {
            Some(bricks) => bricks.wake_bricks(&mut self.live, &changed)?,
            None => {
                let octs = self.woken_octs(&changed);
                live::woke3(&mut self.live, self.grid, &self.plan, &octs, Vec::new())?;
            }
        }
        Ok(out)
    }

    fn cleared(
        &mut self,
        pass: &Pass,
        acting: bool,
        changed: &mut Vec<u32>,
        out: &mut Cleared,
    ) -> Result<Vec<u32>, Failed> {
        let total: u64 = pass.span.iter().map(|side| u64::from(*side)).product();
        let total = u32::try_from(total)?;
        let binding = self.clearer.binding as usize;
        let subgroup = self.plan.subgroup;
        let quiet = vec![Groups::Fixed(0); self.clearer.pass];
        let mut needs = Vec::new();
        let mut base = 0_u32;
        while base < total {
            let units = CAP.min(total - base);
            let mut words = pass
                .window
                .words(pass.start, pass.span, pass.planes, pass.keep);
            border::chunk(&mut words, base, units, acting);
            self.live.write(binding, &words)?;
            let run = [Groups::Fixed(units.div_ceil(subgroup))];
            let got = self
                .live
                .run_reading(&[&quiet, &run], binding, HEAD as usize)?;
            let count = got.get(COUNT as usize).copied().unwrap_or_default();
            out.words += u64::from(got.get(WORDS as usize).copied().unwrap_or_default());
            out.lanes += u64::from(units);
            out.passes += 1;
            if count > 0 {
                let listed = self
                    .live
                    .read_at(binding, HEAD as usize, count.min(CAP) as usize)?;
                for unit in listed {
                    if unit & NEEDS == 0 {
                        changed.push(unit);
                    } else {
                        needs.push(unit & !NEEDS);
                    }
                }
            }
            base += units;
        }
        Ok(needs)
    }

    fn slotted(&mut self, pass: &Pass, needs: &[u32], bits: u32) -> Result<u64, Failed> {
        let Some(bricks) = self.pool else {
            return Ok(0);
        };
        if needs.is_empty() {
            return Ok(0);
        }
        let used = bricks.pool_use(&mut self.live)?;
        if (used.free as usize) < needs.len() {
            return Err(Box::new(crate::host::Error::PoolFull {
                slots: bricks.capacity(),
            }));
        }
        let size = self.size();
        let sides = bricks.sides();
        let lattice = bricks.lattice();
        let around = self
            .law()
            .is_some_and(|law| !crate::rule3::Declared::looks(law).is_empty());
        let table = bricks.table() as usize;
        let mut asked = Vec::new();
        for &brick in needs {
            for plane in 0..bricks.planes() {
                let at = if plane == 0 {
                    brick as usize * ENTRY + VALUE
                } else {
                    bricks.value_at(plane, brick)
                };
                asked.push((table, at, 1));
            }
        }
        let values: Vec<u32> = self
            .live
            .read_runs(&asked)?
            .into_iter()
            .map(|one| one.first().copied().unwrap_or_default())
            .collect();
        let mut by_plane: Vec<Vec<([u32; 3], u32)>> = vec![Vec::new(); bricks.planes()];
        for (index, &brick) in needs.iter().enumerate() {
            let point = [
                brick % lattice[0],
                brick / lattice[0] % lattice[1],
                brick / lattice[0] / lattice[1],
            ];
            let origin = [
                point[0] * sides[0],
                point[1] * sides[1],
                point[2] * sides[2],
            ];
            let inside = (0..sides[2])
                .flat_map(|z| {
                    (0..sides[1]).flat_map(move |y| (0..sides[0]).map(move |x| [x, y, z]))
                })
                .map(|local| {
                    [
                        origin[0] + local[0],
                        origin[1] + local[1],
                        origin[2] + local[2],
                    ]
                })
                .find(|cell| pass.window.holds(size, *cell));
            let picked = (0..bricks.planes()).find(|&plane| {
                (pass.planes >> plane) & 1 == 1
                    && values
                        .get(index * bricks.planes() + plane)
                        .is_some_and(|value| value & bits != 0)
            });
            if let (Some(cell), Some(plane)) = (inside, picked) {
                let value = values
                    .get(index * bricks.planes() + plane)
                    .copied()
                    .unwrap_or_default();
                if let Some(list) = by_plane.get_mut(plane) {
                    list.push((cell, value & pass.keep));
                }
            }
        }
        let mut written = 0_u64;
        for (plane, cells) in by_plane.iter().enumerate() {
            if !cells.is_empty() {
                bricks.put_plane_in(&mut self.live, plane, cells, around)?;
                written += cells.len() as u64;
            }
        }
        Ok(written)
    }

    fn woken_octs(&self, changed: &[u32]) -> Vec<u32> {
        let grid = self.grid;
        let octs = Solid::blocks(grid);
        let bits = octs.map(u32::trailing_zeros);
        let mut woken = Vec::with_capacity(changed.len() * 54);
        for &oct in changed {
            let point = [
                oct & (octs[0] - 1),
                (oct >> bits[0]) & (octs[1] - 1),
                oct >> (bits[0] + bits[1]),
            ];
            let earlier = [
                (point[0] + octs[0] - 1) & (octs[0] - 1),
                (point[1] + octs[1] - 1) & (octs[1] - 1),
                (point[2] + octs[2] - 1) & (octs[2] - 1),
            ];
            let back = earlier[0] | (earlier[1] << bits[0]) | (earlier[2] << (bits[0] + bits[1]));
            woken.extend(Solid::woken(grid, oct));
            woken.extend(Solid::woken(grid, back));
        }
        woken.sort_unstable();
        woken.dedup();
        woken
    }
}
