#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, PARAMS, PIXELS, STATE};
use grit::live;
use grit::render3::march::{Ray, World, brick_of, index, sides};
use grit::render3::{
    self, Build, Camera, HIT_WORDS, INSIDE, Light, MISS, Material, Rays, STEEPEST, Screen, Shading,
    Table, UNIT, occupancy, scene_words,
};
use grit::rule3::code::Code;
use grit::rule3::heat::{self, Heat};
use grit::rule3::sand::Sand;
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::Grid;
use grit::sand3::{self, Grid3};
use grit::sweep;
use grit::view3::Kinds;
use grit::{Groups, Host, Resident};
use simdr::lanes::LaneError;

const RULES: [&str; 3] = ["sand", "heat", "code"];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn below(state: &mut u64, end: u32) -> u32 {
    (next(state) % u64::from(end.max(1))) as u32
}

fn between(state: &mut u64, low: i32, high: i32) -> i32 {
    low + (next(state) % (high - low + 1) as u64) as i32
}

fn kinds(rule: &str) -> Kinds {
    Kinds::named(rule).unwrap()
}

/// A table that shows kind 0 as empty, and the other kinds as opaque, seen
/// through or empty, chosen by `seed`.
fn table(kinds: Kinds, seed: u64) -> Table {
    let mut state = seed;
    let mut out = Table::empty();
    for kind in 1..=kinds.mask() {
        let colour = next(&mut state) as u32 & 0xFF_FFFF;
        let material = match below(&mut state, 5) {
            0 => continue,
            1 => Material::solid(colour, 0.3).clear(0.7),
            2 => Material::solid(colour, 0.2).glowing(2.0),
            _ => Material::solid(colour, 0.25),
        };
        out = out.with(kind, material);
    }
    out
}

/// Random words, about three in four of kind 0.
fn sparse(cells: usize, kinds: Kinds, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let field = kinds.mask() << kinds.shift();
    (0..cells)
        .map(|_| {
            let held = next(&mut state) as u32;
            if next(&mut state).is_multiple_of(4) {
                held
            } else {
                held & !field
            }
        })
        .collect()
}

/// Random words in clumps: whole bricks empty, others full, so the march
/// both jumps and walks.
fn clumped(grid: Grid, kinds: Kinds, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let field = kinds.mask() << kinds.shift();
    let bricks = render3::bricks(grid) as usize;
    let filled: Vec<bool> = (0..bricks).map(|_| below(&mut state, 3) == 0).collect();
    let mut cells = vec![0; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let held = next(&mut state) as u32;
                let brick = brick_of(grid, [x, y, z]) as usize;
                let keep = filled[brick] && below(&mut state, 3) == 0;
                cells[index(grid, [x, y, z]) as usize] = if keep { held } else { held & !field };
            }
        }
    }
    cells
}

/// Random rays that keep the march exact, most of them aimed near the
/// grid, with some whose start lies on cell walls and whose directions are
/// small whole numbers, so ties between axes are common.
fn rays(state: &mut u64, grid: Grid, screen: Screen) -> Rays {
    let sides = sides(grid);
    let unit = UNIT as i32;
    loop {
        let tied = below(state, 3) == 0;
        let origin: [i32; 3] = std::array::from_fn(|axis| {
            let side = sides[axis] as i32;
            let low = (side - 1023 + 1) * unit;
            let high = (1023 - 1) * unit;
            let near = between(state, -2 * side * unit, 3 * side * unit);
            let at = if below(state, 6) == 0 {
                between(state, low, high)
            } else {
                near.clamp(low, high)
            };
            if tied { at & !(unit / 2 - 1) } else { at }
        });
        let aim: [i32; 3] = std::array::from_fn(|axis| {
            let side = sides[axis] as i32 * unit;
            between(state, side / 4, 3 * side / 4) - origin[axis]
        });
        let reach = (screen.wide + screen.high) as i32;
        let longest = aim
            .iter()
            .map(|value| value.abs())
            .max()
            .unwrap_or(1)
            .max(1);
        let (forward, small) = if tied {
            let forward = aim.map(|value| {
                let size = between(state, 1, 6);
                if below(state, 5) == 0 {
                    between(state, -6, 6)
                } else {
                    value.signum() * size
                }
            });
            (forward, 1)
        } else {
            let scale = f64::from(STEEPEST / 2) / f64::from(longest);
            let forward = aim.map(|value| {
                (f64::from(value) * scale) as i32 + between(state, -STEEPEST / 16, STEEPEST / 16)
            });
            (forward, STEEPEST / 8 / reach)
        };
        let made = Rays {
            origin,
            forward,
            right: std::array::from_fn(|_| between(state, -small, small)),
            up: std::array::from_fn(|_| between(state, -small, small)),
        };
        if made.fit(grid, screen) {
            return made;
        }
    }
}

