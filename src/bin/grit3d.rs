use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{Dimension, GRAND, PARAMS, PIXELS, STATE};
use grit::render3::{
    self, Build, Camera, HIT_WORDS, INSIDE, Light, MISS, Material, Screen, Table, of, scene_words,
};
use grit::rule3::code::{self, Code};
use grit::rule3::heat::{self, Heat};
use grit::rule3::sand::Sand;
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::Grid;
use grit::view3::Kinds;
use grit::{Groups, Host, Resident, live, sweep};

type Failed = Box<dyn std::error::Error>;

const VIEW: f32 = 0.95;

const SPREAD: f32 = 0.02;

const TIMED: usize = 60;

const WARM: usize = 20;

struct Example {
    rule: &'static str,
    params: Vec<u32>,
    table: Table,
    words: Vec<(&'static str, u32)>,
    scene: fn(Grid) -> Vec<u32>,
    haze: f32,
}

#[derive(Debug, Clone)]
struct Asked {
    rule: String,
    side: u32,
    screen: Screen,
    frames: Option<u32>,
    shot: Option<String>,
    camera: usize,
    measure: bool,
    suns: u32,
}

fn asked() -> Result<Asked, Failed> {
    let mut held = Asked {
        rule: "sand".to_owned(),
        side: 128,
        screen: Screen {
            wide: 1280,
            high: 720,
        },
        frames: None,
        shot: None,
        camera: 0,
        measure: false,
        suns: 2,
    };
    let mut args = std::env::args().skip(1);
    while let Some(one) = args.next() {
        let mut value = || -> Result<String, Failed> {
            args.next()
                .ok_or_else(|| format!("{one} needs a value").into())
        };
        match one.as_str() {
            "--rule" => held.rule = value()?,
            "--size" => held.side = value()?.parse()?,
            "--width" => held.screen.wide = value()?.parse()?,
            "--height" => held.screen.high = value()?.parse()?,
            "--frames" => held.frames = Some(value()?.parse()?),
            "--shot" => held.shot = Some(value()?),
            "--camera" => held.camera = value()?.parse()?,
            "--suns" => held.suns = value()?.parse()?,
            "--measure" => held.measure = true,
            other => {
                return Err(format!(
                    "unknown argument {other}; grit3d takes --rule sand|heat|code, --size, --width, --height, --frames N, --shot FILE.png, --camera 0|1|2, --suns 1-4 and --measure"
                )
                .into());
            }
        }
    }
    Ok(held)
}

fn main() -> Result<(), Failed> {
    let asked = asked()?;
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    match asked.rule.as_str() {
        "sand" => run(&host, &Sand, &sanded(), &asked),
        "heat" => run(&host, &Heat, &heated(), &asked),
        "code" => run(&host, &Code, &coded(), &asked),
        other => Err(format!("--rule {other}: sand, heat or code").into()),
    }
}

fn put(cells: &mut [u32], grid: Grid, point: [u32; 3], word: u32) {
    if point
        .iter()
        .zip([grid.wide, grid.deep, grid.tall])
        .all(|(&at, side)| at < side)
        && let Some(slot) = cells.get_mut(Solid::cell(grid, point) as usize)
    {
        *slot = word;
    }
}

fn mixed(mut x: u32) -> u32 {
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
    noise(x, z, size, seed) * 0.6
        + noise(x, z, size * 0.5, seed ^ 0x55) * 0.28
        + noise(x, z, size * 0.25, seed ^ 0xAA) * 0.12
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Fills each column from the floor up to `height(x, z)` cells with
/// `word(x, y, z)`.
fn ground(
    cells: &mut [u32],
    grid: Grid,
    height: impl Fn(u32, u32) -> f32,
    word: impl Fn(u32, u32, u32) -> u32,
) {
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let top = height(x, z).clamp(1.0, grid.deep as f32 - 2.0) as u32;
            for up in 0..top {
                let y = grid.deep - 1 - up;
                put(cells, grid, [x, y, z], word(x, y, z));
            }
        }
    }
}

