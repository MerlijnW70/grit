use super::adjoint::{self, Aim, Descender, Descent, Fault, Next, Posed, Reversible, Walked};
use super::dag::Dag;
use super::{Law, Refusal};
use crate::halo::{self, BOX};
use crate::rule3::{CORNERS, Declared, Oct};
use crate::sand3::{Grid3, lattice, reference};

#[derive(Clone, Debug)]
pub struct Kept {
    law: Law,
}

impl Kept {
    #[must_use]
    pub const fn new(law: Law) -> Self {
        Self { law }
    }

    #[must_use]
    pub const fn forward(&self) -> &Dag {
        self.law.dag()
    }

    fn layer(&self, odd: bool, layer: usize) -> usize {
        if !odd {
            return layer;
        }
        for &[looked, target] in self.law.pairs() {
            let looked = usize::try_from(looked).unwrap_or(usize::MAX);
            let target = usize::try_from(target).unwrap_or(usize::MAX);
            if layer == looked {
                return target;
            }
            if layer == target {
                return looked;
            }
        }
        layer
    }

    pub fn walk(
        &self,
        grid: Grid3,
        start: &[u32],
        params: &[u32],
        first: u32,
        count: u32,
        aim: &Aim<'_>,
    ) -> Result<Walked, Fault> {
        let dag = self.law.dag();
        let stride = grid.cells();
        let layers = adjoint::spread(dag) / CORNERS;
        if start.len() != stride * layers {
            return Err(Fault::Cells {
                given: start.len(),
                wanted: stride * layers,
            });
        }
        if params.len() != dag.params() {
            return Err(Fault::Params {
                given: params.len(),
                wanted: dag.params(),
            });
        }
        aim.fits(grid)?;
        let mut held = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
        let mut cells = start.to_vec();
        for done in 0..count {
            let at = first.wrapping_add(done);
            let next = reference(&self.law, grid, at, &cells, params);
            held.push(std::mem::replace(&mut cells, next));
        }
        let objective = aim.measured(grid, &cells);
        let mut weights = aim.weights(grid, &cells);
        let mut told = vec![0_i128; params.len()];
        for (done, before) in held.iter().enumerate().rev() {
            let at = first.wrapping_add(u32::try_from(done).unwrap_or(u32::MAX));
            weights = self.back(grid, at, before, params, &weights, &mut told)?;
        }
        Ok(Walked {
            ended: cells,
            objective,
            told,
            slope: weights,
        })
    }

    fn back(
        &self,
        grid: Grid3,
        at: u32,
        before: &[u32],
        params: &[u32],
        weights: &[i128],
        told: &mut [i128],
    ) -> Result<Vec<i128>, Fault> {
        let dag = self.law.dag();
        let stride = grid.cells();
        let layers = adjoint::spread(dag) / CORNERS;
        let looks = dag.halo();
        let odd = !self.law.pairs().is_empty() && at & 1 == 1;
        let moved: Vec<usize> = (0..layers).map(|layer| self.layer(odd, layer)).collect();
        let source = |layer: usize| moved.get(layer).copied().unwrap_or(layer);
        let mut carried = vec![0_i128; weights.len()];
        for oct in 0..grid.octs() {
            let places = grid.corners(at, oct);
            let spots: Vec<usize> = (0..layers)
                .flat_map(|layer| places.map(|place| source(layer) * stride + place))
                .collect();
            let word_at = |spot: usize| before.get(spot).copied().unwrap_or(0);
            let words = places.map(|place| word_at(source(0) * stride + place));
            let upper: Vec<[u32; CORNERS]> = (1..layers)
                .map(|layer| places.map(|place| word_at(source(layer) * stride + place)))
                .collect();
            let boxes: Vec<[u32; BOX]> = (0..layers)
                .map(|layer| {
                    if !looks.looked(layer) {
                        return [0; BOX];
                    }
                    let from = source(layer) * stride;
                    before
                        .get(from..from + stride)
                        .and_then(|plane| halo::boxed(grid, at, oct, plane))
                        .unwrap_or([0; BOX])
                })
                .collect();
            let given: Vec<i128> = spots
                .iter()
                .map(|&spot| weights.get(spot).copied().unwrap_or(0))
                .collect();
            let mut looked = vec![[0_i128; BOX]; layers];
            let back = adjoint::carried_in(
                dag,
                Oct {
                    words,
                    frame: at,
                    at: lattice(grid, oct),
                    params,
                },
                &upper,
                &boxes,
                &given,
                told,
                &mut looked,
            )?;
            for (spot, value) in spots.into_iter().zip(back) {
                adjoint::added(&mut carried, u32::try_from(spot).unwrap_or(u32::MAX), value)?;
            }
            for (layer, boxed) in looked.iter().enumerate() {
                for (cell, &value) in boxed.iter().enumerate() {
                    if value != 0 {
                        let spot = source(layer) * stride + halo::place(grid, at, oct, cell);
                        adjoint::added(
                            &mut carried,
                            u32::try_from(spot).unwrap_or(u32::MAX),
                            value,
                        )?;
                    }
                }
            }
        }
        Ok(carried)
    }

    pub fn descend(&self, world: &Posed<'_>, from: &[u32], most: u32) -> Result<Descent, Fault> {
        let walk = |params: &[u32]| {
            self.walk(
                world.grid,
                world.start,
                params,
                world.first,
                world.frames,
                &world.aim,
            )
        };
        let mut descender = Descender::new(from, &walk(from)?, most);
        while let Next::Walk(params) = descender.next() {
            descender.walked(&walk(&params)?);
        }
        Ok(descender.descent())
    }
}

#[derive(Clone, Debug)]
pub enum Walker {
    Back(Reversible),
    Kept(Kept),
}

impl Walker {
    pub fn of(law: &Law) -> Result<Self, Refusal> {
        match Reversible::of(law) {
            Ok(reversible) => Ok(Self::Back(reversible)),
            Err(_) if !law.pairs().is_empty() && !law.reversible() => {
                Ok(Self::Kept(Kept::new(law.clone())))
            }
            Err(why) => Err(why),
        }
    }

    #[must_use]
    pub const fn forward(&self) -> &Dag {
        match self {
            Self::Back(reversible) => reversible.forward(),
            Self::Kept(kept) => kept.forward(),
        }
    }

    #[must_use]
    pub fn back_nodes(&self) -> usize {
        match self {
            Self::Back(reversible) => reversible.inverse().len(),
            Self::Kept(kept) => kept.forward().len(),
        }
    }

    pub fn walk(
        &self,
        grid: Grid3,
        start: &[u32],
        params: &[u32],
        first: u32,
        count: u32,
        aim: &Aim<'_>,
    ) -> Result<Walked, Fault> {
        match self {
            Self::Back(reversible) => reversible.walk(grid, start, params, first, count, aim),
            Self::Kept(kept) => kept.walk(grid, start, params, first, count, aim),
        }
    }
}
