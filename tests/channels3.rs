#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::branch::format::{self, Law as FileLaw, State};
use grit::branch::{Made, World, sha256};
use grit::bricks::{self, Bricks, Content};
use grit::compact::Plan;
use grit::law::{Code, Law};
use grit::layout::{AWAKE, BINDINGS, Dimension, PARAMS, STATE};
use grit::rule3::Declared;
use grit::sand::Grid;
use grit::sand3::{self, Grid3};
use grit::{Groups, Host, live, sweep};

const SIZE: [u32; 3] = [32, 32, 32];

const FIXTURE: &[u8] = include_bytes!("branch/two-planes.world");
const FIXTURE_SHA: &str = "9172e3b813a1ca7bcbe0b1030646a118c926c274970c277bf00c7b8fe184ec6b";
const FIXTURE_CELLS: &str = "31862445b8790cec88d48f5954cc1ea393e6d4bf542732719fabaee48a08a08d";

const WALL: u32 = 1;

fn marked(word: u32, plane: usize) -> u32 {
    word.wrapping_mul(2 * plane as u32 + 1)
}

fn moving_law(planes: usize) -> Law {
    let swap_plane = format!(
        "[\"map\",\"p\",[\"range\",{planes}],[\"sel\",\"go\",[\"at\",[\"at\",\"c\",\"b\"],\"p\"],[\"at\",[\"at\",\"c\",\"a\"],\"p\"]]]"
    );
    let swap_back = format!(
        "[\"map\",\"p\",[\"range\",{planes}],[\"sel\",\"go\",[\"at\",[\"at\",\"c\",\"a\"],\"p\"],[\"at\",[\"at\",\"c\",\"b\"],\"p\"]]]"
    );
    let text = format!(
        concat!(
            "{{\"law\":1,\"name\":\"tumble{planes}\",\"params\":0,\"channels\":{planes},\"halo\":0,",
            "\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},",
            "\"defs\":{{",
            "\"empty\":[[\"w\"],[\"eq\",[\"at\",\"w\",0],0]],",
            "\"mobile\":[[\"w\"],[\"ge\",[\"at\",\"w\",0],2]],",
            "\"swap\":[[\"c\",\"a\",\"b\",\"go\"],[\"put\",[\"put\",\"c\",\"a\",{swap_plane}],\"b\",{swap_back}]],",
            "\"fall\":[[\"c\",\"t\"],[\"swap\",\"c\",\"t\",[\"xor\",\"t\",4],[\"both\",[\"mobile\",[\"at\",\"c\",\"t\"]],[\"empty\",[\"at\",\"c\",[\"xor\",\"t\",4]]]]]],",
            "\"slide\":[[\"c\",\"t\",\"axis\"],[\"swap\",\"c\",\"t\",[\"xor\",[\"xor\",\"t\",\"axis\"],4],",
            "[\"both\",[\"both\",[\"mobile\",[\"at\",\"c\",\"t\"]],[\"denied\",[\"empty\",[\"at\",\"c\",[\"xor\",\"t\",4]]]]],",
            "[\"both\",[\"empty\",[\"at\",\"c\",[\"xor\",\"t\",\"axis\"]]],[\"empty\",[\"at\",\"c\",[\"xor\",[\"xor\",\"t\",\"axis\"],4]]]]]]]",
            "}},",
            "\"step\":[\"let\",[[\"a\",[\"fold\",\"c\",\"cells\",\"t\",[\"range\",4],[\"fall\",\"c\",\"t\"]]],",
            "[\"b\",[\"fold\",\"c\",\"a\",\"t\",[\"range\",4],[\"slide\",\"c\",\"t\",\"X\"]]],",
            "[\"out\",[\"fold\",\"c\",\"b\",\"t\",[\"range\",4],[\"slide\",\"c\",\"t\",\"Z\"]]]],",
            "[\"vec\",\"out\",[\"vec\",[\"count\",[\"map\",\"k\",[\"range\",8],[\"ne\",[\"at\",[\"at\",\"cells\",\"k\"],0],[\"at\",[\"at\",\"out\",\"k\"],0]]]]]]]}}"
        ),
        planes = planes,
        swap_plane = swap_plane,
        swap_back = swap_back,
    );
    Law::compile(&text).unwrap_or_else(|why| panic!("{why}"))
}

