use grit::geometry::Solid;
use grit::render3::{Light, Material, Table};
use grit::rule3::code;
use grit::rule3::heat;
use grit::sand::Grid;

/// What the playground's tool bar offers in a scene: the tool's name, the
/// word it writes, and the radius of the ball it writes, in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    pub word: u32,
    pub radius: i32,
}

/// The scenes, in the order of the keys that switch to them, from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Valley,
    Sand,
    Fire,
    Code,
}

impl Which {
    pub const ALL: [Self; 4] = [Self::Valley, Self::Sand, Self::Fire, Self::Code];

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "valley" | "nature" => Some(Self::Valley),
            "sand" => Some(Self::Sand),
            "fire" | "heat" => Some(Self::Fire),
            "code" => Some(Self::Code),
            _ => None,
        }
    }

    pub fn example(self) -> Example {
        match self {
            Self::Valley => super::valley::example(),
            Self::Sand => sanded(),
            Self::Fire => fired(),
            Self::Code => coded(),
        }
    }
}

/// A scene: its name, the rule that runs it and that rule's kind mapping,
/// the rule's parameters, how its kinds look, its tools, how its world is
/// made at a given grid, how many frames run before the first one shown,
/// how far toward dusk its light is and how its light differs from the
/// plain one, how many engine steps a second it runs at when it asks for
/// a pace, the cycle it lives through on its own, if any, and the cells it
/// writes before every engine step, found from its world.
pub struct Example {
    pub name: &'static str,
    pub rule: &'static str,
    pub params: Vec<u32>,
    pub table: Table,
    pub tools: Vec<Tool>,
    pub scene: fn(Grid) -> Vec<u32>,
    pub warm: u32,
    pub dusk: f32,
    pub light: fn(Light) -> Light,
    pub pace: Option<f32>,
    pub life: Option<fn(Grid, &[u32]) -> super::life::Cycle>,
    pub feed: Option<Feed>,
}

/// How a scene finds the cells it writes before every step from its world:
/// each a cell index and its word.
pub type Feed = fn(Grid, &[u32]) -> Vec<(u32, u32)>;

/// The light as [`Light::at`] gives it.
pub const fn plain(light: Light) -> Light {
    light
}

fn put(cells: &mut [u32], grid: Grid, point: [i64; 3], word: u32) {
    let sides = [grid.wide, grid.deep, grid.tall];
    if point
        .iter()
        .zip(sides)
        .all(|(&at, side)| at >= 0 && at < i64::from(side))
    {
        let point = point.map(|value| value as u32);
        if let Some(slot) = cells.get_mut(Solid::cell(grid, point) as usize) {
            *slot = word;
        }
    }
}

fn get(cells: &[u32], grid: Grid, point: [i64; 3]) -> Option<u32> {
    let sides = [grid.wide, grid.deep, grid.tall];
    if point
        .iter()
        .zip(sides)
        .all(|(&at, side)| at >= 0 && at < i64::from(side))
    {
        let point = point.map(|value| value as u32);
        cells.get(Solid::cell(grid, point) as usize).copied()
    } else {
        None
    }
}