fn sanded() -> Example {
    use grit::sand::{AIR, SAND, WALL, WATER};
    let table = Table::empty()
        .with(SAND, Material::solid(0x00E3_B262, 0.14))
        .with(WATER, Material::solid(0x002A_6478, 0.02).clear(0.8))
        .with(WALL, Material::solid(0x006C_6862, 0.2));
    Example {
        rule: "sand",
        params: Vec::new(),
        table,
        words: vec![
            ("sand", SAND),
            ("water", WATER),
            ("rock", WALL),
            ("air", AIR),
        ],
        haze: 0.0007,
        scene: |grid| {
            let mut cells = vec![AIR; grid.cells()];
            let s = grid.wide as f32;
            let rock = |x: u32, z: u32| {
                let (u, v) = (x as f32 / s, z as f32 / s);
                let edge = (u.min(1.0 - u)).min(v.min(1.0 - v));
                let rim = 1.0 - smooth(0.0, 0.2, edge);
                let rough = layered(x as f32, z as f32, s * 0.3, 7) - 0.5;
                let shelf =
                    smooth(0.62, 0.7, u) * smooth(0.14, 0.24, v) * smooth(0.14, 0.24, 1.0 - v);
                let valley = 0.06 + 0.06 * ((v - 0.5) * 2.0).powi(2);
                s * (valley + 0.26 * rim + 0.2 * shelf + 0.04 * rough)
            };
            ground(&mut cells, grid, rock, |_, _, _| WALL);
            for z in 0..grid.tall {
                for x in 0..grid.wide {
                    let (u, v) = (x as f32 / s, z as f32 / s);
                    let apart = ((u - 0.28).powi(2) + (v - 0.5).powi(2)).sqrt();
                    let cone = (1.0 - apart / 0.22).max(0.0);
                    if cone <= 0.0 {
                        continue;
                    }
                    let hill =
                        s * 0.5 * cone + s * 0.03 * cone * layered(x as f32, z as f32, s * 0.1, 3);
                    let base = rock(x, z);
                    for up in 0..(hill as u32) {
                        let height = base as u32 + up;
                        if height + 1 < grid.deep {
                            put(&mut cells, grid, [x, grid.deep - 1 - height, z], SAND);
                        }
                    }
                    let _ = v;
                }
            }
            for z in 0..grid.tall {
                for x in 0..grid.wide {
                    let (u, v) = (x as f32 / s, z as f32 / s);
                    if !(0.72..0.9).contains(&u) || !(0.26..0.74).contains(&v) {
                        continue;
                    }
                    let base = rock(x, z) as u32;
                    for up in 0..(s * 0.2) as u32 {
                        let height = base + up;
                        if height + 1 < grid.deep {
                            put(&mut cells, grid, [x, grid.deep - 1 - height, z], WATER);
                        }
                    }
                }
            }
            cells
        },
    }
}

