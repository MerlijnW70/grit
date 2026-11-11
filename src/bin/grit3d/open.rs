use grit::bricks::{Bricks, Content};
use grit::rule3::nature::{
    EARTH, ICE, LEAVES, ROCK, SAND, SNOW, THAW, WATER, WOOD, air, alive, cell, open_air,
};
use grit::sand::Grid;

use super::valley::{AMBIENT, mixed};

pub const SEED: u32 = 0x0B1E_55ED;

pub const SEA: u32 = 34;

const DEEPEST: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Skin {
    Rock,
    Earth,
    Sand,
    Snow,
    Ice,
    Bed,
}

pub struct Land {
    grid: Grid,
    side: usize,
    ground: Vec<u8>,
    skin: Vec<u8>,
    depth: Vec<u8>,
    wet: Vec<u8>,
    trunk: Vec<u8>,
    low: Vec<u8>,
    high: Vec<u8>,
    seed: u32,
}

fn hashed(x: u32, z: u32, salt: u32, seed: u32) -> u32 {
    mixed(mixed(mixed(x ^ seed.rotate_left(11)) ^ z.wrapping_mul(0x9E37_79B9)) ^ salt)
}

fn unit(key: u32) -> f32 {
    (key & 0x00FF_FFFF) as f32 / 16_777_215.0
}

fn value(x: f32, z: f32, period: u32, seed: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (u, v) = (x - fx, z - fz);
    let ease = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (su, sv) = (ease(u), ease(v));
    let period = period.max(1) as i64;
    let at = |i: i64, j: i64| {
        unit(hashed(
            i.rem_euclid(period) as u32,
            j.rem_euclid(period) as u32,
            0x51,
            seed,
        ))
    };
    let (i, j) = (fx as i64, fz as i64);
    let low = at(i, j) + (at(i + 1, j) - at(i, j)) * su;
    let high = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * su;
    low + (high - low) * sv
}

fn fbm(u: f32, v: f32, period: u32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut weight, mut total, mut period) = (0.0, 0.5, 0.0, period);
    for octave in 0..octaves {
        let size = period as f32;
        sum += weight
            * value(
                u * size,
                v * size,
                period,
                seed.wrapping_add(octave * 0x3C6E_F372),
            );
        total += weight;
        weight *= 0.5;
        period *= 2;
    }
    sum / total
}

fn ridged(u: f32, v: f32, period: u32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut weight, mut total, mut period, mut previous) = (0.0, 0.5, 0.0, period, 1.0);
    for octave in 0..octaves {
        let size = period as f32;
        let n = value(
            u * size,
            v * size,
            period,
            seed.wrapping_add(octave * 0x2545_F491),
        );
        let crest = 1.0 - (2.0 * n - 1.0).abs();
        let crest = crest * crest * (0.35 + 0.65 * previous);
        previous = crest;
        sum += weight * crest;
        total += weight;
        weight *= 0.5;
        period *= 2;
    }
    sum / total
}

fn smooth(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, by: f32) -> f32 {
    a + (b - a) * by
}

struct Shape {
    height: f32,
    river: f32,
    bed: f32,
    alpine: f32,
}

fn shaped(u: f32, v: f32, tall: f32, seed: u32) -> Shape {
    let warp_u = u + 0.03 * (fbm(u, v, 4, 3, seed ^ 0x11) - 0.5);
    let warp_v = v + 0.03 * (fbm(u, v, 4, 3, seed ^ 0x12) - 0.5);
    let continent = fbm(warp_u, warp_v, 3, 3, seed ^ 0x21);
    let range = smooth(0.42, 0.68, fbm(warp_u, warp_v, 2, 3, seed ^ 0x22));
    let ridges = ridged(warp_u, warp_v, 10, 6, seed ^ 0x23);
    let hills = fbm(u, v, 28, 5, seed ^ 0x24);
    let sea = SEA as f32;
    let lowland = sea - 10.0 + 40.0 * smooth(0.25, 0.65, continent) + 22.0 * hills;
    let peaks = range * (tall * 0.62 * ridges.powf(1.35) + 26.0 * hills);
    let mut height = lowland + peaks;
    let shelf = smooth(0.55, 0.75, fbm(u, v, 3, 2, seed ^ 0x25)) * (1.0 - range);
    if shelf > 0.0 {
        let step = 14.0;
        let level = (height / step).floor();
        let part = height / step - level;
        let terraced = step * (level + smooth(0.78, 0.95, part));
        height = mix(height, terraced, shelf);
    }
    let course = fbm(warp_u * 0.9, warp_v * 0.9, 3, 4, seed ^ 0x31) - 0.5;
    let far = course.abs();
    let wide = 0.006 + 0.05 * range;
    let carve = 1.0 - smooth(0.0, wide, far);
    let floor = sea + 2.0 + 0.45 * (height - sea).max(0.0) * (0.35 + 0.4 * range);
    let floor = floor.min(height);
    let u_shape = carve.powf(0.45 + 0.6 * (1.0 - range));
    height = mix(height, floor, u_shape);
    let river = 1.0 - smooth(0.0, 0.0035 + 0.002 * range, far);
    Shape {
        height,
        river,
        bed: floor,
        alpine: range,
    }
}

