#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::type_complexity
)]

mod baking;

use baking::{Mesh, Node, Scene, cuboid, edited, glb, node, one, sphere, torus};
use grit::bake::{self, Asset, Error, Map, Model, Options};
use grit::rule3::nature;

fn swapped(mut mesh: Mesh, swap: impl Fn([f32; 3]) -> [f32; 3]) -> Mesh {
    for point in &mut mesh.points {
        *point = swap(*point);
    }
    mesh
}

fn nature_map(text: &str) -> Map {
    Map::read(text.as_bytes()).unwrap()
}

const ROCKS: &str = r#"{"rule":"nature","heat":300,"default":"ROCK"}"#;

fn baked(scene: &Scene, map: &Map, size: f64) -> Asset {
    let model = bake::read_glb(&glb(scene)).unwrap();
    bake::bake(&model, map, Options { size, scale: 1.0 }).unwrap()
}

/// The lattice the baker should use for `meshes`: the least corner of the
/// bounds of the vertices its triangles use, in grid axes, y flipped, and
/// the count of centres along each.
fn lattice_of(meshes: &[Mesh], size: f64) -> ([f64; 3], [u32; 3]) {
    let grid: Vec<[f64; 3]> = meshes
        .iter()
        .flat_map(|mesh| mesh.indices.iter().map(|&at| mesh.points[at as usize]))
        .map(|p| [f64::from(p[0]), -f64::from(p[1]), f64::from(p[2])])
        .collect();
    let low = [0, 1, 2].map(|axis| grid.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min));
    let high = [0, 1, 2].map(|axis| {
        grid.iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max)
    });
    let count = [0, 1, 2].map(|axis| ((high[axis] - low[axis]) / size - 0.5).floor() as u32 + 1);
    (low, count)
}

/// Every voxel of `asset` with its centre in model axes (y up again).
fn centres(asset: &Asset, low: [f64; 3], size: f64) -> Vec<([f64; 3], Option<u32>)> {
    let [wide, deep, tall] = asset.size;
    let mut out = Vec::new();
    for z in 0..tall {
        for y in 0..deep {
            for x in 0..wide {
                let centre = [x, y, z].map(|at| (f64::from(at) + 0.5) * size);
                let grid = [low[0] + centre[0], low[1] + centre[1], low[2] + centre[2]];
                out.push(([grid[0], -grid[1], grid[2]], asset.at([x, y, z]).unwrap()));
            }
        }
    }
    out
}

