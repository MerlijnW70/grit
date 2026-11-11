#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;
use common::device;

use grit::bricks::Bricks;
use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{Dimension, PARAMS, PIXELS};
use grit::render3::{
    self, Build, Camera, Frame, Light, Material, Screen, Sparse, Steps, Table, framed_words,
};
use grit::rule3::nature::{self, Nature};
use grit::sand::Grid;
use grit::view3::Kinds;
use grit::{Groups, Host, Resident, live, sweep};

const VIEW: f32 = 0.62;

const SCREEN: Screen = Screen { wide: 96, high: 64 };

fn table() -> Table {
    Table::empty()
        .with(
            nature::ROCK,
            Material::solid(0x006E_6C6A, 0.22).strata(0.42),
        )
        .with(nature::EARTH, Material::solid(0x0055_4A30, 0.24))
        .with(nature::SAND, Material::solid(0x00D6_B572, 0.12))
        .with(nature::SNOW, Material::solid(0x00EE_F2F8, 0.05))
        .with(
            nature::WATER,
            Material::solid(0x002A_7A84, 0.02)
                .clear(0.8)
                .liquid(0x0010_3040),
        )
        .with(nature::WOOD, Material::solid(0x003E_2E22, 0.22))
        .with(nature::LEAVES, Material::solid(0x0026_3E1E, 0.34))
        .with(nature::FIRE, Material::solid(0x00FF_6418, 0.5).glowing(2.0))
        .with(nature::SMOKE, Material::solid(0x0058_524D, 0.0).gas(0.62))
}

fn ground(x: u32, z: u32, sides: [u32; 3]) -> u32 {
    let [wide, deep, tall] = sides.map(|side| side as f32);
    let (u, v) = (x as f32 / wide, z as f32 / tall);
    let hills = (u * 9.0).sin() * (v * 7.0).cos() + 0.5 * (u * 23.0 + v * 17.0).sin();
    let height = deep * (0.3 + 0.12 * hills);
    height.clamp(4.0, deep * 0.6) as u32
}

fn world(grid: Grid) -> Vec<u32> {
    let sides = [grid.wide, grid.deep, grid.tall];
    let level = grid.deep * 27 / 100;
    let snowline = grid.deep * 38 / 100;
    let mut cells = vec![nature::air(260, 0); grid.cells()];
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let height = ground(x, z, sides);
            for up in 0..grid.deep {
                let y = grid.deep - 1 - up;
                let kind = if up < height {
                    if up + 1 == height && height >= snowline {
                        nature::SNOW
                    } else if up + 3 >= height {
                        if height <= level + 1 {
                            nature::SAND
                        } else {
                            nature::EARTH
                        }
                    } else {
                        nature::ROCK
                    }
                } else if up < level {
                    nature::WATER
                } else {
                    continue;
                };
                cells[Solid::cell(grid, [x, y, z]) as usize] = nature::cell(kind, 260);
            }
        }
    }
    let mut tree = |x: u32, z: u32| {
        let height = ground(x, z, sides);
        if height <= level + 2 {
            return;
        }
        let base = grid.deep - height;
        for up in 1..9 {
            let y = base - up;
            cells[Solid::cell(grid, [x, y, z]) as usize] = nature::cell(nature::WOOD, 260);
        }
        for dz in 0..5_u32 {
            for dy in 0..4_u32 {
                for dx in 0..5_u32 {
                    let at = [x + dx - 2, base - 9 - dy, z + dz - 2];
                    if at[0] < grid.wide && at[2] < grid.tall {
                        cells[Solid::cell(grid, at) as usize] = nature::cell(nature::LEAVES, 260);
                    }
                }
            }
        }
    };
    let step = (grid.wide / 8).max(8);
    for z in (step / 2..grid.tall - 4).step_by(step as usize) {
        for x in (step / 2..grid.wide - 4).step_by(step as usize) {
            tree(x, z);
        }
    }
    let (x, z) = (grid.wide * 3 / 4, grid.tall / 4);
    let top = grid.deep - ground(x, z, sides) - 1;
    for dz in 0..3 {
        for dx in 0..3 {
            cells[Solid::cell(grid, [x + dx, top, z + dz]) as usize] =
                nature::cell(nature::FIRE, 900);
            for up in 1..4 {
                cells[Solid::cell(grid, [x + dx, top - up, z + dz]) as usize] =
                    nature::cell(nature::SMOKE, 400);
            }
        }
    }
    cells
}

struct Drawn<'h> {
    held: Resident<'h>,
    build: Build,
    stages: Vec<Vec<Groups>>,
    still: Vec<Vec<Groups>>,
}

fn kinds() -> Kinds {
    Kinds::named("nature").unwrap()
}

fn grown(mut sizes: Vec<usize>, build: &Build) -> Vec<usize> {
    if let Some(held) = sizes.get_mut(PIXELS as usize) {
        *held = (*held).max(build.layout().words());
    }
    sizes
}