pub fn mixed(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn lattice(x: i32, z: i32, seed: u32) -> f32 {
    let key = (x as u32).wrapping_mul(0x9E37_79B1) ^ (z as u32).wrapping_mul(0x85EB_CA77) ^ seed;
    (mixed(key) & 0xFFFF) as f32 / 65535.0
}

/// Smooth value noise in [0, 1] at `(x, z)` with cells `size` wide.
fn noise(x: f32, z: f32, size: f32, seed: u32) -> f32 {
    let (u, v) = (x / size, z / size);
    let (i, j) = (u.floor() as i32, v.floor() as i32);
    let (fu, fv) = (u - u.floor(), v - v.floor());
    let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
    let a = lattice(i, j, seed);
    let b = lattice(i + 1, j, seed);
    let c = lattice(i, j + 1, seed);
    let d = lattice(i + 1, j + 1, seed);
    let low = a + (b - a) * su;
    let high = c + (d - c) * su;
    low + (high - low) * sv
}

fn layered(x: f32, z: f32, size: f32, seed: u32) -> f32 {
    noise(x, z, size, seed) * 0.5
        + noise(x, z, size * 0.5, seed ^ 0x55) * 0.27
        + noise(x, z, size * 0.25, seed ^ 0xAA) * 0.15
        + noise(x, z, size * 0.125, seed ^ 0x3C) * 0.08
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hashed(x: u32, y: u32, z: u32, seed: u32) -> u32 {
    mixed(x ^ y.rotate_left(10) ^ z.rotate_left(20) ^ seed)
}

/// The height field of a scene, in cells above the floor, and what each
/// layer of a column is made of, from its top down.
fn columns(
    cells: &mut [u32],
    grid: Grid,
    height: impl Fn(f32, f32) -> f32,
    layer: impl Fn(u32, u32, u32, u32) -> u32,
) -> Vec<u32> {
    let mut tops = vec![0; (grid.wide * grid.tall) as usize];
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let top = height(x as f32, z as f32).clamp(2.0, grid.deep as f32 - 4.0) as u32;
            if let Some(slot) = tops.get_mut((z * grid.wide + x) as usize) {
                *slot = top;
            }
            for up in 0..top {
                let y = grid.deep - 1 - up;
                let depth = top - 1 - up;
                put(
                    cells,
                    grid,
                    [i64::from(x), i64::from(y), i64::from(z)],
                    layer(x, z, depth, top),
                );
            }
        }
    }
    tops
}

fn top_of(tops: &[u32], grid: Grid, x: u32, z: u32) -> u32 {
    tops.get((z.min(grid.tall - 1) * grid.wide + x.min(grid.wide - 1)) as usize)
        .copied()
        .unwrap_or(0)
}

fn slope(tops: &[u32], grid: Grid, x: u32, z: u32) -> u32 {
    let here = top_of(tops, grid, x, z);
    let around = [
        top_of(tops, grid, x.saturating_sub(1), z),
        top_of(tops, grid, x + 1, z),
        top_of(tops, grid, x, z.saturating_sub(1)),
        top_of(tops, grid, x, z + 1),
    ];
    around
        .iter()
        .map(|&other| other.abs_diff(here))
        .max()
        .unwrap_or(0)
}

/// A ball of `word` of radius `radius` around `centre`, written over
/// every cell `keep` allows.
fn ball(
    cells: &mut [u32],
    grid: Grid,
    centre: [f32; 3],
    radius: [f32; 3],
    word: u32,
    keep: impl Fn(u32) -> bool,
) {
    let reach = radius.map(|value| value.ceil() as i64 + 1);
    for dz in -reach[2]..=reach[2] {
        for dy in -reach[1]..=reach[1] {
            for dx in -reach[0]..=reach[0] {
                let at = [
                    centre[0] as i64 + dx,
                    centre[1] as i64 + dy,
                    centre[2] as i64 + dz,
                ];
                let d = (0..3)
                    .map(|axis| {
                        let gap = (at.get(axis).copied().unwrap_or(0) as f32
                            - centre.get(axis).copied().unwrap_or(0.0))
                            / radius.get(axis).copied().unwrap_or(1.0).max(0.1);
                        gap * gap
                    })
                    .sum::<f32>();
                let wobble =
                    noise(at[0] as f32 + at[1] as f32 * 0.7, at[2] as f32, 3.0, word) * 0.35;
                if d + wobble <= 1.0 && get(cells, grid, at).is_some_and(&keep) {
                    put(cells, grid, at, word);
                }
            }
        }
    }
}