fn plain_world<'a>(
    grid: Grid,
    kinds: Kinds,
    table: &Table,
    cells: &'a [u32],
    bricks: &'a [u32],
) -> World<'a> {
    World {
        grid,
        kinds,
        visible: table.visible(),
        opaque: table.opaque(),
        cells,
        bricks,
    }
}

#[test]
fn a_ray_crossing_empty_bricks_in_jumps_finds_what_the_plain_walk_finds() {
    let grids = [(16, 8, 32), (4, 4, 4), (32, 32, 8)];
    let screen = Screen { wide: 7, high: 5 };
    let mut state = 0x7A11;
    let mut checked = 0;
    let mut hits = 0;
    let mut wrong = Vec::new();
    for (seed, rule) in RULES.into_iter().enumerate() {
        let held = kinds(rule);
        let table = table(held, 0xAB + seed as u64);
        for &(wide, deep, tall) in &grids {
            let grid = Grid::solid(wide, deep, tall).unwrap();
            let cells = clumped(grid, held, 0x11 + seed as u64);
            let bricks = occupancy(grid, held, table.visible(), &cells);
            let world = plain_world(grid, held, &table, &cells, &bricks);
            for _ in 0..400 {
                let made = rays(&mut state, grid, screen);
                for y in 0..screen.high {
                    for x in 0..screen.wide {
                        let direction = made.direction(screen, (x, y));
                        if direction == [0; 3] {
                            continue;
                        }
                        let seen = world.seen(made.origin, direction);
                        let walked = world.walked(made.origin, direction);
                        let jumped = (seen[0] != MISS).then_some((seen[0], seen[1] & 7));
                        if jumped != walked {
                            wrong.push(format!(
                                "{rule} {wide}x{deep}x{tall} from {:?} along {direction:?}: {jumped:?}, the walk {walked:?}",
                                made.origin
                            ));
                        }
                        hits += usize::from(walked.is_some());
                        checked += 1;
                    }
                }
            }
        }
    }
    println!("{checked} rays, {hits} hit, {} differ", wrong.len());
    assert!(
        hits > checked / 10 && hits < checked,
        "{hits} of {checked} hit"
    );
    assert!(
        wrong.is_empty(),
        "{}",
        wrong[..wrong.len().min(20)].join("\n")
    );
}

#[test]
fn ties_between_axes_go_x_then_y_then_z() {
    let ray = Ray::from([8, 8, 8], [5, 5, 5]);
    let one = ray.stepped();
    assert_eq!(one.cell, [1, 0, 0]);
    assert_eq!(one.face, 0);
    let two = one.stepped();
    assert_eq!(two.cell, [1, 1, 0]);
    assert_eq!(two.face, 2);
    let three = two.stepped();
    assert_eq!(three.cell, [1, 1, 1]);
    assert_eq!(three.face, 4);
    let back = Ray::from([8, 8, 8], [-3, -3, 0]).stepped();
    assert_eq!(back.cell, [u32::MAX, 0, 0]);
    assert_eq!(back.face, 1);
}