fn length(point: [f64; 3]) -> f64 {
    (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt()
}

/// The least distance from the origin to the plane of any triangle of a
/// convex mesh about the origin: the radius of the ball it holds.
fn inner_radius(mesh: &Mesh) -> f64 {
    mesh.indices
        .chunks(3)
        .map(|three| {
            let [a, b, c] = [0, 1, 2].map(|at| mesh.points[three[at] as usize].map(f64::from));
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let dot = normal[0] * a[0] + normal[1] * a[1] + normal[2] * a[2];
            dot.abs() / length(normal)
        })
        .fold(f64::INFINITY, f64::min)
}

/// What a bake did against what the shape's bounds demand: the voxels
/// filled, the voxels that must be filled, the voxels that may be, and how
/// many voxels break the bounds.
#[derive(Debug)]
struct Held {
    filled: usize,
    least: usize,
    most: usize,
    wrong: usize,
}

/// Holds every voxel of `asset` to the bounds: inside when `surely_in`,
/// outside when `surely_out`.
fn held(
    asset: &Asset,
    low: [f64; 3],
    size: f64,
    surely_in: impl Fn([f64; 3]) -> bool,
    surely_out: impl Fn([f64; 3]) -> bool,
) -> Held {
    let mut held = Held {
        filled: 0,
        least: 0,
        most: 0,
        wrong: 0,
    };
    for (centre, cell) in centres(asset, low, size) {
        let must = surely_in(centre);
        let may = !surely_out(centre);
        held.least += usize::from(must);
        held.most += usize::from(may);
        held.filled += usize::from(cell.is_some());
        if (must && cell.is_none()) || (!may && cell.is_some()) {
            held.wrong += 1;
        }
    }
    held
}

#[test]
fn a_unit_cube_bakes_to_exactly_n_cubed_voxels_at_every_size() {
    let map = nature_map(ROCKS);
    let rock = nature::cell(nature::ROCK, 300);
    let low = [0.25, -1.5, 2.0];
    let high = [1.25, -0.5, 3.0];
    for (size, side) in [
        (1.0, 1),
        (0.5, 2),
        (1.0 / 3.0, 3),
        (0.25, 4),
        (0.2, 5),
        (0.125, 8),
        (1.0 / 7.0, 7),
        (0.0625, 16),
        (0.3, 3),
        (0.45, 2),
        (1.5, 1),
    ] {
        let asset = baked(&one(cuboid(low, high, None), vec![]), &map, size);
        assert_eq!(asset.size, [side; 3], "voxel side {size}");
        assert_eq!(
            asset.filled(),
            (side * side * side) as usize,
            "voxel side {size}: the tolerance is zero voxels"
        );
        assert!(asset.cells.iter().flatten().all(|&word| word == rock));
    }
}

#[test]
fn the_scale_multiplies_the_model_before_the_voxel_side_divides_it() {
    let map = nature_map(ROCKS);
    let cube = one(cuboid([0.0; 3], [2.0; 3], None), vec![]);
    let model = bake::read_glb(&glb(&cube)).unwrap();
    for (scale, size, side) in [(0.5, 0.25, 4), (2.0, 1.0, 4), (3.0, 1.5, 4), (0.25, 0.5, 1)] {
        let asset = bake::bake(&model, &map, Options { size, scale }).unwrap();
        assert_eq!(asset.size, [side; 3], "scale {scale} side {size}");
        assert_eq!(asset.filled(), (side * side * side) as usize);
    }
}

#[test]
fn a_sphere_bakes_between_its_inner_and_outer_ball_at_every_size() {
    let map = nature_map(ROCKS);
    for (segments, stacks) in [(48, 24), (31, 17)] {
        let mesh = sphere(1.0, segments, stacks, false);
        let inner = inner_radius(&mesh);
        let outer = mesh
            .points
            .iter()
            .map(|p| length(p.map(f64::from)))
            .fold(0.0, f64::max);
        for size in [1.0 / 3.0, 0.2, 0.125, 1.0 / 12.0, 0.0625] {
            let asset = baked(&one(mesh.clone(), vec![]), &map, size);
            let (low, count) = lattice_of(std::slice::from_ref(&mesh), size);
            assert_eq!(asset.size, count);
            let held = held(
                &asset,
                low,
                size,
                |at| length(at) < inner,
                |at| length(at) > outer,
            );
            println!(
                "sphere {segments}x{stacks} voxel side {size}: {} filled, bounds {}..={}, ball {:.1}",
                held.filled,
                held.least,
                held.most,
                4.0 / 3.0 * std::f64::consts::PI / size.powi(3)
            );
            assert_eq!(held.wrong, 0, "voxel side {size}: {held:?}");
            assert!((held.least..=held.most).contains(&held.filled));
            assert!(held.least > 0);
        }
    }
}

/// A sphere with a hole, between two cubes on the axis through the hole
/// that stretch the box past it. Each line through the hole misses a
/// crossing, so parity along that axis empties the sphere along it and
/// fills the gap between the sphere and a cube. The other two axes' lines
/// miss the hole and outvote it.
#[test]
fn a_sphere_with_a_hole_still_bakes_solid_by_the_vote_of_three_axes() {
    let map = nature_map(ROCKS);
    let inner = inner_radius(&sphere(1.0, 48, 24, false));
    let (near, far, thick) = (2.45, 3.0, 0.27);
    let in_cube =
        |p: [f64; 3]| p[0].abs() < thick && p[2].abs() < thick && (near..far).contains(&p[1].abs());
    let on_cube = |p: [f64; 3]| {
        p[0].abs() <= thick && p[2].abs() <= thick && (near..=far).contains(&p[1].abs())
    };
    let holes: [(&str, fn([f32; 3]) -> [f32; 3], fn([f64; 3]) -> [f64; 3]); 3] = [
        ("top, y", |p| p, |p| p),
        ("side, x", |p| [p[1], p[0], p[2]], |p| [p[1], p[0], p[2]]),
        ("front, z", |p| [p[0], p[2], p[1]], |p| [p[0], p[2], p[1]]),
    ];
    for (hole, swap, back) in holes {
        let scene = Scene {
            meshes: vec![
                swapped(sphere(1.0, 48, 24, true), swap),
                swapped(cuboid([-0.27, 2.45, -0.27], [0.27, 3.0, 0.27], None), swap),
                swapped(
                    cuboid([-0.27, -3.0, -0.27], [0.27, -2.45, 0.27], None),
                    swap,
                ),
            ],
            materials: vec![],
            nodes: (0..3).map(node).collect(),
        };
        for size in [0.2, 0.125, 0.0625] {
            let asset = baked(&scene, &map, size);
            let (low, _) = lattice_of(&scene.meshes, size);
            let held = held(
                &asset,
                low,
                size,
                |at| length(at) < inner || in_cube(back(at)),
                |at| length(at) > 1.0 && !on_cube(back(at)),
            );
            println!(
                "sphere open at the {hole} voxel side {size}: {} filled, bounds {}..={}, {} wrong",
                held.filled, held.least, held.most, held.wrong
            );
            assert_eq!(held.wrong, 0, "open at the {hole}, side {size}: {held:?}");
        }
    }
}

#[test]
fn a_torus_keeps_its_hole() {
    let map = nature_map(ROCKS);
    let (major, minor) = (1.0_f64, 0.4_f64);
    let (around, across) = (64, 32);
    let mesh = torus(major as f32, minor as f32, around, across);
    let bend = |steps: u32| 1.0 - (std::f64::consts::PI / f64::from(steps)).cos();
    let slack = (major + minor) * bend(around) + minor * bend(across) + 1e-6;
    let tube = |at: [f64; 3]| {
        let reach = (at[0] * at[0] + at[2] * at[2]).sqrt();
        ((reach - major).powi(2) + at[1] * at[1]).sqrt()
    };
    for size in [0.125, 1.0 / 12.0, 0.0625, 0.05] {
        let asset = baked(&one(mesh.clone(), vec![]), &map, size);
        let (low, _) = lattice_of(std::slice::from_ref(&mesh), size);
        let held = held(
            &asset,
            low,
            size,
            |at| tube(at) < minor - slack,
            |at| tube(at) > minor + slack,
        );
        assert_eq!(held.wrong, 0, "voxel side {size}: {held:?}");
        let hole: Vec<Option<u32>> = centres(&asset, low, size)
            .into_iter()
            .filter(|(at, _)| (at[0] * at[0] + at[2] * at[2]).sqrt() < major - minor - slack)
            .map(|(_, cell)| cell)
            .collect();
        println!(
            "torus voxel side {size}: {} filled, bounds {}..={}, {} voxels in the hole",
            held.filled,
            held.least,
            held.most,
            hole.len()
        );
        assert!(
            hole.len() >= 8,
            "voxel side {size}: the hole holds no voxel"
        );
        assert!(
            hole.iter().all(Option::is_none),
            "voxel side {size}: the hole is filled"
        );
    }
}

const TREE: &str = r#"{"rule":"nature","heat":300,"default":"ROCK","materials":{"bark":"WOOD","needles":"leaves","stone":"Rock"}}"#;

#[test]
fn a_two_material_model_maps_each_part_to_its_kind() {
    let map = nature_map(TREE);
    let wood = nature::cell(nature::WOOD, 300);
    let leaves = nature::cell(nature::LEAVES, 300);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0, 0.0, 0.0], [1.0, 2.0, 1.0], Some(0)),
            cuboid([-0.5, 0.0, -0.5], [1.5, 1.0, 1.5], Some(1)),
        ],
        materials: vec![Some("bark"), Some("needles")],
        nodes: vec![
            node(0),
            Node {
                mesh: 1,
                transform: r#","translation":[0,2,0]"#.to_owned(),
            },
        ],
    };
    let size = 0.25;
    let asset = baked(&scene, &map, size);
    assert_eq!(asset.size, [8, 12, 8]);
    let mut counts = [0; 2];
    for z in 0..8 {
        for y in 0..12 {
            for x in 0..8 {
                let cell = asset.at([x, y, z]).unwrap();
                let trunk = (2..6).contains(&x) && (4..12).contains(&y) && (2..6).contains(&z);
                let crown = y < 4;
                let want = if crown {
                    Some(leaves)
                } else if trunk {
                    Some(wood)
                } else {
                    None
                };
                assert_eq!(cell, want, "cell ({x}, {y}, {z})");
                counts[0] += usize::from(cell == Some(wood));
                counts[1] += usize::from(cell == Some(leaves));
            }
        }
    }
    assert_eq!(counts, [4 * 8 * 4, 8 * 4 * 8]);
}