fn sanded() -> Example {
    use grit::sand::{AIR, SAND, WALL, WATER};
    let table = Table::empty()
        .with(
            SAND,
            Material::solid(0x00E2_BC72, 0.14)
                .strata(0.05)
                .surface(0.95, 0.03)
                .detail(0.45, 0.35, 0.0, 0.0)
                .wets(1.0),
        )
        .with(
            WATER,
            Material::solid(0x003F_B5B0, 0.02)
                .clear(0.82)
                .surface(0.06, 0.02)
                .detail(0.0, 0.55, 0.0, 0.0)
                .liquid(0x002F_7FA8),
        )
        .with(
            WALL,
            Material::solid(0x0084_7C73, 0.16)
                .strata(0.32)
                .surface(0.82, 0.04)
                .detail(0.35, 0.5, 0.6, 0.15)
                .mossy(0.55, 0x006B_7350)
                .wets(0.8),
        );
    Example {
        name: "sand",
        rule: "sand",
        params: Vec::new(),
        table,
        tools: vec![
            Tool {
                name: "drop sand",
                word: SAND,
                radius: 3,
            },
            Tool {
                name: "pour water",
                word: WATER,
                radius: 3,
            },
            Tool {
                name: "place rock",
                word: WALL,
                radius: 2,
            },
            Tool {
                name: "erase",
                word: AIR,
                radius: 4,
            },
        ],
        warm: 60,
        dusk: 0.0,
        light: plain,
        pace: None,
        life: None,
        feed: None,
        scene: |grid| {
            let mut cells = vec![AIR; grid.cells()];
            let s = grid.wide as f32;
            let lake = |u: f32, v: f32| {
                let a = ((u - 0.68) / 0.36).powi(2) + ((v - 0.95) / 0.3).powi(2);
                1.0 - smooth(0.5, 1.1, a)
            };
            let shelf = |u: f32, v: f32| {
                smooth(0.6, 0.66, u)
                    * (1.0 - smooth(0.9, 0.95, u))
                    * smooth(0.06, 0.1, v)
                    * (1.0 - smooth(0.22, 0.27, v))
            };
            let height = |x: f32, z: f32| {
                let (u, v) = (x / s, z / s);
                let hills = layered(x, z, s * 0.35, 11);
                let rough = layered(x, z, 8.0, 21) - 0.5;
                let ridge = 1.0 - smooth(0.12, 0.42, v);
                let crag = (layered(x, z, s * 0.1, 33) - 0.5).max(0.0);
                let notch = (1.0 - smooth(0.0, 0.035, (u - 0.75).abs())) * smooth(0.2, 0.3, v);
                s * (0.13 + 0.06 * hills + 0.3 * ridge * (0.7 + 0.3 * hills) + 0.25 * crag * ridge)
                    - s * 0.11 * lake(u, v)
                    - s * 0.05 * shelf(u, v)
                    - s * 0.12 * notch * ridge
                    + 2.0 * rough
            };
            let tops = columns(&mut cells, grid, height, |_, _, _, _| WALL);
            let level = (s * 0.115) as u32;
            for z in 0..grid.tall {
                for x in 0..grid.wide {
                    let (u, v) = (x as f32 / s, z as f32 / s);
                    let base = top_of(&tops, grid, x, z);
                    let at = |h: u32| [i64::from(x), i64::from(grid.deep - 1 - h), i64::from(z)];
                    let apart = ((u - 0.3).powi(2) + (v - 0.52).powi(2)).sqrt();
                    let cone = (1.0 - apart / 0.2).max(0.0);
                    if cone > 0.0 {
                        let hill = s * 0.36 * cone.powf(1.15)
                            + s * 0.02 * cone * layered(x as f32, z as f32, 10.0, 3);
                        for up in 0..(hill as u32) {
                            if base + up + 3 < grid.deep {
                                put(&mut cells, grid, at(base + up), SAND);
                            }
                        }
                    }
                    let beach = lake(u, v);
                    if beach > 0.0 && beach < 0.45 {
                        for up in 0..4 {
                            put(&mut cells, grid, at(base.saturating_sub(1) + up), SAND);
                        }
                    }
                    let dune = (layered(x as f32, z as f32, 16.0, 77) - 0.5).max(0.0);
                    if v > 0.4 && cone <= 0.0 && beach <= 0.0 {
                        for up in 0..(dune * 14.0) as u32 {
                            put(&mut cells, grid, at(base + up), SAND);
                        }
                    }
                    if beach > 0.4 {
                        for h in base..level {
                            put(&mut cells, grid, at(h), WATER);
                        }
                    }
                    if shelf(u, v) > 0.5 {
                        for up in 0..(s * 0.05) as u32 {
                            put(&mut cells, grid, at(base + up), WATER);
                        }
                    }
                }
            }
            cells
        },
    }
}