#[test]
fn a_ray_enters_the_grid_through_the_face_it_reaches_last() {
    let grid = Grid::solid(8, 8, 8).unwrap();
    let ray = Ray::from([-40, 24, 24], [16, 0, 0]);
    let entered = ray.entered(sides(grid)).unwrap();
    assert_eq!(entered.cell, [0, 1, 1]);
    assert_eq!(entered.face, 0);
    let above = Ray::from([24, -100, 24], [1, 16, 2]);
    let entered = above.entered(sides(grid)).unwrap();
    assert_eq!(entered.cell[1], 0);
    assert_eq!(entered.face, 2);
    let away = Ray::from([-40, 24, 24], [-16, 0, 0]);
    assert_eq!(away.entered(sides(grid)), None);
    let past = Ray::from([-40, 24, 24], [16, 0, 100]);
    assert_eq!(past.entered(sides(grid)), None);
    let inside = Ray::from([24, 24, 24], [1, 1, 1]);
    assert_eq!(inside.entered(sides(grid)), Some(inside));
    assert_eq!(inside.face, INSIDE);
}

#[test]
fn a_camera_gives_rays_that_keep_the_march_exact() {
    for side in [4, 64, 128, 256, 512] {
        let grid = Grid::solid(side, side, side).unwrap();
        for screen in [
            Screen { wide: 1, high: 1 },
            Screen {
                wide: 1920,
                high: 1080,
            },
            Screen {
                wide: 4096,
                high: 7,
            },
        ] {
            for at in [[-5000.0, 3.0, 0.5], [0.5, 0.5, 0.5], [3000.0, -800.0, 40.0]] {
                let camera = Camera::looking(at, [2.0, 3.0, 4.0], 1.0);
                let made = camera.rays(grid, screen);
                assert!(made.fit(grid, screen), "{side} {screen:?} {at:?}");
            }
        }
    }
}

#[test]
fn a_grid_or_screen_the_renderer_cannot_draw_is_refused() {
    let build = |grid: Grid, wide: u32, high: u32| Build {
        grid,
        kinds: kinds("heat"),
        screen: Screen { wide, high },
        subgroup: 32,
        suns: 2,
    };
    let solid = Grid::solid(16, 16, 16).unwrap();
    for refused in [
        build(Grid::of(16, 16), 8, 8),
        build(Grid::solid(16, 2, 16).unwrap(), 8, 8),
        build(Grid::solid(1024, 16, 16).unwrap(), 8, 8),
        build(solid, 0, 8),
        build(solid, 8, 4097),
    ] {
        for built in [
            render3::primary(&refused),
            render3::shade(&refused),
            render3::filled(&refused),
            render3::occupied(&refused),
        ] {
            match built {
                Err(LaneError::NoSuchForm { because, .. }) => assert_eq!(because, render3::UNFIT),
                other => panic!("{refused:?} gave {other:?}"),
            }
        }
    }
    assert!(build(solid, 1, 1).passes().is_ok());
    assert!(
        render3::primary(&Build {
            subgroup: 12,
            ..build(solid, 8, 8)
        })
        .is_err()
    );
}

#[test]
fn a_table_tells_which_kinds_are_seen_and_which_block_light() {
    let made = Table::empty()
        .with(1, Material::solid(0xFF_0000, 0.1))
        .with(2, Material::solid(0x00_00FF, 0.0).clear(0.5))
        .with(3, Material::EMPTY)
        .with(15, Material::solid(0xFF_FFFF, 0.0).glowing(4.0))
        .with(16, Material::solid(0xFF_FFFF, 0.0));
    assert_eq!(made.visible(), 0b1000_0000_0000_0110);
    assert_eq!(made.opaque(), 0b1000_0000_0000_0010);
    assert_eq!(made.words().len(), 16 * render3::MATERIAL_WORDS);
    assert!((made.material(1).albedo[0] - 1.0).abs() < 1e-6);
}

#[test]
fn the_brick_watcher_runs_right_after_the_listed_step() {
    let grid = Grid::solid(16, 16, 16).unwrap();
    for width in [4, 32, 64] {
        let plan = plan_of(width, grid);
        let engine = live::wired3_by(&Sand, width, grid, &plan).unwrap();
        let (passes, stages) = engine.clone();
        let build = Build {
            grid,
            kinds: kinds("sand"),
            screen: Screen { wide: 8, high: 4 },
            subgroup: width,
            suns: 1,
        };
        let added = build.passes().unwrap();
        let (watched, groups) = render3::watching(engine, added.clone()).unwrap();
        let step = stages
            .iter()
            .position(|stage| stage == &vec![Plan::listed()])
            .unwrap();
        let before: usize = stages.iter().take(step + 1).map(Vec::len).sum();
        assert_eq!(groups.len(), stages.len() + 1);
        assert_eq!(groups[step + 1], vec![Plan::listed()]);
        assert_eq!(watched[before], added[0]);
        assert_eq!(watched.len(), passes.len() + added.len());
        assert_eq!(&watched[watched.len() - 3..], &added[1..]);
    }
}