fn still_law(planes: usize) -> Law {
    let text = format!(
        "{{\"law\":1,\"name\":\"still{planes}\",\"params\":0,\"channels\":{planes},\"halo\":0,\"tally\":{{\"counts\":1,\"weighted\":false}},\"consts\":{{}},\"defs\":{{}},\"step\":[\"vec\",\"cells\",[\"vec\",0]]}}"
    );
    Law::compile(&text).unwrap_or_else(|why| panic!("{why}"))
}

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn index(size: [u32; 3], at: [u32; 3]) -> usize {
    ((at[2] as usize * size[1] as usize) + at[1] as usize) * size[0] as usize + at[0] as usize
}

fn near_a_seam(size: [u32; 3], at: [u32; 3]) -> bool {
    at.iter()
        .zip(size)
        .any(|(&one, side)| one % 8 <= 1 || one % 8 == 7 || one + 2 >= side)
}

fn start(size: [u32; 3], planes: usize, seed: u64) -> Vec<u32> {
    let cells = size.iter().map(|&side| side as usize).product::<usize>();
    let mut state = seed;
    let mut first = vec![0_u32; cells];
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                let at = [x, y, z];
                let roll = next(&mut state);
                let word = if y % 8 == 6 && !roll.is_multiple_of(5) {
                    WALL
                } else if roll.is_multiple_of(7)
                    || (near_a_seam(size, at) && roll.is_multiple_of(3))
                {
                    2 + (roll >> 32) as u32 % 0xFFFF
                } else {
                    0
                };
                first[index(size, at)] = word;
            }
        }
    }
    let mut out = Vec::with_capacity(cells * planes);
    for plane in 0..planes {
        out.extend(first.iter().map(|&word| marked(word, plane)));
    }
    out
}

fn tags_hold(cells: &[u32], planes: usize, label: &str) {
    let total = cells.len() / planes;
    for plane in 1..planes {
        for at in 0..total {
            assert_eq!(
                cells[plane * total + at],
                marked(cells[at], plane),
                "{label}: plane {plane} at cell {at} left the cell it belongs to"
            );
        }
    }
}

fn written(world: &mut World<'_>, cells: &[u32], size: [u32; 3]) {
    let total = cells.len() / world.planes() as usize;
    for plane in 0..world.planes() {
        let mut list = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let word = cells[plane as usize * total + index(size, [x, y, z])];
                    if word != 0 {
                        list.push(([x, y, z], word));
                    }
                }
            }
        }
        for chunk in list.chunks(4096) {
            world.write_plane(plane, chunk).unwrap();
        }
    }
}

fn law_world<'host>(
    host: &'host Host,
    law: &Law,
    size: [u32; 3],
    sparse: bool,
    cells: &[u32],
) -> World<'host> {
    let made = Made {
        rule: law.name(),
        params: &[],
        size,
        sparse,
        slots: Some(u32::MAX),
    };
    let mut world = World::create_law(host, law, &made).unwrap();
    written(&mut world, cells, size);
    world
}

fn reference(law: &Law, size: [u32; 3], frame: u32, cells: &[u32]) -> Vec<u32> {
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    sand3::reference(law, grid, frame, cells, &[])
}