const COLD: u32 = 250;
const EARTH: u32 = 8;
const GRASS: u32 = 9;
const SHORE: u32 = 10;
const TRUNK: u32 = 11;
const SNOW: u32 = 12;

fn fired() -> Example {
    use heat::{AIR, ASH, FIRE, FUEL, ICE, STEAM, WALL, WATER, cell};
    let table = Table::empty()
        .with(
            WALL,
            Material::solid(0x007E_7872, 0.12)
                .strata(0.26)
                .surface(0.82, 0.04)
                .detail(0.35, 0.5, 0.6, 0.15)
                .mossy(0.55, 0x006B_7350)
                .wets(0.8),
        )
        .with(
            ICE,
            Material::solid(0x002C_5528, 0.3)
                .surface(0.75, 0.03)
                .detail(0.6, 0.5, 0.0, 0.1),
        )
        .with(
            WATER,
            Material::solid(0x003F_B5B0, 0.02)
                .clear(0.84)
                .surface(0.06, 0.02)
                .detail(0.0, 0.55, 0.0, 0.0)
                .liquid(0x002F_7FA8),
        )
        .with(STEAM, Material::solid(0x0060_5A55, 0.0).gas(0.5))
        .with(
            FUEL,
            Material::solid(0x0034_5C2C, 0.32)
                .surface(0.75, 0.03)
                .detail(0.6, 0.5, 0.0, 0.1),
        )
        .with(FIRE, Material::solid(0x00FF_5A14, 0.5).glowing(1.7))
        .with(
            ASH,
            Material::solid(0x0040_3D3A, 0.25).detail(0.5, 0.3, 0.0, 0.0),
        )
        .with(
            EARTH,
            Material::solid(0x0078_5A3C, 0.14)
                .strata(0.22)
                .detail(0.5, 0.45, 0.15, 0.0)
                .wets(0.7),
        )
        .with(
            GRASS,
            Material::solid(0x005E_7E38, 0.26)
                .detail(0.55, 0.4, 0.0, 0.3)
                .wets(0.5),
        )
        .with(
            SHORE,
            Material::solid(0x00D8_BE86, 0.12)
                .surface(0.95, 0.03)
                .detail(0.45, 0.35, 0.0, 0.0)
                .wets(1.0),
        )
        .with(
            TRUNK,
            Material::solid(0x005C_3F28, 0.2)
                .detail(0.6, 0.5, 0.2, 0.9)
                .mossy(0.35, 0x004F_5E32),
        )
        .with(
            SNOW,
            Material::solid(0x00F2_F3F5, 0.04)
                .surface(0.55, 0.03)
                .detail(0.15, 0.35, 0.0, 0.0)
                .sparkle(1.0),
        );
    Example {
        name: "fire",
        rule: "heat",
        params: heat::TUNED.to_vec(),
        table,
        tools: vec![
            Tool {
                name: "light fire",
                word: cell(FIRE, 900),
                radius: 2,
            },
            Tool {
                name: "pour water",
                word: cell(WATER, 280),
                radius: 3,
            },
            Tool {
                name: "plant forest",
                word: cell(FUEL, COLD),
                radius: 3,
            },
            Tool {
                name: "place rock",
                word: cell(WALL, COLD),
                radius: 2,
            },
            Tool {
                name: "erase",
                word: cell(AIR, COLD),
                radius: 4,
            },
        ],
        warm: 150,
        dusk: 0.0,
        light: plain,
        pace: None,
        life: None,
        feed: None,
        scene: |grid| {
            let mut cells = vec![cell(AIR, COLD); grid.cells()];
            let s = grid.wide as f32;
            let lake = |u: f32, v: f32| {
                let a = ((u - 0.74) / 0.3).powi(2) + ((v - 0.92) / 0.24).powi(2);
                1.0 - smooth(0.55, 1.15, a)
            };
            let river = |u: f32, v: f32| {
                let path = 0.36 + 0.3 * v + 0.06 * (v * 9.0).sin();
                let apart = (u - path).abs();
                (1.0 - smooth(0.012, 0.045, apart)) * smooth(0.18, 0.3, v)
            };
            let height = |x: f32, z: f32| {
                let (u, v) = (x / s, z / s);
                let peak = ((u - 0.24).powi(2) + (v - 0.2).powi(2)).sqrt();
                let mountain = (1.0 - peak / 0.42).max(0.0).powf(1.5);
                let ridge = (layered(x, z, s * 0.16, 41) - 0.5) * mountain;
                let hills = layered(x, z, s * 0.35, 11);
                let rough = layered(x, z, 7.0, 5) - 0.5;
                let base = s * (0.15 + 0.07 * hills) + s * 0.52 * mountain + s * 0.12 * ridge;
                let carved = base - s * 0.1 * lake(u, v) - s * 0.035 * river(u, v);
                carved + 2.0 * rough
            };
            let level = (s * 0.155) as u32;
            let tops = columns(&mut cells, grid, height, |x, z, depth, top| {
                let (u, v) = (x as f32 / s, z as f32 / s);
                let snowline = s * (0.43 + 0.04 * noise(x as f32, z as f32, 12.0, 9));
                let rocky = layered(x as f32, z as f32, 6.0, 13) > 0.62;
                if top as f32 > snowline && depth < 2 {
                    cell(SNOW, COLD)
                } else if top as f32 > snowline * 0.82 || rocky && depth < 3 && top > level + 20 {
                    cell(WALL, COLD)
                } else if (top <= level + 2 && lake(u, v) > 0.05 || river(u, v) > 0.3) && depth < 3
                {
                    cell(SHORE, COLD)
                } else if depth == 0 {
                    cell(GRASS, COLD)
                } else if depth < 5 {
                    cell(EARTH, COLD)
                } else {
                    cell(WALL, COLD)
                }
            });
            for z in 0..grid.tall {
                for x in 0..grid.wide {
                    let top = top_of(&tops, grid, x, z);
                    let (u, v) = (x as f32 / s, z as f32 / s);
                    let filled = if lake(u, v) > 0.05 {
                        level
                    } else if river(u, v) > 0.3 {
                        top + 2
                    } else {
                        0
                    };
                    for h in top..filled.min(grid.deep - 4) {
                        put(
                            &mut cells,
                            grid,
                            [i64::from(x), i64::from(grid.deep - 1 - h), i64::from(z)],
                            cell(WATER, 280),
                        );
                    }
                }
            }
            let mut key = 91_u32;
            let mut placed = 0;
            let mut tries = 0;
            let mut fires = Vec::new();
            while placed < (s * s / 220.0) as u32 && tries < 200_000 {
                tries += 1;
                key = mixed(key.wrapping_add(1));
                let x = (key & 0xFFFF) % grid.wide;
                let z = (key >> 16) % grid.tall;
                let (u, v) = (x as f32 / s, z as f32 / s);
                let top = top_of(&tops, grid, x, z);
                let ground = get(
                    &cells,
                    grid,
                    [i64::from(x), i64::from(grid.deep - top), i64::from(z)],
                );
                let grassy = ground.is_some_and(|word| heat::kind_of(word) == GRASS);
                let dense = noise(x as f32, z as f32, s * 0.12, 3) > 0.32;
                if !grassy || !dense || slope(&tops, grid, x, z) > 3 || lake(u, v) > 0.0 {
                    continue;
                }
                let tall = s * (0.05 + 0.05 * ((key >> 7) & 0xFF) as f32 / 255.0);
                let wide = tall * 0.3;
                let foot = grid.deep as f32 - top as f32;
                for up in 0..(tall * 0.35) as i64 {
                    put(
                        &mut cells,
                        grid,
                        [i64::from(x), foot as i64 - 1 - up, i64::from(z)],
                        cell(TRUNK, COLD),
                    );
                }
                let rise = tall * 0.2;
                let crown = (tall - rise) as i64;
                for step in 0..crown {
                    let share = 1.0 - step as f32 / crown as f32;
                    let radius = wide * share + 0.6;
                    let y = foot as i64 - 1 - rise as i64 - step;
                    let reach = radius.ceil() as i64;
                    for dz in -reach..=reach {
                        for dx in -reach..=reach {
                            let far = ((dx * dx + dz * dz) as f32).sqrt();
                            let point = [i64::from(x) + dx, y, i64::from(z) + dz];
                            let jag =
                                (hashed(point[0] as u32, point[1] as u32, point[2] as u32, key)
                                    & 0xFF) as f32
                                    / 255.0;
                            if far > radius + 0.4 * jag - 0.2 {
                                continue;
                            }
                            let frost = hashed(
                                point[0] as u32,
                                point[1] as u32,
                                point[2] as u32,
                                key ^ 0x51,
                            ) % 100
                                < 8;
                            let open = get(&cells, grid, point)
                                .is_some_and(|word| heat::kind_of(word) == AIR);
                            if open {
                                put(
                                    &mut cells,
                                    grid,
                                    point,
                                    if frost {
                                        cell(ICE, COLD)
                                    } else {
                                        cell(FUEL, COLD)
                                    },
                                );
                            }
                        }
                    }
                }
                let apart = ((u - 0.66).powi(2) + (v - 0.42).powi(2)).sqrt();
                if apart < 0.05 && fires.len() < 4 {
                    fires.push([
                        i64::from(x),
                        foot as i64 - 1 - rise as i64 - 2,
                        i64::from(z),
                    ]);
                }
                placed += 1;
            }
            for &at in &fires {
                for dy in 0..3 {
                    put(
                        &mut cells,
                        grid,
                        [at[0], at[1] - dy, at[2]],
                        cell(FIRE, 900),
                    );
                    put(
                        &mut cells,
                        grid,
                        [at[0] + 1, at[1] - dy, at[2]],
                        cell(FIRE, 900),
                    );
                }
            }
            let _ = (ASH, STEAM);
            let boulders = (s * s / 3000.0) as u32;
            for at in 0..boulders {
                let key = mixed(at.wrapping_mul(7919) ^ 0xB0);
                let x = key % grid.wide;
                let z = (key >> 12) % grid.tall;
                let top = top_of(&tops, grid, x, z);
                let (u, v) = (x as f32 / s, z as f32 / s);
                if lake(u, v) > 0.0 || river(u, v) > 0.0 {
                    continue;
                }
                let r = 1.5 + (key >> 28) as f32 * 0.3;
                ball(
                    &mut cells,
                    grid,
                    [x as f32, (grid.deep - top) as f32 - 0.5, z as f32],
                    [r, r * 0.7, r],
                    cell(WALL, COLD),
                    |word| heat::kind_of(word) == AIR,
                );
            }
            cells
        },
    }
}