fn heated() -> Example {
    use heat::{AIR, ASH, FIRE, FUEL, ICE, STEAM, WALL, WATER, cell};
    let table = Table::empty()
        .with(WALL, Material::solid(0x0068_5E44, 0.22))
        .with(ICE, Material::solid(0x00D2_ECF4, 0.05).clear(0.55))
        .with(WATER, Material::solid(0x002A_6478, 0.02).clear(0.8))
        .with(STEAM, Material::solid(0x00E4_E6E8, 0.06).clear(0.4))
        .with(FUEL, Material::solid(0x003C_6A2C, 0.3))
        .with(FIRE, Material::solid(0x00F0_4A12, 0.6).glowing(2.2))
        .with(ASH, Material::solid(0x0048_4746, 0.2));
    Example {
        rule: "heat",
        params: heat::TUNED.to_vec(),
        table,
        words: vec![
            ("fire", cell(FIRE, 900)),
            ("fuel", cell(FUEL, 250)),
            ("water", cell(WATER, 280)),
            ("ice", cell(ICE, 250)),
            ("ground", cell(WALL, 250)),
            ("air", cell(AIR, 250)),
        ],
        haze: 0.0007,
        scene: |grid| {
            let cold = 250;
            let mut cells = vec![cell(AIR, cold); grid.cells()];
            let s = grid.wide as f32;
            let land = |x: u32, z: u32| {
                s * (0.1 + 0.08 * layered(x as f32, z as f32, s * 0.4, 11))
                    + 2.5 * layered(x as f32, z as f32, 6.0, 5)
            };
            ground(&mut cells, grid, land, |_, _, _| cell(WALL, cold));
            for z in 0..grid.tall {
                for x in 0..grid.wide {
                    let (u, v) = (x as f32 / s, z as f32 / s);
                    let apart = ((u - 0.75).powi(2) + (v - 0.72).powi(2)).sqrt();
                    if apart < 0.12 {
                        let top = land(x, z) as u32;
                        for up in top.saturating_sub(3)..top {
                            put(
                                &mut cells,
                                grid,
                                [x, grid.deep - 1 - up, z],
                                cell(WATER, 280),
                            );
                        }
                    }
                }
            }
            let mut trees = Vec::new();
            let mut key = 17_u32;
            while trees.len() < 22 {
                key = mixed(key.wrapping_add(1));
                let u = 0.12 + 0.76 * (key & 0xFFFF) as f32 / 65535.0;
                let v = 0.12 + 0.76 * (key >> 16) as f32 / 65535.0;
                let pond = ((u - 0.75).powi(2) + (v - 0.72).powi(2)).sqrt() < 0.16;
                let crowded = trees.iter().any(|&(a, b, _): &(f32, f32, u32)| {
                    ((a - u).powi(2) + (b - v).powi(2)).sqrt() < 0.11
                });
                if !pond && !crowded {
                    trees.push((u, v, key));
                }
            }
            for &(u, v, key) in &trees {
                let (tx, tz) = ((u * s) as u32, (v * s) as u32);
                let base = land(tx, tz) as u32;
                let trunk = (s * (0.1 + 0.06 * ((key >> 8) & 0xFF) as f32 / 255.0)) as u32;
                let crown = s * (0.05 + 0.03 * (key & 0xFF) as f32 / 255.0);
                for up in 0..trunk {
                    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        put(
                            &mut cells,
                            grid,
                            [tx + dx, grid.deep - 1 - (base + up), tz + dz],
                            cell(FUEL, cold),
                        );
                    }
                }
                let centre = [
                    tx as f32,
                    (grid.deep - 1 - (base + trunk)) as f32,
                    tz as f32,
                ];
                let reach = crown.ceil() as i32 + 1;
                for dz in -reach..=reach {
                    for dy in -reach..=reach {
                        for dx in -reach..=reach {
                            let at = [
                                centre[0] + dx as f32,
                                centre[1] + dy as f32 * 1.0,
                                centre[2] + dz as f32,
                            ];
                            let far =
                                ((dx * dx) as f32 + (dy * dy) as f32 * 1.4 + (dz * dz) as f32)
                                    .sqrt();
                            let wobble = layered(at[0] + at[1], at[2], 3.0, key) * 1.6;
                            if far + wobble > crown + 0.8 || at.iter().any(|&value| value < 0.0) {
                                continue;
                            }
                            let point = [at[0] as u32, at[1] as u32, at[2] as u32];
                            let wet = mixed(point[0] ^ (point[1] << 10) ^ (point[2] << 20) ^ key)
                                .is_multiple_of(11);
                            let word = if wet {
                                cell(ICE, cold)
                            } else {
                                cell(FUEL, cold)
                            };
                            put(&mut cells, grid, point, word);
                        }
                    }
                }
            }
            if let Some(&(u, v, _)) = trees.first() {
                let (tx, tz) = ((u * s) as u32, (v * s) as u32);
                let base = land(tx, tz) as u32;
                for up in 0..3 {
                    put(
                        &mut cells,
                        grid,
                        [tx + 2, grid.deep - 1 - (base + up), tz],
                        cell(FIRE, 900),
                    );
                    put(
                        &mut cells,
                        grid,
                        [tx - 1, grid.deep - 1 - (base + up), tz + 1],
                        cell(FIRE, 900),
                    );
                }
            }
            cells
        },
    }
}