#[test]
fn planes_move_with_their_cell() {
    let Some(host) = device("channels3") else {
        return;
    };
    for planes in [2_usize, 4] {
        let law = moving_law(planes);
        assert_eq!(law.planes(), planes);
        for sparse in [false, true] {
            let label = format!("{planes} planes, sparse {sparse}");
            let first = start(SIZE, planes, 7 + planes as u64);
            tags_hold(&first, planes, &label);
            let mut world = law_world(&host, &law, SIZE, sparse, &first);
            assert_eq!(world.cells().unwrap(), first, "{label}: written");
            let mut want = first.clone();
            for frame in 0..48_u32 {
                want = reference(&law, SIZE, frame, &want);
                world.step(1).unwrap();
                if frame % 6 == 5 {
                    let got = world.cells().unwrap();
                    tags_hold(&got, planes, &label);
                    assert!(got == want, "{label}: frame {frame} differs from the CPU");
                }
            }
            let total = first.len() / planes;
            let moved = (0..total).filter(|&at| want[at] != first[at]).count();
            assert!(moved > 500, "{label}: only {moved} cells moved");
            for plane in 1..planes {
                let moved = (0..total)
                    .filter(|&at| want[plane * total + at] != first[plane * total + at])
                    .count();
                assert!(moved > 500, "{label}: plane {plane} moved {moved} cells");
            }
        }
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

fn whole_dense(host: &Host, law: &Law, cells: &[u32], frames: u32) -> Vec<u32> {
    let width = host.width();
    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let built = sand3::settle_by(law, width, grid, host.limits().groups).unwrap();
    let tick = live::tick(width).unwrap();
    let modules: Vec<&[u32]> = vec![&tick, &built.words];
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = 16;
    sizes[STATE as usize] = cells.len();
    let mut held = host.resident(&sizes, &modules).unwrap();
    held.write(PARAMS as usize, &[u32::MAX]).unwrap();
    held.write(STATE as usize, cells).unwrap();
    let groups = [Groups::Fixed(1), built.spread.dispatch()];
    for _ in 0..frames {
        held.frame(&groups).unwrap();
    }
    held.read(STATE as usize, cells.len()).unwrap()
}

fn whole_sparse(host: &Host, law: &Law, cells: &[u32], frames: u32) -> Vec<u32> {
    let width = host.width();
    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let plan = plan_of(width, grid);
    let pool = Bricks::new(grid, &plan, plan.blocks)
        .unwrap()
        .with_planes(law.channels())
        .unwrap();
    let (passes, stages) = bricks::whole(law, &pool, &plan).unwrap();
    let sizes = pool.sizes(law, &plan);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    pool.write(&mut held, &plan, cells).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    for _ in 0..frames {
        held.staged(&groups).unwrap();
    }
    pool.read(&mut held).unwrap()
}

#[test]
fn a_two_plane_law_listed_equals_whole_equals_cpu_across_every_seam() {
    let Some(host) = device("channels3") else {
        return;
    };
    let law = moving_law(2);
    let first = start(SIZE, 2, 99);
    let frames = 40;
    let mut want = first.clone();
    for frame in 0..frames {
        want = reference(&law, SIZE, frame, &want);
    }
    tags_hold(&want, 2, "cpu");
    let total = first.len() / 2;
    let crossed = (0..total)
        .filter(|&at| want[at] >= 2 && first[at] < 2)
        .filter(|&at| {
            let x = at as u32 % SIZE[0];
            let y = (at as u32 / SIZE[0]) % SIZE[1];
            let z = at as u32 / SIZE[0] / SIZE[1];
            [x, y, z].iter().any(|&one| one % 8 == 0 || one % 8 == 7)
        })
        .count();
    assert!(
        crossed > 100,
        "only {crossed} grains came to rest by a seam"
    );

    assert!(
        whole_dense(&host, &law, &first, frames) == want,
        "dense whole differs from the CPU"
    );
    assert!(
        whole_sparse(&host, &law, &first, frames) == want,
        "sparse whole differs from the CPU"
    );
    for sparse in [false, true] {
        let mut world = law_world(&host, &law, SIZE, sparse, &first);
        world.step(frames).unwrap();
        assert!(
            world.cells().unwrap() == want,
            "listed, sparse {sparse}, differs from the CPU"
        );
    }
}

#[test]
fn a_brick_is_uniform_only_when_every_plane_is() {
    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let plan = plan_of(32, grid);
    let pool = Bricks::new(grid, &plan, plan.blocks)
        .unwrap()
        .with_planes(2)
        .unwrap();
    let per = pool.cells();
    let mut odd = vec![0_u32; per];
    odd[3] = 9;
    let packed = pool
        .compose_planes(|brick| match brick {
            0 => vec![Content::Uniform(4), Content::Mixed(odd.clone())],
            1 => vec![Content::Uniform(4), Content::Uniform(6)],
            _ => vec![Content::Uniform(0), Content::Mixed(vec![2; per])],
        })
        .unwrap();
    assert_eq!(
        packed.held, 1,
        "only the brick whose plane 1 is mixed is held"
    );
    assert_eq!(packed.table[0], 0);
    assert_eq!(packed.table[bricks::ENTRY], bricks::UNIFORM);
    assert_eq!(packed.table[bricks::ENTRY + bricks::VALUE], 4);
    assert_eq!(packed.table[pool.value_at(1, 1)], 6);
    assert_eq!(packed.table[pool.value_at(1, 2)], 2);
    assert_eq!(&packed.pool[..per], &vec![4; per][..]);
    assert_eq!(&packed.pool[per..], &odd[..]);

    let Some(host) = device("channels3") else {
        return;
    };
    let law = still_law(2);
    let made = Made {
        rule: law.name(),
        params: &[],
        size: SIZE,
        sparse: true,
        slots: None,
    };
    let mut world = World::create_law(&host, &law, &made).unwrap();
    assert_eq!(world.pool_use().unwrap().unwrap().held, 0);
    world.write_plane(1, &[([3, 4, 5], 5)]).unwrap();
    world.step(6).unwrap();
    assert_eq!(
        world.pool_use().unwrap().unwrap().held,
        1,
        "plane 0 is uniform and plane 1 is not: the brick stays held"
    );
    let cells = world.cells().unwrap();
    let total = cells.len() / 2;
    assert!(cells[..total].iter().all(|&word| word == 0));
    assert_eq!(cells[total + index(SIZE, [3, 4, 5])], 5);
    assert_eq!(cells[total..].iter().filter(|&&word| word != 0).count(), 1);

    world.write_plane(1, &[([3, 4, 5], 0)]).unwrap();
    world.step(6).unwrap();
    assert_eq!(
        world.pool_use().unwrap().unwrap().held,
        0,
        "every plane uniform again: the brick is released"
    );
    assert!(world.cells().unwrap().iter().all(|&word| word == 0));

    let sides = world.pool().unwrap().sides();
    let brick: Vec<([u32; 3], u32)> = (0..sides[2])
        .flat_map(|z| (0..sides[1]).flat_map(move |y| (0..sides[0]).map(move |x| ([x, y, z], 7))))
        .collect();
    world.write_plane(1, &brick).unwrap();
    world.step(6).unwrap();
    assert_eq!(
        world.pool_use().unwrap().unwrap().held,
        0,
        "plane 1 uniform at 7: the brick is uniform again, its value in the table"
    );
    let cells = world.cells().unwrap();
    for z in 0..SIZE[2] {
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let inside = x < sides[0] && y < sides[1] && z < sides[2];
                assert_eq!(cells[index(SIZE, [x, y, z])], 0);
                assert_eq!(
                    cells[total + index(SIZE, [x, y, z])],
                    if inside { 7 } else { 0 }
                );
            }
        }
    }
}