fn coded() -> Example {
    use code::{ADD, COPY, DATA, HALT, MINUS_Y, MOVE, NOP, PLUS_X, PLUS_Z, TURN, word};
    let mut table = Table::empty()
        .with(DATA, Material::solid(0x002F_8FC0, 0.12).glowing(0.8))
        .with(
            MOVE,
            Material::solid(0x00FF_B030, 0.12).glowing(2.4).gap(0.2),
        )
        .with(
            COPY,
            Material::solid(0x007F_D17F, 0.1).glowing(1.8).gap(0.2),
        )
        .with(ADD, Material::solid(0x00E0_70E0, 0.1).glowing(1.8).gap(0.2))
        .with(TURN, Material::solid(0x00D9_6A2E, 0.1).glowing(0.6))
        .with(HALT, Material::solid(0x004E_525B, 0.1).strata(0.08));
    for inert in 7..16 {
        table = table.with(inert, Material::solid(0x0060_6060, 0.1));
    }
    Example {
        name: "code",
        rule: "code",
        params: Vec::new(),
        table,
        tools: vec![
            Tool {
                name: "send packets",
                word: word(MOVE, PLUS_X, 1),
                radius: 1,
            },
            Tool {
                name: "drop data",
                word: word(DATA, 0, 1),
                radius: 2,
            },
            Tool {
                name: "place wall",
                word: word(HALT, 0, 0),
                radius: 2,
            },
            Tool {
                name: "erase",
                word: word(NOP, 0, 0),
                radius: 3,
            },
        ],
        warm: 40,
        dusk: 0.35,
        light: plain,
        pace: None,
        life: None,
        feed: None,
        scene: |grid| {
            let mut cells = vec![word(NOP, 0, 0); grid.cells()];
            let (w, d, t) = (grid.wide, grid.deep, grid.tall);
            let floor = d - 1;
            let at = |x: u32, y: u32, z: u32| [i64::from(x), i64::from(y), i64::from(z)];
            for z in 0..t {
                for x in 0..w {
                    for up in 0..d / 16 {
                        put(&mut cells, grid, at(x, floor - up, z), word(HALT, 0, 0));
                    }
                }
            }
            let ground = floor - d / 16;
            let top = ground - d * 5 / 16;
            let lattice = w * 5 / 8;
            for z in t / 8..t * 7 / 8 {
                for y in top..=ground {
                    let beam = (z % 16) < 3 || (y % 16) < 3;
                    if beam {
                        for dx in 0..3 {
                            put(&mut cells, grid, at(lattice + dx, y, z), word(HALT, 0, 0));
                        }
                    }
                    for dx in 0..2 {
                        put(&mut cells, grid, at(w * 7 / 8 + dx, y, z), word(HALT, 0, 0));
                    }
                }
            }
            for z in (t / 8 + 8..t * 7 / 8).step_by(32) {
                for y in (top + 8..ground - 2).step_by(32) {
                    for dy in 0..2 {
                        put(
                            &mut cells,
                            grid,
                            at(w * 3 / 8, y + dy, z),
                            word(TURN, PLUS_Z, 0),
                        );
                    }
                }
            }
            for z in (t / 8 + 8..t * 7 / 8).step_by(12) {
                for y in (top + 6..ground - 2).step_by(12) {
                    for x in (4..w * 3 / 10).step_by(3) {
                        put(&mut cells, grid, at(x, y, z), word(MOVE, PLUS_X, x + y + z));
                    }
                }
            }
            for x in (w / 6..w / 2).step_by((w / 6).max(1) as usize) {
                for z in (t / 4..t * 3 / 4).step_by((t / 4).max(1) as usize) {
                    for up in (0..d / 3).step_by(4) {
                        put(
                            &mut cells,
                            grid,
                            at(x, ground - 2 - up, z),
                            word(MOVE, MINUS_Y, x),
                        );
                    }
                }
            }
            cells
        },
    }
}