fn coded() -> Example {
    use code::{ADD, COPY, DATA, HALT, MINUS_Y, MOVE, NOP, PLUS_X, PLUS_Z, TURN, word};
    let mut table = Table::empty()
        .with(DATA, Material::solid(0x002F_8FC0, 0.12).glowing(0.8))
        .with(MOVE, Material::solid(0x00FF_B030, 0.15).glowing(2.6))
        .with(COPY, Material::solid(0x007F_D17F, 0.1).glowing(2.0))
        .with(ADD, Material::solid(0x00E0_70E0, 0.1).glowing(2.0))
        .with(TURN, Material::solid(0x00D9_6A2E, 0.1).glowing(0.6))
        .with(HALT, Material::solid(0x004A_4E57, 0.12));
    for inert in 7..16 {
        table = table.with(inert, Material::solid(0x0060_6060, 0.1));
    }
    Example {
        rule: "code",
        params: Vec::new(),
        table,
        words: vec![
            ("packet", word(MOVE, PLUS_X, 1)),
            ("halt", word(HALT, 0, 0)),
            ("turn up", word(TURN, MINUS_Y, 0)),
            ("data", word(DATA, 0, 1)),
            ("copy", word(COPY, PLUS_Z, 0)),
            ("empty", word(NOP, 0, 0)),
        ],
        haze: 0.0007,
        scene: |grid| {
            let mut cells = vec![word(NOP, 0, 0); grid.cells()];
            let (w, d, t) = (grid.wide, grid.deep, grid.tall);
            let floor = d - 1;
            for z in 0..t {
                for x in 0..w {
                    put(&mut cells, grid, [x, floor, z], word(HALT, 0, 0));
                    put(&mut cells, grid, [x, floor - 1, z], word(HALT, 0, 0));
                }
            }
            let top = floor - d * 3 / 8;
            let lattice = w * 5 / 8;
            for z in t / 8..t * 7 / 8 {
                for y in top..floor - 1 {
                    let beam = (z % 8) < 2 || (y % 8) < 2;
                    if beam {
                        put(&mut cells, grid, [lattice, y, z], word(HALT, 0, 0));
                        put(&mut cells, grid, [lattice + 1, y, z], word(HALT, 0, 0));
                    }
                    put(&mut cells, grid, [w * 7 / 8, y, z], word(HALT, 0, 0));
                }
            }
            for z in (t / 8 + 4..t * 7 / 8).step_by(16) {
                for y in (top + 4..floor - 2).step_by(16) {
                    for dy in 0..2 {
                        put(
                            &mut cells,
                            grid,
                            [w * 3 / 8, y + dy, z],
                            word(TURN, PLUS_Z, 0),
                        );
                    }
                }
            }
            for z in (t / 8 + 4..t * 7 / 8).step_by(8) {
                for y in (top + 4..floor - 2).step_by(8) {
                    for x in (2..w * 3 / 10).step_by(2) {
                        put(&mut cells, grid, [x, y, z], word(MOVE, PLUS_X, x + y + z));
                    }
                }
            }
            for x in (w / 6..w / 2).step_by(w as usize / 6) {
                for z in (t / 4..t * 3 / 4).step_by(t as usize / 4) {
                    for up in (0..d / 3).step_by(3) {
                        put(
                            &mut cells,
                            grid,
                            [x, floor - 2 - up, z],
                            word(MOVE, MINUS_Y, x),
                        );
                    }
                }
            }
            cells
        },
    }
}

/// Where each rule's cameras stand and what they look at, in sides of the
/// world.
fn framed(rule: &str, which: usize, grid: Grid) -> Camera {
    let s = grid.wide as f32;
    let (at, target): ([f32; 3], [f32; 3]) = match (rule, which) {
        ("sand", 0) => ([1.08, -0.42, -0.14], [0.48, 0.8, 0.52]),
        ("sand", 1) => ([0.9, 0.24, 0.84], [0.32, 0.78, 0.42]),
        ("heat", 0) => ([1.1, -0.3, -0.1], [0.5, 0.78, 0.5]),
        ("heat", 1) => ([0.05, 0.52, 0.1], [0.45, 0.72, 0.5]),
        ("code", 0) => ([-0.36, -0.12, -0.36], [0.5, 0.74, 0.5]),
        ("code", 1) => ([0.22, 0.55, 0.06], [0.62, 0.7, 0.55]),
        (_, _) => ([1.75, -0.75, -0.95], [0.5, 0.78, 0.5]),
    };
    Camera::looking(
        at.map(|value| value * s),
        target.map(|value| value * s),
        VIEW,
    )
}

struct Orbit {
    target: [f32; 3],
    yaw: f32,
    pitch: f32,
    far: f32,
}