fn rows<F>(side: usize, out: &mut [Vec<u8>], fill: F)
where
    F: Fn(usize, &mut [&mut [u8]]) + Sync,
{
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let per = side.div_ceil(threads);
    let mut parts: Vec<Vec<&mut [u8]>> = (0..threads).map(|_| Vec::new()).collect();
    for field in out.iter_mut() {
        for (part, chunk) in parts.iter_mut().zip(field.chunks_mut(per * side)) {
            part.push(chunk);
        }
    }
    std::thread::scope(|scope| {
        for (at, mut part) in parts.into_iter().enumerate() {
            let fill = &fill;
            scope.spawn(move || {
                let first = at * per;
                let count = part.first().map_or(0, |held| held.len() / side.max(1));
                for row in 0..count {
                    let mut line: Vec<&mut [u8]> = part
                        .iter_mut()
                        .filter_map(|held| held.get_mut(row * side..(row + 1) * side))
                        .collect();
                    fill(first + row, &mut line);
                }
            });
        }
    });
}

impl Land {
    #[must_use]
    pub fn new(grid: Grid, seed: u32) -> Self {
        let side = grid.wide.min(grid.tall) as usize;
        let tall = grid.deep as f32;
        let top = grid.deep.saturating_sub(12) as f32;
        let s = side as f32;
        let mut fields = vec![vec![0_u8; side * side]; 3];
        rows(side, &mut fields, |z, line| {
            for x in 0..side {
                let (u, v) = ((x as f32 + 0.5) / s, (z as f32 + 0.5) / s);
                let shape = shaped(u, v, tall, seed);
                let height = shape.height.clamp(6.0, top);
                let water = if height < SEA as f32 {
                    SEA as f32
                } else if shape.river > 0.0 && shape.bed > SEA as f32 {
                    shape.bed.min(height) + 2.0
                } else {
                    0.0
                };
                let channel = if shape.river > 0.0 && water > 0.0 {
                    (height - 2.0).max(6.0)
                } else {
                    height
                };
                let put = |line: &mut [&mut [u8]], at: usize, value: f32| {
                    if let Some(slot) = line.get_mut(at).and_then(|row| row.get_mut(x)) {
                        *slot = value.clamp(0.0, 255.0) as u8;
                    }
                };
                put(line, 0, channel);
                put(line, 1, water.min(top));
                put(line, 2, shape.alpine * 255.0);
            }
        });
        let alpine = fields.pop().unwrap_or_default();
        let wet = fields.pop().unwrap_or_default();
        let ground = fields.pop().unwrap_or_default();
        let mut land = Self {
            grid,
            side,
            skin: vec![0; side * side],
            depth: vec![0; side * side],
            trunk: vec![0; side * side],
            low: vec![0; side * side],
            high: vec![0; side * side],
            ground,
            wet,
            seed,
        };
        land.dressed(&alpine);
        land.planted(&alpine);
        land
    }

    fn slot(&self, x: i64, z: i64) -> usize {
        let side = self.side as i64;
        (z.rem_euclid(side) * side + x.rem_euclid(side)) as usize
    }

    fn ground_at(&self, x: i64, z: i64) -> u32 {
        u32::from(self.ground.get(self.slot(x, z)).copied().unwrap_or(0))
    }

    fn drop(&self, x: i64, z: i64) -> u32 {
        let here = self.ground_at(x, z);
        let mut most = 0;
        for dz in -1..=1 {
            for dx in -1..=1 {
                most = most.max(here.saturating_sub(self.ground_at(x + dx, z + dz)));
            }
        }
        most
    }