fn plan_of(width: u32, grid: Grid) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap()
}

/// A world held on the device with the picture's passes and nothing of the
/// engine: the occupancy rebuild, the march and the shading, one a stage.
struct Still<'h> {
    held: Resident<'h>,
    build: Build,
}

impl<'h> Still<'h> {
    fn new(host: &'h Host, build: Build, cells: &[u32]) -> Self {
        let passes = [
            render3::filled(&build).unwrap(),
            render3::primary(&build).unwrap(),
            render3::shade(&build).unwrap(),
        ];
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let mut sizes = [1_usize; BINDINGS as usize];
        sizes[PARAMS as usize] = Dimension::Three.params_words();
        sizes[STATE as usize] = build.grid.cells();
        let sizes = build.layout().sized(sizes);
        let mut held = host.resident(&sizes, &modules).unwrap();
        for (binding, &words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0; words]).unwrap();
        }
        held.write(STATE as usize, cells).unwrap();
        Self { held, build }
    }

    fn set(&mut self, table: &Table, scene: &[u32]) {
        let layout = self.build.layout();
        self.held
            .write_at(PIXELS as usize, layout.table(), &table.words())
            .unwrap();
        self.held
            .write_at(PIXELS as usize, layout.scene(), scene)
            .unwrap();
    }

    fn run(&mut self, which: usize) {
        let layout = self.build.layout();
        let bricks = layout.bricks.div_ceil(self.build.subgroup);
        let pixels = self.build.screen.groups(self.build.subgroup);
        let mut stages = [
            vec![Groups::Fixed(0)],
            vec![Groups::Fixed(0)],
            vec![Groups::Fixed(0)],
        ];
        stages[which] = vec![[Groups::Fixed(bricks), pixels, pixels][which]];
        let refs: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        self.held.staged(&refs).unwrap();
    }

    fn read(&mut self, at: usize, count: usize) -> Vec<u32> {
        let all = self.held.read(PIXELS as usize, at + count).unwrap();
        all[at..].to_vec()
    }
}

fn first_difference(got: &[u32], want: &[u32], per: usize) -> Option<String> {
    if got.len() != want.len() {
        return Some(format!("{} words, {} wanted", got.len(), want.len()));
    }
    let at = got.iter().zip(want).position(|(a, b)| a != b)?;
    let wrong = got.iter().zip(want).filter(|(a, b)| a != b).count();
    let start = at - at % per;
    Some(format!(
        "pixel {}: {:x?}, {:x?} wanted; {wrong} words differ",
        at / per,
        &got[start..start + per],
        &want[start..start + per]
    ))
}