fn zero(held: &mut Resident<'_>, sizes: &[usize]) {
    let zeros = vec![0_u32; 1 << 22];
    for (binding, &words) in sizes.iter().enumerate() {
        let mut at = 0;
        while at < words {
            let run = zeros.len().min(words - at);
            held.write_at(binding, at, &zeros[..run]).unwrap();
            at += run;
        }
    }
}

fn still(stages: &[Vec<Groups>]) -> Vec<Vec<Groups>> {
    stages
        .iter()
        .map(|stage| stage.iter().map(|_| Groups::Fixed(0)).collect())
        .collect()
}

fn dense<'h>(host: &'h Host, grid: Grid, cells: &[u32]) -> Drawn<'h> {
    let width = host.width();
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap();
    let windows = plan.windows();
    let build = Build {
        grid,
        kinds: kinds(),
        screen: SCREEN,
        subgroup: width,
        suns: 2,
        windows,
        sparse: None,
    };
    let engine = live::wired3_by(&Nature, width, grid, &plan).unwrap();
    let (passes, stages) = render3::watching(engine, build.passes().unwrap()).unwrap();
    let sizes = windows.laid(&build.layout().sized(live::sizes3(&Nature, &plan)));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    zero(&mut held, &sizes);
    live::write_state(&mut held, windows, 0, cells).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    held.write_at(PIXELS as usize, build.layout().table(), &table().words())
        .unwrap();
    let still = still(&stages);
    Drawn {
        held,
        build,
        stages,
        still,
    }
}

fn pooled(width: u32, world: Grid) -> Option<(Plan, Bricks)> {
    let plan = Plan::bricked(width, world.blocks().div_ceil(sweep::cells(width))).ok()?;
    let bricks = Bricks::new(world, &plan, plan.blocks).ok()?;
    Some((plan, bricks))
}

fn sparse<'h>(
    host: &'h Host,
    world: Grid,
    window: Grid,
    corner: [u32; 3],
    cells: &[u32],
) -> Drawn<'h> {
    let width = host.width();
    let (plan, bricks) = pooled(width, world).unwrap();
    let reading = Sparse::of(&bricks);
    let build = Build {
        grid: window,
        kinds: kinds(),
        screen: SCREEN,
        subgroup: width,
        suns: 2,
        windows: bricks.windows(),
        sparse: Some(reading),
    };
    let engine = grit::bricks::arranged(&Nature, &bricks, &plan, true).unwrap();
    let (passes, stages) = render3::watching_settled(engine, build.passes().unwrap()).unwrap();
    let sizes = reading.sized(grown(bricks.sizes(&Nature, &plan), &build));
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    zero(&mut held, &sizes);
    held.write(PARAMS as usize, &[u32::MAX]).unwrap();
    bricks.write(&mut held, &plan, cells).unwrap();
    bricks.seed(&mut held, &plan).unwrap();
    held.write_at(reading.table as usize, reading.origin as usize, &corner)
        .unwrap();
    held.write_at(PIXELS as usize, build.layout().table(), &table().words())
        .unwrap();
    let still = still(&stages);
    Drawn {
        held,
        build,
        stages,
        still,
    }
}

impl Drawn<'_> {
    fn frame(&mut self, moving: bool, first: bool) -> Vec<u32> {
        let grid = self.build.grid;
        let side = grid.wide as f32;
        let camera = Camera::looking(
            [side * 1.3, -(grid.deep as f32) * 0.6, side * 1.4],
            [side * 0.5, grid.deep as f32 * 0.7, side * 0.5],
            VIEW,
        );
        let light = Light::evening(SCREEN.high, VIEW);
        let frame = Frame {
            suns: light.sun_rays(2, 0.02),
            floor: 0.07,
            before: camera.before(grid, SCREEN),
            flags: render3::flags::RESET,
            drawn: 0,
            top: 0.0,
        };
        let words = framed_words(&camera.rays(grid, SCREEN), &table(), &frame, &light);
        self.held
            .write_at(PIXELS as usize, self.build.layout().scene(), &words)
            .unwrap();
        let mut all = if moving {
            self.stages.clone()
        } else {
            self.still.clone()
        };
        all.extend(self.build.stages(Steps {
            refill: first,
            ..Steps::ALL
        }));
        let groups: Vec<&[Groups]> = all.iter().map(Vec::as_slice).collect();
        let layout = self.build.layout();
        let (_, colours) = self
            .held
            .staged_reading(&groups, PIXELS as usize, SCREEN.pixels())
            .unwrap();
        let hits = self
            .held
            .read(PIXELS as usize, layout.fog())
            .unwrap()
            .split_off(layout.hits());
        colours.into_iter().chain(hits).collect()
    }
}

fn first_difference(label: &str, got: &[u32], want: &[u32]) -> Option<String> {
    if got.len() != want.len() {
        return Some(format!(
            "{label}: {} words, {} wanted",
            got.len(),
            want.len()
        ));
    }
    let wrong = got.iter().zip(want).filter(|(a, b)| a != b).count();
    let at = got.iter().zip(want).position(|(a, b)| a != b)?;
    Some(format!(
        "{label}: {wrong} of {} words differ, first at {at}: {:#010x}, the dense picture {:#010x}",
        got.len(),
        got[at],
        want[at]
    ))
}