#[test]
fn where_parts_overlap_the_later_part_wins() {
    let map = nature_map(TREE);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0; 3], Some(0)),
            cuboid([0.5, 0.0, 0.0], [1.0; 3], Some(1)),
        ],
        materials: vec![Some("bark"), Some("stone")],
        nodes: vec![node(0), node(1)],
    };
    let asset = baked(&scene, &map, 0.25);
    for z in 0..4 {
        for y in 0..4 {
            for x in 0..4 {
                let kind = nature::kind_of(asset.at([x, y, z]).unwrap().unwrap());
                assert_eq!(kind, if x < 2 { nature::WOOD } else { nature::ROCK });
            }
        }
    }
}

#[test]
fn node_transforms_move_rotate_and_scale_their_parts() {
    let map = nature_map(TREE);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0; 3], Some(0)),
            cuboid([0.0; 3], [2.0, 1.0, 1.0], Some(1)),
        ],
        materials: vec![Some("bark"), Some("needles")],
        nodes: vec![
            node(0),
            Node {
                mesh: 1,
                transform: format!(r#","translation":[5,0,0],"rotation":[0,{half},0,{half}]"#),
            },
        ],
    };
    let asset = baked(&scene, &map, 0.5);
    assert_eq!(asset.size, [12, 2, 6], "x 0..6, y 0..1, z -2..1");
    let leaves = |x: u32, z: u32| (10..12).contains(&x) && z < 4;
    let wood = |x: u32, z: u32| x < 2 && (4..6).contains(&z);
    for z in 0..6 {
        for x in 0..12 {
            for y in 0..2 {
                let kind = asset.at([x, y, z]).unwrap().map(nature::kind_of);
                let want = if leaves(x, z) {
                    Some(nature::LEAVES)
                } else if wood(x, z) {
                    Some(nature::WOOD)
                } else {
                    None
                };
                assert_eq!(kind, want, "cell ({x}, {y}, {z})");
            }
        }
    }

    let scaled = Scene {
        nodes: vec![
            node(0),
            Node {
                mesh: 0,
                transform: r#","matrix":[2,0,0,0, 0,1,0,0, 0,0,1,0, 3,0,0,1]"#.to_owned(),
            },
        ],
        ..scene
    };
    let asset = baked(&scaled, &map, 0.5);
    assert_eq!(asset.size, [10, 2, 2]);
    let row: Vec<bool> = (0..10)
        .map(|x| asset.at([x, 1, 1]).unwrap().is_some())
        .collect();
    assert_eq!(
        row,
        [
            true, true, false, false, false, false, true, true, true, true
        ]
    );
}