impl Orbit {
    fn of(camera: Camera) -> Self {
        let [forward, _, _] = camera.basis();
        let far = 100.0;
        Self {
            target: [0, 1, 2].map(|axis| of(camera.at, axis) + of(forward, axis) * far),
            yaw: camera.yaw,
            pitch: camera.pitch,
            far,
        }
    }

    fn camera(&self) -> Camera {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let at = [
            self.target[0] - cos_pitch * cos_yaw * self.far,
            self.target[1] - sin_pitch * self.far,
            self.target[2] - cos_pitch * sin_yaw * self.far,
        ];
        Camera {
            at,
            yaw: self.yaw,
            pitch: self.pitch,
            view: VIEW,
        }
    }
}

struct Session<'h> {
    held: Resident<'h>,
    build: Build,
    plan: Plan,
    stages: Vec<Vec<Groups>>,
    still: Vec<Vec<Groups>>,
    watcher: usize,
    engine_stages: usize,
}

fn session<'h, R: Rule>(
    host: &'h Host,
    rule: &R,
    example: &Example,
    asked: &Asked,
) -> Result<Session<'h>, Failed> {
    let width = host.width();
    let grid = Grid::solid(asked.side, asked.side, asked.side)?;
    let kinds = Kinds::named(example.rule).ok_or("no kinds for the rule")?;
    let build = Build {
        grid,
        kinds,
        screen: asked.screen,
        subgroup: width,
        suns: asked.suns,
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
    let mut held = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words])?;
    }
    held.write(STATE as usize, &(example.scene)(grid))?;
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
    Ok(Session {
        held,
        build,
        plan,
        stages,
        still,
        watcher,
        engine_stages,
    })
}

impl Session<'_> {
    fn light(&self, example: &Example) -> Light {
        let mut light = Light::evening(self.build.screen.high, VIEW);
        light.haze = example.haze;
        light
    }

    fn aim(&mut self, camera: &Camera, example: &Example) -> Result<(), Failed> {
        let light = self.light(example);
        let suns = light.sun_rays(self.build.suns as usize, SPREAD);
        let rays = camera.rays(self.build.grid, self.build.screen);
        let words = scene_words(&rays, &example.table, &suns, &light);
        self.held
            .write_at(PIXELS as usize, self.build.layout().scene(), &words)?;
        Ok(())
    }

    fn frame(
        &mut self,
        moving: bool,
        first: bool,
        reading: bool,
    ) -> Result<(Duration, Vec<u32>), Failed> {
        let engine = if moving { &self.stages } else { &self.still };
        let render = self.build.stages(first);
        let mut groups: Vec<&[Groups]> = engine.iter().map(Vec::as_slice).collect();
        groups.extend(render.iter().map(Vec::as_slice));
        if reading {
            Ok(self
                .held
                .staged_reading(&groups, PIXELS as usize, self.build.screen.pixels())?)
        } else {
            Ok((self.held.staged(&groups)?, Vec::new()))
        }
    }

    /// The cell a pixel sees, and the face it sees it by.
    fn under(&mut self, (x, y): (u32, u32)) -> Result<Option<(u32, u32)>, Failed> {
        let layout = self.build.layout();
        let at =
            layout.hits() + (y as usize * self.build.screen.wide as usize + x as usize) * HIT_WORDS;
        let words = self.held.read(PIXELS as usize, at + HIT_WORDS)?;
        let cell = words.get(at).copied().unwrap_or(MISS);
        let info = words.get(at + 1).copied().unwrap_or(0);
        Ok((cell != MISS).then_some((cell, info & 7)))
    }

    fn drop_word(&mut self, (cell, face): (u32, u32), word: u32) -> Result<(), Failed> {
        let grid = self.build.grid;
        let mut point = Solid::point(grid, cell).map(|value| value as i32);
        if face != INSIDE {
            let axis = (face / 2) as usize;
            let sign = if face % 2 == 1 { 1 } else { -1 };
            if let Some(slot) = point.get_mut(axis) {
                *slot += sign;
            }
        }
        let mut cells = Vec::new();
        for dz in -1..=1_i32 {
            for dy in -1..=1_i32 {
                for dx in -1..=1_i32 {
                    if dx * dx + dy * dy + dz * dz > 2 {
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
                        cells.push((Solid::cell(grid, at), word));
                    }
                }
            }
        }
        cells.sort_unstable();
        cells.dedup();
        live::put3(&mut self.held, grid, &self.plan, &cells)?;
        Ok(())
    }
}