    fn dressed(&mut self, alpine: &[u8]) {
        let side = self.side;
        let s = side as f32;
        let tall = self.grid.deep as f32;
        let seed = self.seed;
        let mut skin = vec![0_u8; side * side];
        let mut depth = vec![0_u8; side * side];
        for z in 0..side {
            for x in 0..side {
                let at = z * side + x;
                let (xi, zi) = (x as i64, z as i64);
                let h = self.ground_at(xi, zi);
                let fall = self.drop(xi, zi);
                let wet = u32::from(self.wet.get(at).copied().unwrap_or(0));
                let range = f32::from(alpine.get(at).copied().unwrap_or(0)) / 255.0;
                let (u, v) = ((x as f32 + 0.5) / s, (z as f32 + 0.5) / s);
                let line =
                    tall * (0.5 - 0.06 * range) + 14.0 * (fbm(u, v, 40, 3, seed ^ 0x41) - 0.5);
                let tree_line = line - 34.0;
                let around = [(-3, 0), (3, 0), (0, -3), (0, 3)]
                    .iter()
                    .map(|&(dx, dz)| self.ground_at(xi + dx, zi + dz))
                    .sum::<u32>() as f32
                    / 4.0;
                let hollow = (around - h as f32).max(0.0);
                let (kind, deep) = if wet > h && h + 1 >= SEA && wet > SEA {
                    (Skin::Bed, 2)
                } else if wet > h {
                    (if fall <= 1 { Skin::Sand } else { Skin::Rock }, 3)
                } else if (h as f32) > line && fall <= 1 {
                    if hollow > 3.0 && range > 0.5 {
                        (Skin::Ice, 2 + (hollow * 0.6).min(5.0) as u32)
                    } else {
                        (Skin::Snow, 1 + (hollow * 0.5).min(3.0) as u32)
                    }
                } else if fall <= 1 && h <= SEA + 2 {
                    (Skin::Sand, 3)
                } else if fall <= 1 && (h as f32) < tree_line + 20.0 {
                    (Skin::Earth, 2 + (hollow * 0.4).min(3.0) as u32)
                } else {
                    (Skin::Rock, 0)
                };
                if let Some(slot) = skin.get_mut(at) {
                    *slot = kind as u8;
                }
                if let Some(slot) = depth.get_mut(at) {
                    *slot = deep as u8;
                }
            }
        }
        self.skin = skin;
        self.depth = depth;
    }

    fn planted(&mut self, alpine: &[u8]) {
        let side = self.side;
        let s = side as f32;
        let tall = self.grid.deep as f32;
        let seed = self.seed;
        let spacing = 6_usize;
        for cz in 0..side / spacing {
            for cx in 0..side / spacing {
                let key = hashed(cx as u32, cz as u32, 0x71, seed);
                let x = cx * spacing + (key % spacing as u32) as usize;
                let z = cz * spacing + ((key >> 8) % spacing as u32) as usize;
                let at = z * side + x;
                let (u, v) = ((x as f32 + 0.5) / s, (z as f32 + 0.5) / s);
                let h = u32::from(self.ground.get(at).copied().unwrap_or(0));
                let range = f32::from(alpine.get(at).copied().unwrap_or(0)) / 255.0;
                if self.skin.get(at).copied() != Some(Skin::Earth as u8)
                    || self.drop(x as i64, z as i64) > 1
                    || u32::from(self.wet.get(at).copied().unwrap_or(0)) > 0
                {
                    continue;
                }
                let line = tall * (0.5 - 0.06 * range) - 34.0;
                let height_share = (h as f32 - SEA as f32) / (line - SEA as f32).max(1.0);
                let grove = fbm(u, v, 48, 3, seed ^ 0x72);
                let density = smooth(0.38, 0.62, grove) * (1.0 - smooth(0.75, 1.0, height_share));
                if unit(key >> 3) > density {
                    continue;
                }
                let high_up = smooth(0.25, 0.8, height_share);
                let size = unit(mixed(key ^ 0x5151));
                let tree = (6.0 + 9.0 * size + 5.0 * (1.0 - high_up)) as u32;
                let reach = if high_up > 0.5 { 2 } else { 3 };
                self.grown((x as i64, h, z as i64), tree, reach, high_up > 0.5, key);
            }
        }
    }