#[test]
fn the_grid_points_y_down_so_the_top_of_a_model_is_its_first_row() {
    let map = nature_map(TREE);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0; 3], Some(0)),
            cuboid([0.0, 1.0, 0.0], [1.0, 2.0, 1.0], Some(1)),
        ],
        materials: vec![Some("bark"), Some("needles")],
        nodes: vec![node(0), node(1)],
    };
    let asset = baked(&scene, &map, 1.0);
    assert_eq!(asset.size, [1, 2, 1]);
    assert_eq!(
        asset.at([0, 0, 0]).unwrap().map(nature::kind_of),
        Some(nature::LEAVES)
    );
    assert_eq!(
        asset.at([0, 1, 0]).unwrap().map(nature::kind_of),
        Some(nature::WOOD)
    );
    assert_eq!(asset.anchor(), [0, 1, 0]);
}

#[test]
fn a_missing_unnamed_or_unmapped_material_takes_the_default() {
    let map = nature_map(TREE);
    let rock = nature::cell(nature::ROCK, 300);
    let wood = nature::cell(nature::WOOD, 300);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0; 3], None),
            cuboid([1.0, 0.0, 0.0], [2.0, 1.0, 1.0], Some(0)),
            cuboid([2.0, 0.0, 0.0], [3.0, 1.0, 1.0], Some(1)),
            cuboid([3.0, 0.0, 0.0], [4.0, 1.0, 1.0], Some(2)),
        ],
        materials: vec![None, Some("glass"), Some("bark")],
        nodes: (0..4).map(node).collect(),
    };
    let asset = baked(&scene, &map, 1.0);
    let row: Vec<Option<u32>> = (0..4).map(|x| asset.at([x, 0, 0]).unwrap()).collect();
    assert_eq!(row, [Some(rock), Some(rock), Some(rock), Some(wood)]);
    assert_eq!(map.word(None), rock);
    assert_eq!(map.word(Some("glass")), rock);
    assert_eq!(map.word(Some("bark")), wood);
}