fn run<R: Rule>(host: &Host, rule: &R, example: &Example, asked: &Asked) -> Result<(), Failed> {
    if !render3::fits(
        Grid::solid(asked.side, asked.side, asked.side)?,
        asked.screen,
    ) {
        return Err(render3::UNFIT.into());
    }
    if asked.measure {
        return measured(host, rule, example, asked);
    }
    let mut session = session(host, rule, example, asked)?;
    let grid = session.build.grid;
    let screen = session.build.screen;
    println!(
        "{} width {} {} {}x{}x{} screen {}x{} suns {}",
        host.name(),
        host.width(),
        example.rule,
        grid.wide,
        grid.deep,
        grid.tall,
        screen.wide,
        screen.high,
        session.build.suns
    );
    let mut camera = framed(example.rule, asked.camera, grid);
    let mut orbit = Orbit::of(camera);
    let mut window = minifb::Window::new(
        "grit3d",
        screen.wide as usize,
        screen.high as usize,
        minifb::WindowOptions::default(),
    )?;
    window.set_target_fps(0);
    let mut frame = 0_u32;
    let mut shown = 0_u32;
    let mut picture = Vec::new();
    let mut paused = false;
    let mut chosen = 0_usize;
    let mut dragged: Option<(f32, f32)> = None;
    let mut clock = Instant::now();
    let mut counted = 0_u32;
    let mut cost = Duration::ZERO;
    let mut rate = 0.0;
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let step;
        {
            use minifb::{Key, KeyRepeat, MouseButton, MouseMode};
            let pressed = |key| window.is_key_pressed(key, KeyRepeat::No);
            if pressed(Key::Space) {
                paused = !paused;
            }
            step = paused && pressed(Key::N);
            let digits = [
                Key::Key1,
                Key::Key2,
                Key::Key3,
                Key::Key4,
                Key::Key5,
                Key::Key6,
                Key::Key7,
                Key::Key8,
                Key::Key9,
            ];
            for (at, key) in digits.into_iter().enumerate().take(example.words.len()) {
                if pressed(key) {
                    chosen = at;
                }
            }
            let speed = orbit.far * 0.012 + 0.3;
            let flat = [orbit.yaw.cos(), orbit.yaw.sin()];
            let mut moved = [0.0_f32; 3];
            if window.is_key_down(Key::W) {
                moved = [flat[0], 0.0, flat[1]];
            }
            if window.is_key_down(Key::S) {
                moved = [-flat[0], 0.0, -flat[1]];
            }
            if window.is_key_down(Key::A) {
                moved = [flat[1], 0.0, -flat[0]];
            }
            if window.is_key_down(Key::D) {
                moved = [-flat[1], 0.0, flat[0]];
            }
            if window.is_key_down(Key::Q) {
                moved[1] = 1.0;
            }
            if window.is_key_down(Key::E) {
                moved[1] = -1.0;
            }
            for (target, change) in orbit.target.iter_mut().zip(moved) {
                *target += change * speed;
            }
            if let Some((_, wheel)) = window.get_scroll_wheel() {
                orbit.far = (orbit.far * (1.0 - wheel * 0.1)).clamp(4.0, 900.0);
            }
            let held = window.get_mouse_down(MouseButton::Left);
            let mouse = window.get_mouse_pos(MouseMode::Discard);
            match (held, mouse, dragged) {
                (true, Some(now), Some(before)) => {
                    orbit.yaw += (now.0 - before.0) * 0.006;
                    orbit.pitch = (orbit.pitch + (now.1 - before.1) * 0.006).clamp(-0.2, 1.45);
                    dragged = Some(now);
                }
                (true, Some(now), None) => dragged = Some(now),
                _ => dragged = None,
            }
            if window.get_mouse_down(MouseButton::Right)
                && let Some((x, y)) = mouse
                && x >= 0.0
                && y >= 0.0
                && (x as u32) < screen.wide
                && (y as u32) < screen.high
                && let Some(seen) = session.under((x as u32, y as u32))?
            {
                let word = example.words.get(chosen).map_or(0, |held| held.1);
                session.drop_word(seen, word)?;
            }
        }
        if shown > 0 {
            camera = orbit.camera().kept(grid);
        }
        session.aim(&camera, example)?;
        let moving = !paused || step;
        let (spent, got) = session.frame(moving, shown == 0, true)?;
        picture = got;
        cost += spent;
        counted += 1;
        if moving {
            frame = frame.wrapping_add(1);
        }
        window.update_with_buffer(&picture, screen.wide as usize, screen.high as usize)?;
        shown += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            rate = f64::from(counted) / clock.elapsed().as_secs_f64();
            clock = Instant::now();
            cost = Duration::ZERO;
            counted = 0;
            let listed = session
                .held
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0);
            let word = example.words.get(chosen).map_or("", |held| held.0);
            window.set_title(&format!(
                "grit3d {} | frame {frame} | {rate:.0} fps | {listed} octs listed | drop: {word} | drag orbit, wheel zoom, WASD/QE fly, right click drop, 1-{} word, space pause{}",
                example.rule,
                example.words.len(),
                if paused { " (paused)" } else { "" }
            ));
        }
        if asked.frames.is_some_and(|last| shown >= last) {
            break;
        }
    }
    let _ = (cost, rate);
    if let Some(path) = &asked.shot {
        std::fs::write(path, render3::png(&picture, screen))?;
        println!("{path}: frame {frame}, {}x{}", screen.wide, screen.high);
    }
    Ok(())
}