#[test]
fn plan_sizes_match_the_planes_table() {
    let side = 256_usize;
    let cells = side * side * side;
    let grid = Grid::solid(256, 256, 256).unwrap();
    let plan = plan_of(32, grid);
    let bytes_of = |sizes: &[usize]| sizes.iter().map(|&words| words * 4).sum::<usize>();
    let one = bytes_of(&plan.windows().laid(&plan.sizes())) as f64 / cells as f64;
    for (planes, wanted) in [(1_usize, 6.14), (2, 10.14), (4, 18.14)] {
        let law = still_law(planes);
        let sizes = live::windows3(&law, &plan).laid(&live::sizes3(&law, &plan));
        assert_eq!(sizes[STATE as usize], planes * plan.state_words());
        let per_cell = bytes_of(&sizes) as f64 / cells as f64;
        assert!(
            (per_cell - wanted).abs() < 0.01,
            "{planes} planes: {per_cell} bytes a cell, the table says {wanted}"
        );
        assert!((per_cell - one - 4.0 * (planes - 1) as f64).abs() < 1e-6);

        let pool = Bricks::new(grid, &plan, 64)
            .unwrap()
            .with_planes(planes as u32)
            .unwrap();
        assert_eq!(pool.pool_words(), planes * 64 * pool.cells() + 1);
        assert_eq!(pool.plane_words(), 64 * pool.cells());
        let held_bytes = (pool.pool_words() - 1) * 4 / (64 * pool.cells());
        assert_eq!(held_bytes, 4 * planes, "sparse pool bytes a held cell");
        let count = pool.count() as usize;
        assert_eq!(
            pool.table_words(),
            count * bricks::ENTRY + bricks::ENTRY + (planes - 1) * (count + 1)
        );
    }
}