#[test]
fn a_map_is_refused_by_name() {
    let refused = |text: &str| Map::read(text.as_bytes()).unwrap_err();
    assert_eq!(refused(r#"{"rule":"nature"}"#), Error::NoDefault);
    assert!(matches!(
        refused(r#"{"rule":"lava","default":0}"#),
        Error::UnknownRule { asked, .. } if asked == "lava"
    ));
    assert!(matches!(
        refused(r#"{"default":"MAGMA"}"#),
        Error::UnknownWord { word, .. } if word == "MAGMA"
    ));
    assert!(matches!(
        refused(r#"{"rule":"sand","default":"ROCK"}"#),
        Error::UnknownWord { rule, .. } if rule == "sand"
    ));
    assert!(matches!(
        refused(r#"{"default":4294967296}"#),
        Error::UnknownWord { .. }
    ));
    assert!(matches!(
        refused(r#"{"default":"ROCK","heat":65536}"#),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        refused(r#"{"default":"ROCK","materials":{"a":"WOOD","a":"ROCK"}}"#),
        Error::Malformed { .. }
    ));
    assert!(matches!(refused(r#"{"default":"ROCK""#), Error::Json(_)));
    assert!(matches!(refused("[1]"), Error::Malformed { .. }));

    let map = nature_map(r#"{"rule":"sand","default":3,"materials":{"x":4294967295}}"#);
    assert_eq!((map.default, map.word(Some("x"))), (3, u32::MAX));
    let map = nature_map(r#"{"default":"steam"}"#);
    assert_eq!(map.default, nature::cell(nature::STEAM, 0));
}

#[test]
fn a_material_map_can_name_every_kind_the_nature_rule_has_char_among_them() {
    assert_eq!(bake::NATURE_KINDS.len() as u32, nature::KINDS);
    let expected = [
        ("AIR", nature::AIR),
        ("ROCK", nature::ROCK),
        ("EARTH", nature::EARTH),
        ("SAND", nature::SAND),
        ("SNOW", nature::SNOW),
        ("ICE", nature::ICE),
        ("WATER", nature::WATER),
        ("WOOD", nature::WOOD),
        ("LEAVES", nature::LEAVES),
        ("FIRE", nature::FIRE),
        ("ASH", nature::ASH),
        ("SMOKE", nature::SMOKE),
        ("STEAM", nature::STEAM),
        ("CHAR", nature::CHAR),
    ];
    for (at, (name, kind)) in expected.into_iter().enumerate() {
        assert_eq!(bake::NATURE_KINDS.get(at).copied(), Some(name));
        let text = format!(r#"{{"default":"{}","heat":200}}"#, name.to_lowercase());
        let map = nature_map(&text);
        assert_eq!(map.default, nature::cell(kind, 200), "{name}");
        assert_eq!(nature::kind_of(map.default), kind, "{name}");
    }
    let burnt = nature_map(r#"{"default":"ROCK","materials":{"trunk":"Char"}}"#);
    assert_eq!(burnt.word(Some("trunk")), nature::cell(nature::CHAR, 0));
    assert!(matches!(
        Map::read(br#"{"default":"CHARCOAL"}"#),
        Err(Error::UnknownWord { word, .. }) if word == "CHARCOAL"
    ));
}

fn l_shape() -> Asset {
    let size = [3, 2, 2];
    let mut cells = vec![None; 12];
    for (at, cell) in cells.iter_mut().enumerate() {
        if at % 5 != 3 {
            *cell = Some(at as u32 + 100);
        }
    }
    Asset::new(size, cells).unwrap()
}

#[test]
fn rotation_and_anchor_place_exactly() {
    let asset = l_shape();
    assert_eq!(asset.anchor(), [1, 1, 1]);
    let world = [16, 16, 16];
    let index = |[x, y, z]: [u32; 3]| (z * 16 + y) * 16 + x;
    let word = |[x, y, z]: [u32; 3]| asset.at([x, y, z]).unwrap().unwrap();
    let at = [5, 6, 7];
    let expected: [[([u32; 3], [u32; 3]); 4]; 4] = [
        [
            ([1, 1, 1], [5, 6, 7]),
            ([2, 1, 1], [6, 6, 7]),
            ([0, 0, 0], [4, 5, 6]),
            ([2, 0, 0], [6, 5, 6]),
        ],
        [
            ([1, 1, 1], [5, 6, 7]),
            ([2, 1, 1], [5, 6, 6]),
            ([0, 0, 0], [4, 5, 8]),
            ([2, 0, 0], [4, 5, 6]),
        ],
        [
            ([1, 1, 1], [5, 6, 7]),
            ([2, 1, 1], [4, 6, 7]),
            ([0, 0, 0], [6, 5, 8]),
            ([2, 0, 0], [4, 5, 8]),
        ],
        [
            ([1, 1, 1], [5, 6, 7]),
            ([2, 1, 1], [5, 6, 8]),
            ([0, 0, 0], [6, 5, 6]),
            ([2, 0, 0], [6, 5, 8]),
        ],
    ];
    for (turns, pairs) in expected.iter().enumerate() {
        let placed = asset.placed(world, at, turns as u32).unwrap();
        assert_eq!(placed.len(), asset.filled());
        for (cell, landed) in pairs {
            assert!(
                placed.contains(&(index(*landed), word(*cell))),
                "turn {turns}: cell {cell:?} should land on {landed:?}"
            );
        }
        let again = asset.placed(world, at, turns as u32 + 4).unwrap();
        assert_eq!(again, placed, "four turns are none");
    }
    let mut seen = std::collections::BTreeSet::new();
    for turns in 0..4 {
        let mut cells: Vec<u32> = asset
            .placed(world, at, turns)
            .unwrap()
            .into_iter()
            .map(|(cell, _)| cell)
            .collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(
            cells.len(),
            asset.filled(),
            "turn {turns} lands two cells on one"
        );
        seen.insert(cells);
    }
    assert_eq!(seen.len(), 4, "every turn places differently");
}

#[test]
fn a_placement_past_the_world_is_refused_and_one_at_its_edge_fits() {
    let asset = l_shape();
    let world = [16, 16, 16];
    assert!(asset.placed(world, [1, 1, 1], 0).is_ok());
    assert!(asset.placed(world, [14, 15, 14], 0).is_ok());
    assert!(asset.placed(world, [1, 15, 1], 3).is_ok());
    for (at, turns) in [
        ([0, 1, 1], 0),
        ([1, 0, 1], 0),
        ([1, 1, 0], 0),
        ([15, 1, 1], 0),
        ([1, 1, 15], 2),
        ([1, 1, 15], 1),
        ([1, 16, 1], 0),
        ([u32::MAX, 1, 1], 0),
    ] {
        assert!(
            matches!(asset.placed(world, at, turns), Err(Error::Outside { .. })),
            "at {at:?} turned {turns}"
        );
    }
}

#[test]
fn a_baked_model_turns_about_its_base_centre() {
    let map = nature_map(TREE);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0, 3.0, 1.0], Some(0)),
            cuboid([1.0, 0.0, 0.0], [3.0, 1.0, 1.0], Some(1)),
        ],
        materials: vec![Some("bark"), Some("needles")],
        nodes: vec![node(0), node(1)],
    };
    let asset = baked(&scene, &map, 1.0);
    assert_eq!(asset.size, [3, 3, 1]);
    assert_eq!(asset.anchor(), [1, 2, 0]);
    let world = [8, 8, 8];
    let kinds = |turns: u32| -> Vec<(u32, u32)> {
        let mut placed: Vec<(u32, u32)> = asset
            .placed(world, [4, 7, 4], turns)
            .unwrap()
            .into_iter()
            .map(|(cell, word)| (cell, nature::kind_of(word)))
            .collect();
        placed.sort_unstable();
        placed
    };
    let at = |x: u32, y: u32, z: u32| (z * 8 + y) * 8 + x;
    let mut want = vec![
        (at(3, 5, 4), nature::WOOD),
        (at(3, 6, 4), nature::WOOD),
        (at(3, 7, 4), nature::WOOD),
        (at(4, 7, 4), nature::LEAVES),
        (at(5, 7, 4), nature::LEAVES),
    ];
    want.sort_unstable();
    assert_eq!(kinds(0), want);
    let mut want = vec![
        (at(4, 5, 5), nature::WOOD),
        (at(4, 6, 5), nature::WOOD),
        (at(4, 7, 5), nature::WOOD),
        (at(4, 7, 4), nature::LEAVES),
        (at(4, 7, 3), nature::LEAVES),
    ];
    want.sort_unstable();
    assert_eq!(kinds(1), want, "a quarter turn takes +x to -z");
}

#[test]
fn an_asset_file_holds_the_asset_exactly() {
    let map = nature_map(TREE);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0, 2.0, 1.0], Some(0)),
            sphere(1.0, 24, 12, false),
        ],
        materials: vec![Some("bark")],
        nodes: vec![
            node(0),
            Node {
                mesh: 1,
                transform: r#","translation":[0.5,3,0.5]"#.to_owned(),
            },
        ],
    };
    let asset = baked(&scene, &map, 0.125);
    let file = asset.write();
    assert_eq!(&file[..4], b"GRVX");
    assert_eq!(Asset::read(&file).unwrap(), asset);
    assert!(
        file.len() < 4 * asset.cells.len(),
        "the runs are shorter than plain words"
    );
    for asset in [
        l_shape(),
        Asset::new([1, 1, 1], vec![None]).unwrap(),
        Asset::new([2, 1, 1], vec![Some(0), Some(0)]).unwrap(),
        Asset::new([2, 1, 1], vec![Some(u32::MAX), None]).unwrap(),
    ] {
        assert_eq!(Asset::read(&asset.write()).unwrap(), asset);
    }
}

fn word_put(file: &mut [u8], at: usize, word: u32) {
    file[at..at + 4].copy_from_slice(&word.to_le_bytes());
}

#[test]
fn a_malformed_asset_file_is_refused_by_name() {
    let file = l_shape().write();
    for cut in 0..file.len() {
        assert!(
            matches!(Asset::read(&file[..cut]), Err(Error::Truncated { .. })),
            "cut at {cut}"
        );
    }
    let mut bad = file.clone();
    bad[3] = b'Y';
    assert!(matches!(
        Asset::read(&bad),
        Err(Error::BadAssetMagic { .. })
    ));
    let mut bad = file.clone();
    word_put(&mut bad, 4, 2);
    assert_eq!(Asset::read(&bad), Err(Error::AssetVersion { found: 2 }));
    let mut bad = file.clone();
    bad.push(0);
    assert!(matches!(Asset::read(&bad), Err(Error::Trailing { .. })));
    let mut bad = file.clone();
    word_put(&mut bad, 8, 0);
    assert!(matches!(Asset::read(&bad), Err(Error::TooBig { .. })));
    let mut bad = file.clone();
    word_put(&mut bad, 8, 1 << 20);
    word_put(&mut bad, 12, 1 << 20);
    assert!(matches!(Asset::read(&bad), Err(Error::TooBig { .. })));
    let mut bad = file.clone();
    word_put(&mut bad, 24, bake::FILLED);
    assert!(matches!(Asset::read(&bad), Err(Error::Runs { .. })));
    let mut bad = file.clone();
    word_put(&mut bad, 24, bake::FILLED | 13);
    assert!(matches!(Asset::read(&bad), Err(Error::Runs { .. })));
    let mut bad = file.clone();
    let runs = u32::from_le_bytes(bad[20..24].try_into().unwrap());
    word_put(&mut bad, 20, runs - 1);
    assert!(matches!(
        Asset::read(&bad),
        Err(Error::Runs { .. }) | Err(Error::Trailing { .. })
    ));
    assert!(matches!(
        Asset::new([2, 2, 2], vec![None; 7]),
        Err(Error::Runs { .. })
    ));
}

fn cube_scene() -> Scene {
    one(cuboid([0.0; 3], [1.0; 3], Some(0)), vec![Some("bark")])
}

fn refused(bytes: &[u8]) -> Error {
    bake::read_glb(bytes).unwrap_err()
}

#[test]
fn bad_magic_and_a_wrong_version_are_refused_by_name() {
    let good = glb(&cube_scene());
    let mut bad = good.clone();
    bad[..4].copy_from_slice(b"glTx");
    assert_eq!(refused(&bad), Error::BadMagic { found: *b"glTx" });
    assert_eq!(
        refused(b"PK\x03\x04 not a model"),
        Error::BadMagic {
            found: *b"PK\x03\x04"
        }
    );
    let mut bad = good.clone();
    word_put(&mut bad, 4, 1);
    assert_eq!(refused(&bad), Error::Version { found: 1 });
    let mut bad = good;
    word_put(&mut bad, 16, 0x004E_4942);
    assert_eq!(refused(&bad), Error::NoJson);
}

#[test]
fn every_truncation_of_a_file_is_refused_by_name() {
    let good = glb(&cube_scene());
    assert!(bake::read_glb(&good).is_ok());
    for cut in 0..good.len() {
        assert!(
            matches!(refused(&good[..cut]), Error::Truncated { .. }),
            "cut at {cut}: {:?}",
            bake::read_glb(&good[..cut])
        );
    }
    let mut short = good.clone();
    let declared = good.len() as u32 - 4;
    word_put(&mut short, 8, declared);
    assert!(matches!(refused(&short), Error::Truncated { what, .. } if what == "a chunk"));
}

#[test]
fn an_index_out_of_range_is_refused_by_name() {
    let scene = cube_scene();
    let cases: [(&str, fn(String) -> String); 7] = [
        ("vertex index of mesh 0 primitive 0", |json| json),
        ("material", |json| {
            json.replace(r#""material":0"#, r#""material":7"#)
        }),
        ("mesh", |json| json.replace(r#""mesh":0"#, r#""mesh":3"#)),
        ("node", |json| {
            json.replace(r#""nodes":[0]"#, r#""nodes":[0,9]"#)
        }),
        ("accessor", |json| {
            json.replace(r#""POSITION":0"#, r#""POSITION":2"#)
        }),
        ("buffer view", |json| {
            json.replace(r#""bufferView":1,"#, r#""bufferView":4,"#)
        }),
        ("scene", |json| json.replace(r#""scene":0"#, r#""scene":1"#)),
    ];
    for (what, edit) in cases {
        let bytes = if what.starts_with("vertex") {
            let mut mesh = scene.meshes[0].clone();
            mesh.indices[5] = 8;
            glb(&Scene {
                meshes: vec![mesh],
                ..scene.clone()
            })
        } else {
            edited(&scene, edit)
        };
        match refused(&bytes) {
            Error::OutOfRange { what: named, .. } => assert_eq!(named, what),
            other => panic!("{what}: {other:?}"),
        }
    }
}

#[test]
fn a_view_or_accessor_past_its_bytes_is_refused_by_name() {
    let scene = cube_scene();
    let cases: [(&str, fn(String) -> String); 4] = [
        ("buffer view 1", |json| {
            json.replace(
                r#""byteOffset":96,"byteLength":144}"#,
                r#""byteOffset":96,"byteLength":145}"#,
            )
        }),
        ("accessor 0", |json| {
            json.replace(r#""count":8,"#, r#""count":9,"#)
        }),
        ("accessor 1", |json| {
            json.replace(r#""count":36,"#, r#""count":37,"#)
        }),
        ("buffer 0 in the binary chunk", |json| {
            json.replace(
                r#""buffers":[{"byteLength":240}"#,
                r#""buffers":[{"byteLength":241}"#,
            )
        }),
    ];
    for (what, edit) in cases {
        match refused(&edited(&scene, edit)) {
            Error::Truncated { what: named, .. } => assert_eq!(named, what),
            other => panic!("{what}: {other:?}"),
        }
    }
}

#[test]
fn malformed_json_and_fields_are_refused_by_name() {
    let scene = cube_scene();
    let json = |edit: fn(String) -> String| refused(&edited(&scene, edit));
    assert!(matches!(
        json(|json| json.replace("\"scenes\"", "\"scenes")),
        Error::Json(_)
    ));
    assert!(matches!(json(|json| format!("{json},")), Error::Json(_)));
    assert!(matches!(
        json(|json| json.replace(r#""count":8,"#, r#""count":-8,"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""indices":1"#, r#""indices":1,"mode":9"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""count":36,"#, r#""count":35,"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""componentType":5126"#, r#""componentType":5120"#)),
        Error::Unsupported { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""buffers":[{"#, r#""buffers":[{"uri":"x.bin","#)),
        Error::Unsupported { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""mesh":0"#, r#""mesh":0,"children":[0]"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""mesh":0"#, r#""mesh":0,"rotation":[0,0,0,0]"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""mesh":0"#, r#""mesh":0,"matrix":[1,2]"#)),
        Error::Malformed { .. }
    ));
    assert!(matches!(
        json(|json| json.replace(r#""POSITION":0"#, r#""NORMAL":0"#)),
        Error::Missing { .. }
    ));
    let mut bad = glb(&scene);
    bad[20] = 0xFF;
    assert!(matches!(refused(&bad), Error::Json(_)));
}

#[test]
fn bad_options_and_empty_or_endless_models_are_refused_by_name() {
    let map = nature_map(ROCKS);
    let model = bake::read_glb(&glb(&cube_scene())).unwrap();
    for (size, scale) in [
        (0.0, 1.0),
        (-1.0, 1.0),
        (f64::NAN, 1.0),
        (1.0, 0.0),
        (1.0, f64::INFINITY),
    ] {
        assert!(matches!(
            bake::bake(&model, &map, Options { size, scale }),
            Err(Error::Size { .. })
        ));
    }
    assert!(matches!(
        bake::bake(
            &model,
            &map,
            Options {
                size: 1e-4,
                scale: 1.0
            }
        ),
        Err(Error::TooBig { .. })
    ));
    assert_eq!(
        bake::bake(
            &Model::default(),
            &map,
            Options {
                size: 1.0,
                scale: 1.0
            }
        ),
        Err(Error::Empty)
    );
    let lines = edited(&cube_scene(), |json| {
        json.replace(r#""indices":1"#, r#""indices":1,"mode":1"#)
    });
    let lines = bake::read_glb(&lines).unwrap();
    assert_eq!(
        bake::bake(
            &lines,
            &map,
            Options {
                size: 1.0,
                scale: 1.0
            }
        ),
        Err(Error::Empty)
    );
    let mut endless = cube_scene();
    endless.meshes[0].points[3] = [f32::NAN, 0.0, 0.0];
    let endless = bake::read_glb(&glb(&endless)).unwrap();
    assert_eq!(
        bake::bake(
            &endless,
            &map,
            Options {
                size: 1.0,
                scale: 1.0
            }
        ),
        Err(Error::NotFinite)
    );
}

#[test]
fn strips_fans_and_unindexed_triangles_bake_as_lists_do() {
    let map = nature_map(ROCKS);
    let cube = cuboid([0.0; 3], [1.0; 3], None);
    let listed = baked(&one(cube.clone(), vec![]), &map, 0.25);
    let flat: Vec<[f32; 3]> = cube
        .indices
        .iter()
        .map(|&at| cube.points[at as usize])
        .collect();
    let unindexed = Mesh {
        points: flat.clone(),
        indices: (0..flat.len() as u32).collect(),
        material: None,
    };
    let unindexed = edited(&one(unindexed, vec![]), |json| {
        json.replace(r#","indices":1"#, "")
    });
    let model = bake::read_glb(&unindexed).unwrap();
    assert_eq!(model.parts[0].triangles.len(), 12);
    let asset = bake::bake(
        &model,
        &map,
        Options {
            size: 0.25,
            scale: 1.0,
        },
    )
    .unwrap();
    assert_eq!(asset, listed);

    let fan = Mesh {
        points: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        indices: vec![0, 1, 2, 3],
        material: None,
    };
    for mode in [5, 6] {
        let bytes = edited(&one(fan.clone(), vec![]), |json| {
            json.replace(r#""indices":1"#, &format!(r#""indices":1,"mode":{mode}"#))
        });
        let model = bake::read_glb(&bytes).unwrap();
        assert_eq!(model.parts[0].triangles.len(), 2, "mode {mode}");
    }
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

/// Thousands of files with bytes changed at random: each is read, and
/// baked when it reads, and none panics.
#[test]
fn files_with_random_damage_never_panic() {
    let map = nature_map(TREE);
    let scene = Scene {
        meshes: vec![
            cuboid([0.0; 3], [1.0; 3], Some(0)),
            sphere(0.5, 8, 5, false),
        ],
        materials: vec![Some("bark")],
        nodes: vec![
            node(0),
            Node {
                mesh: 1,
                transform: r#","translation":[0,1.5,0],"scale":[1,2,1]"#.to_owned(),
            },
        ],
    };
    let good = glb(&scene);
    let mut state = 0x5EED;
    let (mut read, mut baked_count) = (0, 0);
    for round in 0..4000 {
        let mut bad = good.clone();
        let flips = 1 + next(&mut state) % 4;
        for _ in 0..flips {
            let at = (next(&mut state) % bad.len() as u64) as usize;
            bad[at] = next(&mut state) as u8;
        }
        if round % 7 == 0 {
            let cut = (next(&mut state) % bad.len() as u64) as usize;
            bad.truncate(cut);
        }
        let outcome = std::panic::catch_unwind(|| {
            let Ok(model) = bake::read_glb(&bad) else {
                return (false, false);
            };
            let small = model
                .parts
                .iter()
                .flat_map(|part| part.triangles.iter().flatten().flatten())
                .all(|value| value.abs() < 8.0);
            let baked = small
                && bake::bake(
                    &model,
                    &map,
                    Options {
                        size: 0.25,
                        scale: 1.0,
                    },
                )
                .is_ok();
            (true, baked)
        });
        let Ok((was_read, was_baked)) = outcome else {
            panic!("round {round} panicked");
        };
        read += usize::from(was_read);
        baked_count += usize::from(was_baked);
    }
    println!("4000 damaged files: {read} read, {baked_count} baked, no panic");
    assert!(
        read > 0,
        "no damaged file read at all: the damage is too coarse to test the reader"
    );
    assert!(baked_count > 0, "no damaged file baked at all");
}