#[test]
fn the_device_march_finds_the_cpu_hits_bit_for_bit_for_random_cameras_and_worlds() {
    let Some(host) = device("render3 march") else {
        return;
    };
    let width = host.width();
    let screens = [
        Screen { wide: 37, high: 23 },
        Screen { wide: 3, high: 2 },
        Screen { wide: 1, high: 1 },
    ];
    let grids = [(16, 8, 32), (16, 16, 16)];
    let mut state = 0xD1CE;
    let mut wrong = Vec::new();
    let mut cameras = 0;
    let mut pixels = 0;
    let mut seen = 0;
    for (seed, rule) in RULES.into_iter().enumerate() {
        let held = kinds(rule);
        let table = table(held, 0x51 + seed as u64);
        for (at_grid, &(wide, deep, tall)) in grids.iter().enumerate() {
            let grid = Grid::solid(wide, deep, tall).unwrap();
            let cells = if at_grid == 0 {
                clumped(grid, held, 0x99 + seed as u64)
            } else {
                sparse(grid.cells(), held, 0x77 + seed as u64)
            };
            let bricks = occupancy(grid, held, table.visible(), &cells);
            let world = plain_world(grid, held, &table, &cells, &bricks);
            for screen in screens {
                let build = Build {
                    grid,
                    kinds: held,
                    screen,
                    subgroup: width,
                    suns: 1,
                };
                let layout = build.layout();
                let mut still = Still::new(&host, build, &cells);
                still.set(
                    &table,
                    &scene_words(
                        &rays(&mut state, grid, screen),
                        &table,
                        &[],
                        &Light::evening(8, 1.0),
                    ),
                );
                still.run(0);
                let got = still.read(layout.occupancy(), layout.bricks as usize);
                if got != bricks {
                    wrong.push(format!("{rule} {wide}x{deep}x{tall}: the bricks differ"));
                }
                for _ in 0..12 {
                    let made = rays(&mut state, grid, screen);
                    still.set(
                        &table,
                        &scene_words(&made, &table, &[], &Light::evening(8, 1.0)),
                    );
                    still.run(1);
                    let got = still.read(layout.hits(), screen.pixels() * HIT_WORDS);
                    let shading = Shading {
                        world,
                        screen,
                        layout,
                        pixels: &[],
                        suns: 1,
                    };
                    let want = shading.hits(&made);
                    if let Some(why) = first_difference(&got, &want, HIT_WORDS) {
                        wrong.push(format!(
                            "{rule} {wide}x{deep}x{tall} {}x{} rays {made:?}: {why}",
                            screen.wide, screen.high
                        ));
                    }
                    cameras += 1;
                    pixels += screen.pixels();
                    seen += want.chunks(HIT_WORDS).filter(|hit| hit[0] != MISS).count();
                }
            }
        }
    }
    println!(
        "width {width}: {cameras} cameras, {pixels} pixels, {seen} hit, {} differ from the CPU",
        wrong.len()
    );
    assert!(seen > pixels / 10, "{seen} of {pixels} pixels hit");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The largest difference of any channel of any pixel.
fn widest(got: &[u32], want: &[u32]) -> (u32, usize) {
    let mut most = 0;
    let mut over = 0;
    for (a, b) in got.iter().zip(want) {
        let gap = [16, 8, 0]
            .iter()
            .map(|shift| ((a >> shift) & 0xFF).abs_diff((b >> shift) & 0xFF))
            .max()
            .unwrap_or(0);
        most = most.max(gap);
        over += usize::from(gap > TOLERANCE);
    }
    (most, over)
}

/// The most any channel of a shaded pixel may differ from the CPU.
const TOLERANCE: u32 = 2;

#[test]
fn the_shaded_picture_is_within_two_of_the_cpu_on_every_channel() {
    let Some(host) = device("render3 shading") else {
        return;
    };
    let width = host.width();
    let screen = Screen { wide: 48, high: 32 };
    let mut state = 0x5AD;
    let mut wrong = Vec::new();
    let mut most = 0;
    for (seed, rule) in RULES.into_iter().enumerate() {
        let held = kinds(rule);
        let table = table(held, 0x61 + seed as u64);
        let grid = Grid::solid(16, 16, 16).unwrap();
        let cells = clumped(grid, held, 0x31 + seed as u64);
        let bricks = occupancy(grid, held, table.visible(), &cells);
        let world = plain_world(grid, held, &table, &cells, &bricks);
        for suns in [1, 2, 4] {
            let build = Build {
                grid,
                kinds: held,
                screen,
                subgroup: width,
                suns,
            };
            let layout = build.layout();
            let mut still = Still::new(&host, build, &cells);
            for _ in 0..3 {
                let camera = Camera::looking(
                    [
                        between(&mut state, -20, 36) as f32,
                        between(&mut state, -30, 8) as f32,
                        between(&mut state, -20, 36) as f32,
                    ],
                    [8.0, 8.0, 8.0],
                    0.9,
                );
                let made = camera.rays(grid, screen);
                let light = Light::evening(screen.high, 0.9);
                let suns_made = light.sun_rays(suns as usize, 0.05);
                let scene = scene_words(&made, &table, &suns_made, &light);
                still.set(&table, &scene);
                still.run(0);
                still.run(1);
                still.run(2);
                let hits = still.read(layout.hits(), screen.pixels() * HIT_WORDS);
                let got = still.read(0, screen.pixels());
                let mut words = vec![0; layout.hits()];
                words[layout.table()..layout.table() + table.words().len()]
                    .copy_from_slice(&table.words());
                words[layout.scene()..layout.scene() + scene.len()].copy_from_slice(&scene);
                let shading = Shading {
                    world,
                    screen,
                    layout,
                    pixels: &words,
                    suns,
                };
                let want = shading.picture(&hits).unwrap();
                let (gap, over) = widest(&got, &want);
                most = most.max(gap);
                if over > 0 {
                    wrong.push(format!(
                        "{rule} suns {suns} camera {camera:?}: {over} pixels past {TOLERANCE}, the widest {gap}"
                    ));
                }
                let lit = want.iter().filter(|&&pixel| pixel != want[0]).count();
                assert!(lit > 0, "{rule}: a picture of one colour");
            }
        }
    }
    println!(
        "width {width}: the widest channel difference is {most}, {} pictures past {TOLERANCE}",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

struct Ran {
    name: &'static str,
    params: Vec<u32>,
}

/// Runs a world of `rule` on the listed engine with the brick watcher and
/// the march after every frame, writes cells between frames, and checks
/// the bricks and the hits against the CPU after each frame.
fn running<R: Rule>(host: &Host, rule: &R, ran: &Ran) -> Vec<String> {
    let width = host.width();
    let (wide, deep, tall) = (16, 16, 16);
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let reference = Grid3::new(wide, deep, tall).unwrap();
    let plan = plan_of(width, grid);
    let held_kinds = kinds(ran.name);
    let table = table(held_kinds, 0x4242);
    let screen = Screen { wide: 24, high: 16 };
    let build = Build {
        grid,
        kinds: held_kinds,
        screen,
        subgroup: width,
        suns: 1,
    };
    let layout = build.layout();
    let engine = live::wired3_by(rule, width, grid, &plan).unwrap();
    let (passes, mut stages) = render3::watching(engine, build.passes().unwrap()).unwrap();
    let sizes = layout.sized(live::sizes3(rule, &plan));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0; words]).unwrap();
    }
    let mut cells = sparse(reference.cells(), held_kinds, 0xBEE5);
    held.write(STATE as usize, &cells).unwrap();
    if !ran.params.is_empty() {
        held.write_at(PARAMS as usize, FIRST_PARAM, &ran.params)
            .unwrap();
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    held.write_at(PIXELS as usize, layout.table(), &table.words())
        .unwrap();
    stages.extend(build.stages(false));
    let mut state = 0xF00D;
    let mut wrong = Vec::new();
    for frame in 0..16_u32 {
        if frame % 4 == 2 {
            let mut writes: Vec<(u32, u32)> = (0..6)
                .map(|_| {
                    (
                        below(&mut state, grid.cells() as u32),
                        next(&mut state) as u32,
                    )
                })
                .collect();
            writes.sort_unstable();
            writes.dedup_by_key(|write| write.0);
            live::put3(&mut held, grid, &plan, &writes).unwrap();
            for &(at, word) in &writes {
                cells[at as usize] = word;
            }
        }
        let made = rays(&mut state, grid, screen);
        let light = Light::evening(screen.high, 1.0);
        held.write_at(
            PIXELS as usize,
            layout.scene(),
            &scene_words(&made, &table, &[], &light),
        )
        .unwrap();
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        held.staged(&groups).unwrap();
        cells = sand3::reference(rule, reference, frame, &cells, &ran.params);
        let bricks = occupancy(grid, held_kinds, table.visible(), &cells);
        let all = held.read(PIXELS as usize, layout.claims()).unwrap();
        if all[layout.occupancy()..layout.claims()] != bricks[..] {
            let differ = all[layout.occupancy()..]
                .iter()
                .zip(&bricks)
                .filter(|(a, b)| a != b)
                .count();
            wrong.push(format!(
                "{} frame {frame}: {differ} bricks differ",
                ran.name
            ));
        }
        let world = plain_world(grid, held_kinds, &table, &cells, &bricks);
        let shading = Shading {
            world,
            screen,
            layout,
            pixels: &[],
            suns: 1,
        };
        let want = shading.hits(&made);
        let got = &all[layout.hits()..layout.hits() + screen.pixels() * HIT_WORDS];
        if let Some(why) = first_difference(got, &want, HIT_WORDS) {
            wrong.push(format!("{} frame {frame}: {why}", ran.name));
        }
    }
    let state = held.read(STATE as usize, reference.cells()).unwrap();
    if state != cells {
        wrong.push(format!("{}: the world left the reference", ran.name));
    }
    wrong
}

#[test]
fn a_running_world_keeps_its_bricks_and_its_hits_equal_to_the_cpu_under_every_rule() {
    let Some(host) = device("render3 running world") else {
        return;
    };
    let mut wrong = running(
        &host,
        &Sand,
        &Ran {
            name: "sand",
            params: Vec::new(),
        },
    );
    wrong.extend(running(
        &host,
        &Heat,
        &Ran {
            name: "heat",
            params: heat::TUNED.to_vec(),
        },
    ));
    wrong.extend(running(
        &host,
        &Code,
        &Ran {
            name: "code",
            params: Vec::new(),
        },
    ));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_quiet_world_rebuilds_no_brick() {
    let Some(host) = device("render3 quiet world") else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(16, 16, 16).unwrap();
    let plan = plan_of(width, grid);
    let held_kinds = kinds("code");
    let table = table(held_kinds, 0x4243);
    let build = Build {
        grid,
        kinds: held_kinds,
        screen: Screen { wide: 4, high: 4 },
        subgroup: width,
        suns: 1,
    };
    let layout = build.layout();
    let engine = live::wired3_by(&Code, width, grid, &plan).unwrap();
    let (passes, mut stages) = render3::watching(engine, build.passes().unwrap()).unwrap();
    let sizes = layout.sized(live::sizes3(&Code, &plan));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0; words]).unwrap();
    }
    let data = grit::rule3::code::word(grit::rule3::code::DATA, 0, 7);
    let cells: Vec<u32> = (0..grid.cells())
        .map(|at| if at % 5 == 0 { data } else { 0 })
        .collect();
    held.write(STATE as usize, &cells).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let mut state = 0x9;
    let made = rays(&mut state, grid, build.screen);
    held.write_at(
        PIXELS as usize,
        layout.scene(),
        &scene_words(&made, &table, &[], &Light::evening(4, 1.0)),
    )
    .unwrap();
    held.write_at(PIXELS as usize, layout.table(), &table.words())
        .unwrap();
    stages.extend(build.stages(false));
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    let mut frames = 0;
    loop {
        held.staged(&groups).unwrap();
        frames += 1;
        let listed = held.read(grit::layout::GRAND as usize, 1).unwrap()[0];
        if listed == 0 {
            break;
        }
        assert!(frames < 16, "a world of data alone did not go quiet");
    }
    let before = held.read(PIXELS as usize, layout.words()).unwrap();
    for _ in 0..3 {
        held.staged(&groups).unwrap();
    }
    let claims = held.read(PIXELS as usize, layout.words()).unwrap();
    assert_eq!(
        &claims[layout.claims()..],
        &before[layout.claims()..],
        "a quiet frame claimed a brick"
    );
    assert!(
        claims[layout.claims()..].iter().all(|&stamp| stamp != 0),
        "the seeded frame claims every brick"
    );
    println!("quiet after {frames} frames; three more claimed no brick");
    let bricks = occupancy(grid, held_kinds, table.visible(), &cells);
    let got = &claims[layout.occupancy()..layout.claims()];
    let differ = got.iter().zip(&bricks).filter(|(a, b)| a != b).count();
    assert_eq!(
        differ,
        0,
        "{differ} bricks differ; {} full here, {} on the CPU, visible {:#x}",
        got.iter().sum::<u32>(),
        bricks.iter().sum::<u32>(),
        table.visible()
    );
}