#[test]
fn a_world_of_planes_saves_loads_and_forks_as_itself() {
    let Some(host) = device("channels3") else {
        return;
    };
    let law = moving_law(2);
    let folder = std::env::temp_dir().join(format!("grit-channels3-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for sparse in [false, true] {
        let first = start(SIZE, 2, 5);
        let mut world = law_world(&host, &law, SIZE, sparse, &first);
        world.step(5).unwrap();
        let path = folder.join(format!("planes-{sparse}.world"));
        world.save(&path).unwrap();
        let image = format::fetch(&path).unwrap();
        assert_eq!(image.channels(), 2);
        assert!(matches!(image.law, FileLaw::Law { .. }));
        let mut loaded = World::load(&host, &path).unwrap();
        let mut forked = world.fork().unwrap();
        assert_eq!(loaded.planes(), 2);
        assert_eq!(loaded.frame(), 5);
        assert_eq!(loaded.cells().unwrap(), world.cells().unwrap());
        for _ in 0..4 {
            world.step(4).unwrap();
            loaded.step(4).unwrap();
            forked.step(4).unwrap();
            let hash = world.state_hash().unwrap();
            assert_eq!(loaded.state_hash().unwrap(), hash, "sparse {sparse}: load");
            assert_eq!(forked.state_hash().unwrap(), hash, "sparse {sparse}: fork");
        }
        let mut again = World::load(&host, &path).unwrap();
        again.restore(&mut world).unwrap();
        assert_eq!(again.state_hash().unwrap(), world.state_hash().unwrap());
        std::fs::remove_file(&path).unwrap();
    }
    let _ = std::fs::remove_dir(&folder);
}

fn fixture_image() -> format::Image {
    let law = moving_law(2);
    let size = [16, 16, 16];
    let cells = start(size, 2, 3);
    format::Image {
        law: FileLaw::Law {
            name: law.name().to_owned(),
            canonical: law.canonical().to_owned(),
        },
        params: Vec::new(),
        frame: 12,
        size,
        state: format::bricked_planes(&cells, 2, size, [8, 8, 4], None),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn words_hex(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    hex(&sha256(&bytes))
}

#[test]
fn the_checked_in_two_plane_file_reads_as_its_cells() {
    if std::env::var_os("GRIT_WRITE_CHANNELS_FIXTURE").is_some() {
        let bytes = format::encoded(&fixture_image());
        std::fs::write("tests/branch/two-planes.world", &bytes).unwrap();
        println!(
            "sha {} cells {}",
            hex(&sha256(&bytes)),
            words_hex(&fixture_image().unpacked())
        );
        return;
    }
    assert_eq!(hex(&sha256(FIXTURE)), FIXTURE_SHA);
    let image = format::decoded(FIXTURE).unwrap();
    assert_eq!(image, fixture_image());
    assert_eq!(image.channels(), 2);
    assert!(matches!(image.state, State::Sparse { .. }));
    let cells = image.unpacked();
    assert_eq!(cells.len(), 2 * 16 * 16 * 16);
    assert_eq!(words_hex(&cells), FIXTURE_CELLS);
    tags_hold(&cells, 2, "fixture");
    assert_eq!(format::encoded(&image), FIXTURE);

    let mut header = FIXTURE.to_vec();
    let at = 64;
    assert_eq!(&header[at..at + 4], &2_u32.to_le_bytes());
    header[at..at + 4].copy_from_slice(&1_u32.to_le_bytes());
    assert!(format::decoded(&header).is_err(), "a wrong channel count");
    header[at..at + 4].copy_from_slice(&5_u32.to_le_bytes());
    assert!(format::decoded(&header).is_err(), "five channels");

    let Some(host) = device("channels3") else {
        return;
    };
    let mut world = World::open(&host, &image).unwrap();
    assert_eq!(world.planes(), 2);
    assert_eq!(world.frame(), 12);
    assert_eq!(world.cells().unwrap(), cells);
    let law = moving_law(2);
    let mut want = cells;
    for frame in 12..20 {
        want = reference(&law, [16, 16, 16], frame, &want);
    }
    world.step(8).unwrap();
    assert_eq!(world.cells().unwrap(), want);
}

#[test]
fn seeding_marks_every_held_slot_and_nothing_else() {
    let Some(host) = device("channels3") else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let plan = plan_of(width, grid);
    let capacity = 8_u32.min(plan.blocks);
    let pool = Bricks::new(grid, &plan, capacity).unwrap();
    let law = still_law(1);
    let sizes = pool.sizes(&law, &plan);
    let mut held = host.resident(&sizes, &[]).unwrap();
    let per_block = pool.per_block() as usize;
    let awake_words = sizes[AWAKE as usize];
    let before: Vec<u32> = (0..awake_words).map(|at| 7 + (at as u32 % 3)).collect();
    held.write(AWAKE as usize, &before).unwrap();
    held.write(PARAMS as usize, &[41, 0]).unwrap();
    let mut table = vec![0_u32; pool.table_words()];
    for entry in table.chunks_mut(bricks::ENTRY) {
        entry[0] = bricks::UNIFORM;
    }
    let mut slots = [0_u32, 1, 2, 5, 7]
        .into_iter()
        .filter(|&slot| slot < capacity)
        .collect::<Vec<_>>();
    slots.truncate(pool.count() as usize);
    let mut owners = Vec::new();
    for (index, &slot) in slots.iter().enumerate() {
        let brick = (index * 3) % pool.count() as usize;
        if owners.contains(&brick) {
            continue;
        }
        owners.push(brick);
        table[brick * bricks::ENTRY] = slot;
    }
    held.write(pool.table() as usize, &table).unwrap();
    pool.seed(&mut held, &plan).unwrap();
    let after = held.read(AWAKE as usize, awake_words).unwrap();
    let mut want = before;
    for &brick in &owners {
        let slot = table[brick * bricks::ENTRY] as usize;
        for word in &mut want[slot * per_block..(slot + 1) * per_block] {
            *word = bricks::AWAKE_LEVEL;
        }
    }
    assert_eq!(after, want);
    assert_eq!(
        held.read(PARAMS as usize, 2).unwrap(),
        vec![41, pool.dump()]
    );
    let runs = pool.held_runs(&table);
    let held_slots: u32 = runs.iter().map(|&(_, length)| length).sum();
    assert_eq!(held_slots as usize, owners.len());
}

#[test]
fn planes_past_their_limits_are_refused_by_name() {
    let why = Law::compile(
        &still_law(2)
            .canonical()
            .replace("\"channels\":2", "\"channels\":5"),
    )
    .unwrap_err();
    assert_eq!(why.code, Code::Channels);
    let words_out = still_law(1)
        .canonical()
        .replace("\"channels\":1", "\"channels\":2")
        .replace(
            "[\"vec\",\"cells\",[\"vec\",0]]",
            "[\"vec\",[\"map\",\"w\",\"cells\",[\"at\",\"w\",0]],[\"vec\",0]]",
        );
    let why = Law::compile(&words_out).unwrap_err();
    assert_eq!(why.code, Code::Result, "{why}");
    assert!(why.expected.contains("8 vecs of 2 words"), "{why}");

    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let plan = plan_of(32, grid);
    let pool = Bricks::new(grid, &plan, 4).unwrap();
    assert!(pool.with_planes(0).is_err());
    assert!(pool.with_planes(5).is_err());
    let why = bricks::arranged(&still_law(2), &pool, &plan, true).unwrap_err();
    assert!(
        format!("{why:?}").contains(bricks::UNLIKE_PLANES),
        "{why:?}"
    );
    let two = pool.with_planes(2).unwrap();
    let why = bricks::arranged(&still_law(1), &two, &plan, true).unwrap_err();
    assert!(
        format!("{why:?}").contains(bricks::UNLIKE_PLANES),
        "{why:?}"
    );

    let huge = Grid::solid(2048, 1024, 2048).unwrap();
    let why = sand3::settle_by(&still_law(2), 32, huge, 65_535).unwrap_err();
    assert!(format!("{why:?}").contains(sand3::PLANES_PAST), "{why:?}");
}

fn exact(raw: &[u32], words: usize) -> Vec<u128> {
    (0..words)
        .map(|word| {
            let part = |at: usize| u128::from(raw.get(at).copied().unwrap_or(0));
            part(word) | part(words + word) << 32 | part(2 * words + word) << 64
        })
        .collect()
}

#[test]
fn a_tally_past_four_billion_is_exact() {
    use grit::layout::{COARSE, TALLY};

    let Some(host) = device("channels3") else {
        return;
    };
    let width = host.width();
    let most = u32::MAX;
    let text = "{\"law\":1,\"name\":\"heavy\",\"params\":0,\"channels\":1,\"halo\":0,\"tally\":{\"counts\":1,\"weighted\":true},\"consts\":{},\"defs\":{},\"step\":[\"vec\",\"cells\",[\"vec\",4294967295]]}";
    let law = Law::compile(text).unwrap();
    let grid = Grid::solid(SIZE[0], SIZE[1], SIZE[2]).unwrap();
    let plan = plan_of(width, grid);
    let octs = grid.blocks();
    let lattice = [SIZE[0] / 2, SIZE[1] / 2, SIZE[2] / 2];
    let mut want = vec![u128::from(octs) * u128::from(most)];
    for side in lattice {
        let others = u128::from(octs / side);
        let along: u128 = (0..side).map(|at| u128::from(at >> COARSE)).sum();
        want.push(along * others * u128::from(most));
    }
    want.push(u128::from(octs));
    assert!(want[1] > u128::from(u32::MAX));

    for sparse in [false, true] {
        let pool = Bricks::new(grid, &plan, plan.blocks).unwrap();
        let ((passes, stages), sizes) = if sparse {
            (
                bricks::arranged(&law, &pool, &plan, true).unwrap(),
                pool.sizes(&law, &plan),
            )
        } else {
            (
                live::wired3_by(&law, width, grid, &plan).unwrap(),
                plan.windows().laid(&live::sizes3(&law, &plan)),
            )
        };
        let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
        let mut held = host.resident(&sizes, &modules).unwrap();
        for (binding, &words) in sizes.iter().enumerate() {
            held.write(binding, &vec![0_u32; words]).unwrap();
        }
        held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
            .unwrap();
        if sparse {
            pool.write(&mut held, &plan, &vec![3; grid.cells()])
                .unwrap();
            pool.seed(&mut held, &plan).unwrap();
        } else {
            live::seed(&mut held, grid, &plan).unwrap();
        }
        let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        held.staged(&groups).unwrap();
        let raw = held.read(TALLY as usize, 15).unwrap();
        assert_eq!(exact(&raw, 5), want, "sparse {sparse}");
        assert_eq!(
            live::read_tally(&mut held, 5).unwrap(),
            want,
            "sparse {sparse}"
        );
    }
}

#[test]
fn a_flat_tally_past_four_billion_is_exact() {
    use grit::layout::{COARSE, TALLY, TALLY_WORDS};
    use grit::sand::{AIR, SAND, WATER, material_of};

    let Some(host) = device("channels3") else {
        return;
    };
    let width = host.width();
    let (grid, plan) = [128_u32, 64, 32, 16, 8, 4, 2]
        .into_iter()
        .find_map(|deep| {
            let grid = Grid::of(65_536, deep);
            Plan::over(width, grid.quads().div_ceil(sweep::cells(width)))
                .ok()
                .map(|plan| (grid, plan))
        })
        .unwrap();
    let quads = grid.quads();
    let (passes, stages) = live::wired(width, grid, &plan, None).unwrap();
    let sizes = plan.sizes();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; words]).unwrap();
    }
    let cells = grid.cells();
    let before: Vec<u32> = (0..cells)
        .map(|at| {
            let row = at / grid.wide as usize;
            if row.is_multiple_of(2) && row > 0 {
                SAND
            } else {
                AIR
            }
        })
        .collect();
    held.write(STATE as usize, &before).unwrap();
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    held.staged(&groups).unwrap();
    let after = held.read(STATE as usize, cells).unwrap();
    let raw = held.read(TALLY as usize, 3 * TALLY_WORDS).unwrap();
    let got = exact(&raw, TALLY_WORDS);

    let mut want = vec![0_u128; TALLY_WORDS];
    for quad in 0..quads {
        let column = (quad % grid.across()) * 2;
        let row = (quad / grid.across()) * 2;
        let corners = [
            (row * grid.wide + column) as usize,
            (row * grid.wide + column + 1) as usize,
            ((row + 1) * grid.wide + column) as usize,
            ((row + 1) * grid.wide + column + 1) as usize,
        ];
        let moved = corners
            .iter()
            .filter(|&&at| before[at] != after[at])
            .count() as u128;
        want[0] += moved;
        for at in corners {
            match material_of(after[at]) {
                WATER => want[1] += 1,
                SAND => want[2] += 1,
                _ => {}
            }
        }
        want[3] += u128::from((quad % grid.across()) >> COARSE) * moved;
        want[4] += u128::from((quad / grid.across()) >> COARSE) * moved;
        want[5] += u128::from(moved > 0);
    }
    if grid.deep == 128 {
        assert!(want[3] > u128::from(u32::MAX), "{want:?}");
    }
    assert_eq!(got, want);
}