    fn grown(&mut self, (x, h, z): (i64, u32, i64), tree: u32, reach: i64, pine: bool, key: u32) {
        let top = self.grid.deep.saturating_sub(10);
        let trunk = (h + tree).min(top);
        let here = self.slot(x, z);
        if let Some(slot) = self.trunk.get_mut(here) {
            *slot = (*slot).max(trunk as u8);
        }
        let crown = if pine { h + 3 } else { h + tree / 2 };
        let crest = (trunk + 2).min(top);
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let off = (dx * dx + dz * dz) as f32;
                let radius = reach as f32 + 0.5;
                if off > radius * radius {
                    continue;
                }
                let share = off.sqrt() / radius;
                let (low, high) = if pine {
                    let reach_up = (1.0 - share) * (crest - crown) as f32;
                    (crown + (share * 2.0) as u32, crown + reach_up as u32 + 1)
                } else {
                    let middle = (crown + crest) / 2;
                    let half = ((1.0 - share * share).sqrt() * (crest - crown) as f32 * 0.5) as u32;
                    (middle.saturating_sub(half), middle + half + 1)
                };
                if (mixed(key ^ (dx as u32).wrapping_mul(31) ^ (dz as u32).wrapping_mul(97)) & 7)
                    == 0
                    && share > 0.6
                {
                    continue;
                }
                let at = self.slot(x + dx, z + dz);
                let ground = u32::from(self.ground.get(at).copied().unwrap_or(0));
                let low = low.max(ground + 1);
                if high <= low {
                    continue;
                }
                if let Some(slot) = self.low.get_mut(at) {
                    *slot = if *slot == 0 {
                        low as u8
                    } else {
                        (*slot).min(low as u8)
                    };
                }
                if let Some(slot) = self.high.get_mut(at) {
                    *slot = (*slot).max(high.min(top) as u8);
                }
            }
        }
    }

    fn column(&self, x: u32, z: u32) -> Column {
        let at = self.slot(i64::from(x), i64::from(z));
        let get = |field: &Vec<u8>| u32::from(field.get(at).copied().unwrap_or(0));
        let ground = get(&self.ground);
        let wet = get(&self.wet).max(ground);
        let trunk = get(&self.trunk).max(ground);
        let high = get(&self.high);
        let low = if high > 0 { get(&self.low) } else { 0 };
        Column {
            ground,
            wet,
            trunk,
            low,
            high,
            skin: get(&self.skin),
            depth: get(&self.depth),
            x,
            z,
        }
    }

    fn word(&self, column: &Column, up: u32) -> u32 {
        let Column {
            ground,
            wet,
            trunk,
            low,
            high,
            skin,
            depth,
            x,
            z,
        } = *column;
        if up < ground {
            let from_top = ground - 1 - up;
            let kind = if from_top < depth {
                match skin {
                    s if s == Skin::Snow as u32 => SNOW,
                    s if s == Skin::Ice as u32 => {
                        if from_top == 0 {
                            SNOW
                        } else {
                            ICE
                        }
                    }
                    s if s == Skin::Earth as u32 => EARTH,
                    s if s == Skin::Sand as u32 => SAND,
                    s if s == Skin::Bed as u32 => {
                        if hashed(x, z, up, self.seed).is_multiple_of(3) {
                            SAND
                        } else {
                            ROCK
                        }
                    }
                    _ => ROCK,
                }
            } else {
                ROCK
            };
            if kind == SNOW {
                return alive(SNOW, AMBIENT, hashed(x ^ up, z, 0x5A0, self.seed) & THAW);
            }
            return cell(kind, AMBIENT);
        }
        if up < wet {
            return cell(WATER, AMBIENT);
        }
        if up < trunk {
            return cell(WOOD, AMBIENT);
        }
        if high > 0 && up >= low && up < high {
            return cell(LEAVES, AMBIENT);
        }
        let below = if high > 0 && up >= high {
            high
        } else {
            wet.max(trunk)
        };
        let over = (up - below).min(DEEPEST);
        if up >= column.top() {
            open_air(AMBIENT, over)
        } else {
            air(AMBIENT, over)
        }
    }

    #[must_use]
    pub fn top(&self, x: u32, z: u32) -> u32 {
        self.column(x, z).top()
    }

    pub fn content(&self, bricks: &Bricks, brick: u32) -> Content {
        let [x0, y0, z0] = bricks.point_of(brick, 0);
        let [wide, deep, tall] = bricks.sides();
        let rows = self.grid.deep;
        let highest = rows - 1 - y0;
        let lowest = rows - (y0 + deep);
        let mut columns = Vec::with_capacity((wide * tall) as usize);
        let (mut top, mut solid) = (0, u32::MAX);
        for dz in 0..tall {
            for dx in 0..wide {
                let column = self.column(x0 + dx, z0 + dz);
                top = top.max(column.top());
                solid = solid.min(column.ground.saturating_sub(column.depth + 1));
                columns.push(column);
            }
        }
        if lowest >= top + DEEPEST {
            return Content::Uniform(open_air(AMBIENT, DEEPEST));
        }
        if highest < solid {
            return Content::Uniform(cell(ROCK, AMBIENT));
        }
        let cells = bricks.cells();
        let mut words = vec![0_u32; cells];
        for (offset, slot) in (0_u32..).zip(words.iter_mut()) {
            let [x, y, z] = bricks.point_of(brick, offset);
            let column = columns
                .get(((z - z0) * wide + (x - x0)) as usize)
                .copied()
                .unwrap_or_else(|| self.column(x, z));
            *slot = self.word(&column, rows - 1 - y);
        }
        Content::Mixed(words)
    }
}

#[derive(Debug, Clone, Copy)]
struct Column {
    ground: u32,
    wet: u32,
    trunk: u32,
    low: u32,
    high: u32,
    skin: u32,
    depth: u32,
    x: u32,
    z: u32,
}

impl Column {
    fn top(&self) -> u32 {
        self.ground.max(self.wet).max(self.trunk).max(self.high)
    }
}
