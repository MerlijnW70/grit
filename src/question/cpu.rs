use crate::ops::Cpu;

use super::{Answer, Map, Measure, Reduce, Region, Seen};

#[derive(Debug, Clone, Copy)]
pub struct World<'held> {
    pub size: [u32; 3],
    pub cells: &'held [u32],
}

impl World<'_> {
    #[must_use]
    pub fn cell(&self, [x, y, z]: [u32; 3]) -> u32 {
        let [wide, deep, _] = self.size.map(|side| side as usize);
        let at = (z as usize * deep + y as usize) * wide + x as usize;
        self.cells.get(at).copied().unwrap_or(0)
    }
}

#[must_use]
pub fn mapped<M: Map>(map: &M, word: u32, at: [u32; 3]) -> u32 {
    let planes = [word];
    map.map(
        &mut Cpu,
        Seen {
            planes: &planes,
            at,
        },
    )
}

fn low(total: u128) -> u64 {
    u64::try_from(total & u128::from(u64::MAX)).unwrap_or(u64::MAX)
}

#[must_use]
pub fn answer<M: Map>(
    map: &M,
    reduce: Reduce,
    world: World<'_>,
    region: Region,
    target: &[u32],
) -> Answer {
    let [from_x, from_y, from_z] = region.from;
    let [to_x, to_y, to_z] = region.to;
    let [wide, deep, _] = region.sides().map(u128::from);
    let bins = match reduce {
        Reduce::Histogram { bins } => bins,
        _ => 0,
    };
    let mut total = 0_u128;
    let mut sums = [0_u128; 3];
    let mut least = u32::MAX;
    let mut most = 0_u32;
    let mut histogram = vec![0_u128; bins as usize + 1];
    for z in from_z..to_z {
        for y in from_y..to_y {
            for x in from_x..to_x {
                let at = [x, y, z];
                let value = mapped(map, world.cell(at), at);
                match reduce {
                    Reduce::Count => total += u128::from(value != 0),
                    Reduce::Sum => total += u128::from(value),
                    Reduce::Min => least = least.min(value),
                    Reduce::Max => most = most.max(value),
                    Reduce::Histogram { .. } => {
                        let bin = value.min(bins) as usize;
                        if let Some(slot) = histogram.get_mut(bin) {
                            *slot += 1;
                        }
                    }
                    Reduce::Moments => {
                        if value != 0 {
                            total += 1;
                            for (sum, &along) in sums.iter_mut().zip(&at) {
                                *sum += u128::from(along);
                            }
                        }
                    }
                    Reduce::Differ(measure) => {
                        let index = (u128::from(z - from_z) * deep + u128::from(y - from_y)) * wide
                            + u128::from(x - from_x);
                        let aimed = usize::try_from(index)
                            .ok()
                            .and_then(|index| target.get(index))
                            .copied()
                            .unwrap_or(0);
                        total += match measure {
                            Measure::Count => u128::from(value != aimed),
                            Measure::Sum => u128::from(value.abs_diff(aimed)),
                        };
                    }
                }
            }
        }
    }
    match reduce {
        Reduce::Count => Answer::Count(low(total)),
        Reduce::Sum => Answer::Sum(low(total)),
        Reduce::Differ(_) => Answer::Differ(low(total)),
        Reduce::Min => Answer::Min(least),
        Reduce::Max => Answer::Max(most),
        Reduce::Histogram { .. } => {
            let past = histogram.pop().map_or(0, low);
            Answer::Histogram {
                bins: histogram.into_iter().map(low).collect(),
                past,
            }
        }
        Reduce::Moments => Answer::Moments {
            count: low(total),
            sums: sums.map(low),
        },
    }
}