fn largest(width: u32) -> Grid {
    for [wide, deep, tall] in [[256, 256, 256], [256, 128, 256], [128, 128, 128]] {
        let grid = Grid::solid(wide, deep, tall).unwrap();
        if pooled(width, grid).is_some() {
            return grid;
        }
    }
    panic!("no world of 128 cells a side is held by a pool at width {width}");
}

#[test]
fn a_sparse_world_draws_the_dense_picture_word_for_word() {
    let Some(host) = device("sparse3") else {
        return;
    };
    let grid = largest(host.width());
    let cells = world(grid);
    let mut want = dense(&host, grid, &cells);
    let mut got = sparse(&host, grid, grid, [0; 3], &cells);
    let frames = 4;
    let mut lit = 0;
    for at in 0..frames {
        let wanted = want.frame(true, at == 0);
        let given = got.frame(true, at == 0);
        let label = format!(
            "{} width {} {}x{}x{} frame {at}",
            host.name(),
            host.width(),
            grid.wide,
            grid.deep,
            grid.tall
        );
        if let Some(why) = first_difference(&label, &given, &wanted) {
            panic!("{why}");
        }
        lit = wanted[..SCREEN.pixels()]
            .iter()
            .filter(|&&pixel| pixel != wanted[0])
            .count();
    }
    assert!(lit > SCREEN.pixels() / 4, "the picture shows little: {lit}");
    println!(
        "{} width {}: {}x{}x{} dense and sparse pictures equal word for word over {frames} frames, {lit} pixels unlike the corner",
        host.name(),
        host.width(),
        grid.wide,
        grid.deep,
        grid.tall
    );
}

#[test]
fn a_window_onto_a_wider_sparse_world_draws_the_cells_it_holds() {
    let Some(host) = device("sparse3") else {
        return;
    };
    let width = host.width();
    let window = Grid::solid(64, 64, 64).unwrap();
    let world = Grid::solid(128, 64, 128).unwrap();
    let Some((_, bricks)) = pooled(width, world) else {
        panic!("no pool holds a world of 128x64x128 at width {width}");
    };
    let reading = Sparse::of(&bricks);
    let cells = self::world(world);
    for middle in [[96.0, 32.0, 48.0], [8.0, 32.0, 120.0]] {
        let corner = reading.corner(window, middle);
        let mut inside = vec![0_u32; window.cells()];
        for z in 0..window.tall {
            for y in 0..window.deep {
                for x in 0..window.wide {
                    let at = [
                        (x + corner[0]) % world.wide,
                        (y + corner[1]) % world.deep,
                        (z + corner[2]) % world.tall,
                    ];
                    inside[Solid::cell(window, [x, y, z]) as usize] =
                        cells[Solid::cell(world, at) as usize];
                }
            }
        }
        let mut want = dense(&host, window, &inside);
        let mut got = sparse(&host, world, window, corner, &cells);
        let wanted = want.frame(false, true);
        let given = got.frame(false, true);
        let label = format!(
            "{} width {width} window at {corner:?} of {}x{}x{}",
            host.name(),
            world.wide,
            world.deep,
            world.tall
        );
        if let Some(why) = first_difference(&label, &given, &wanted) {
            panic!("{why}");
        }
    }
}

#[test]
fn a_window_is_placed_on_whole_bricks_and_wraps_around_the_world() {
    let world = Grid::solid(128, 64, 256).unwrap();
    let plan = Plan::bricked(8, world.blocks().div_ceil(sweep::cells(8))).unwrap();
    let bricks = Bricks::new(world, &plan, 4).unwrap();
    let reading = Sparse::of(&bricks);
    let window = Grid::solid(64, 64, 64).unwrap();
    assert!(reading.fits(window));
    assert!(!reading.fits(Grid::solid(256, 64, 64).unwrap()));
    let sides = bricks.sides();
    for middle in [
        [0.0, 0.0, 0.0],
        [33.0, 10.0, 250.0],
        [-40.0, 32.0, 300.0],
        [127.9, 63.0, 129.0],
    ] {
        let corner = reading.corner(window, middle);
        for axis in 0..3 {
            assert_eq!(corner[axis] % sides[axis], 0, "{middle:?} gives {corner:?}");
        }
        assert_eq!(
            corner[1], 0,
            "a window as deep as the world starts at its top"
        );
        assert!(corner[0] < world.wide && corner[2] < world.tall);
    }
    assert_eq!(reading.corner(window, [64.0, 32.0, 32.0]), [32, 0, 0]);
    assert_eq!(reading.corner(window, [0.0, 32.0, 0.0]), [96, 0, 224]);
    let build = Build {
        grid: Grid::solid(256, 64, 64).unwrap(),
        kinds: kinds(),
        screen: SCREEN,
        subgroup: 8,
        suns: 1,
        windows: bricks.windows(),
        sparse: Some(reading),
    };
    let refused = render3::primary(&build).unwrap_err().to_string();
    assert!(refused.contains(render3::UNFIT_SPARSE), "{refused}");
}