fn milliseconds(times: &mut [Duration]) -> (f64, f64) {
    times.sort_unstable();
    let least = times.first().copied().unwrap_or_default();
    let middle = times.get(times.len() / 2).copied().unwrap_or_default();
    (least.as_secs_f64() * 1000.0, middle.as_secs_f64() * 1000.0)
}

fn measured<R: Rule>(
    host: &Host,
    rule: &R,
    example: &Example,
    asked: &Asked,
) -> Result<(), Failed> {
    let mut session = session(host, rule, example, asked)?;
    let grid = session.build.grid;
    let camera = framed(example.rule, asked.camera, grid);
    session.aim(&camera, example)?;
    let mut totals = Vec::with_capacity(TIMED);
    let mut engine = Vec::with_capacity(TIMED);
    let mut watcher = Vec::with_capacity(TIMED);
    let mut marching = Vec::with_capacity(TIMED);
    let mut shading = Vec::with_capacity(TIMED);
    let mut listed = Vec::with_capacity(TIMED);
    for at in 0..WARM + TIMED {
        let _ = session.frame(true, at == 0, false)?;
        if at < WARM {
            continue;
        }
        let spans = session.held.stage_times().to_vec();
        let total = session
            .held
            .on_device()
            .ok_or("the device keeps no timestamps")?;
        let mut all = session.stages.clone();
        all.extend(session.build.stages(false));
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
        let stages = session.engine_stages;
        totals.push(total);
        engine.push((0..stages).map(time_of).sum::<Duration>());
        watcher.push(time_of(session.watcher));
        marching.push(time_of(stages + 1));
        shading.push(time_of(stages + 2));
        listed.push(
            session
                .held
                .read(GRAND as usize, 1)?
                .first()
                .copied()
                .unwrap_or(0),
        );
    }
    let (frame_min, frame_mid) = milliseconds(&mut totals);
    let (engine_min, engine_mid) = milliseconds(&mut engine);
    let (watch_min, watch_mid) = milliseconds(&mut watcher);
    let (march_min, march_mid) = milliseconds(&mut marching);
    let (shade_min, shade_mid) = milliseconds(&mut shading);
    listed.sort_unstable();
    println!(
        "{} | {} {}^3 | {}x{} | suns {} | frames {WARM}..{} | listed octs median {} | frame {frame_min:.3}/{frame_mid:.3} ms | engine {engine_min:.3}/{engine_mid:.3} | bricks {watch_min:.3}/{watch_mid:.3} | march {march_min:.3}/{march_mid:.3} | shade {shade_min:.3}/{shade_mid:.3} | {:.1} fps at the median frame",
        host.name(),
        example.rule,
        grid.wide,
        asked.screen.wide,
        asked.screen.high,
        session.build.suns,
        WARM + TIMED,
        listed.get(listed.len() / 2).copied().unwrap_or(0),
        1000.0 / frame_mid.max(1e-6)
    );
    Ok(())
}
