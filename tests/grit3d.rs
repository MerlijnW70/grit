#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

#[path = "../src/bin/grit3d/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/bin/grit3d/life.rs"]
#[allow(dead_code)]
mod life;
#[path = "../src/bin/grit3d/open.rs"]
#[allow(dead_code)]
mod open;
#[path = "../src/bin/grit3d/scenes.rs"]
#[allow(dead_code)]
mod scenes;
#[path = "../src/bin/grit3d/session.rs"]
#[allow(dead_code)]
mod session;
#[path = "../src/bin/grit3d/ui.rs"]
#[allow(dead_code)]
mod ui;
#[path = "../src/bin/grit3d/valley.rs"]
#[allow(dead_code)]
mod valley;

use grit::render3::{Camera, Screen, Steps};
use grit::rule3::Rule;
use grit::rule3::code::Code;
use grit::rule3::heat::Heat;
use grit::rule3::nature::Nature;
use grit::rule3::sand::Sand;
use grit::sand::Grid;
use grit::sand3::{self, Grid3};

use scenes::Which;
use session::{Session, Setup};

const SIDE: u32 = 32;

const DENSE: [Which; 4] = [Which::Valley, Which::Sand, Which::Fire, Which::Code];

const LAND: [u32; 3] = [256, 64, 256];

fn setup(which: Which) -> Setup {
    Setup {
        which,
        side: SIDE,
        screen: Screen { wide: 16, high: 12 },
        suns: 1,
        sparse: false,
        at: None,
        world: (which == Which::Open).then_some(LAND),
    }
}

/// The world of `which` after its warm-up, found on the CPU.
fn warmed(which: Which) -> Vec<u32> {
    let example = which.example();
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    let mut cells = (example.scene)(grid);
    let feed = fed(which);
    for frame in 0..example.warm {
        feeding(&feed, frame, &mut cells);
        cells = stepped(which, frame, &cells, &example.params);
    }
    cells
}

/// The cells `which` writes before every step its feed is due at.
fn fed(which: Which) -> Vec<(u32, u32)> {
    let example = which.example();
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    example
        .feed
        .map_or_else(Vec::new, |made| made(grid, &(example.scene)(grid)))
}

fn feeding(feed: &[(u32, u32)], frame: u32, cells: &mut [u32]) {
    if frame.is_multiple_of(valley::FEED_EVERY) {
        for &(cell, word) in feed {
            cells[cell as usize] = word;
        }
    }
}

fn stepped(which: Which, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
    let grid = Grid3::new(SIDE, SIDE, SIDE).unwrap();
    fn one<R: Rule>(rule: &R, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
        sand3::reference(rule, grid, frame, cells, params)
    }
    match which {
        Which::Valley => one(&Nature, grid, frame, cells, params),
        Which::Sand => one(&Sand, grid, frame, cells, params),
        Which::Fire => one(&Heat, grid, frame, cells, params),
        Which::Code => one(&Code, grid, frame, cells, params),
        Which::Open => panic!("the open world has no dense run"),
    }
}

fn looking(session: &Session<'_>) -> Camera {
    let side = session.grid().wide as f32;
    Camera::looking(
        [side * 1.6, -side * 0.9, side * 1.7],
        [side * 0.5, side * 0.7, side * 0.5],
        0.8,
    )
}

#[test]
fn switching_or_restarting_a_scene_starts_it_as_the_rule_runs_it_from_its_world() {
    let Some(host) = device("grit3d scenes") else {
        return;
    };
    let mut wrong = Vec::new();
    for (from, to) in [
        (Which::Sand, Which::Fire),
        (Which::Fire, Which::Code),
        (Which::Code, Which::Sand),
        (Which::Sand, Which::Valley),
        (Which::Fire, Which::Fire),
    ] {
        let mut before = Session::open(&host, setup(from)).unwrap();
        let camera = looking(&before);
        for at in 0..5 {
            before.aim(&camera, at % 2 == 1, at == 0).unwrap();
            before.frame(true, Steps::ALL, false).unwrap();
        }
        let tool = before.example.tools[0];
        let middle = SIDE / 2;
        let cell = grit::geometry::Solid::cell(before.grid(), [middle, middle, middle]);
        before.stroke((cell, 2), tool).unwrap();
        before.frame(true, Steps::KEPT, false).unwrap();
        drop(before);
        let mut after = Session::open(&host, setup(to)).unwrap();
        let got = after.state().unwrap();
        let want = warmed(to);
        if got != want {
            let differ = got.iter().zip(&want).filter(|(a, b)| a != b).count();
            wrong.push(format!(
                "{from:?} to {to:?}: {differ} cells differ from the {to:?} world after its warm-up"
            ));
        }
        println!(
            "{from:?} to {to:?}: {} cells, {} frames of warm-up, the same as the rule's own run",
            got.len(),
            after.example.warm
        );
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_tool_stroke_goes_through_put3_and_the_listed_engine_stays_equal_to_the_whole_grid() {
    let Some(host) = device("grit3d tool") else {
        return;
    };
    let mut wrong = Vec::new();
    for which in DENSE {
        let mut session = Session::open(&host, setup(which)).unwrap();
        let mut cells = session.state().unwrap();
        let camera = looking(&session);
        let params = session.example.params.clone();
        let feed = fed(which);
        let mut strokes = 0;
        for at in 0..12_u32 {
            if at % 3 == 1 {
                let screen = session.build.screen;
                let tool_at = (at as usize / 3) % session.example.tools.len();
                let tool = session.example.tools[tool_at];
                let mut found = None;
                for step in 0..screen.pixels() as u32 {
                    let pixel = (
                        (screen.wide / 2 + at + step) % screen.wide,
                        (screen.high / 2 + step / screen.wide) % screen.high,
                    );
                    found = session.under(pixel).unwrap();
                    if found.is_some() {
                        break;
                    }
                }
                if let Some(seen) = found {
                    let writes = session.stroke(seen, tool).unwrap();
                    for &(cell, word) in &writes {
                        cells[cell as usize] = word;
                    }
                    strokes += usize::from(!writes.is_empty());
                }
            }
            let frame = session.steps;
            session.aim(&camera, false, at == 0).unwrap();
            session.frame(true, Steps::KEPT, false).unwrap();
            feeding(&feed, frame, &mut cells);
            cells = stepped(which, frame, &cells, &params);
            let got = session.state().unwrap();
            if got != cells {
                let differ = got.iter().zip(&cells).filter(|(a, b)| a != b).count();
                wrong.push(format!("{which:?} frame {frame}: {differ} cells differ"));
                break;
            }
        }
        println!(
            "{which:?}: {strokes} tool strokes, the world equal to the whole-grid rule after each frame"
        );
        if strokes == 0 {
            wrong.push(format!("{which:?}: no stroke landed"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_active_overlay_marks_exactly_the_octs_the_engine_steps() {
    let Some(host) = device("grit3d overlay") else {
        return;
    };
    let mut checked = 0;
    for which in DENSE {
        let mut session = Session::open(&host, setup(which)).unwrap();
        let camera = looking(&session);
        for at in 0..6 {
            let mut listed = session.listed().unwrap();
            listed.sort_unstable();
            listed.dedup();
            session.aim(&camera, true, at == 0).unwrap();
            session.frame(true, Steps::KEPT, false).unwrap();
            let marked = session.marked().unwrap();
            assert_eq!(
                marked, listed,
                "{which:?} frame {at}: the overlay marks other octs than the list"
            );
            assert_eq!(session.active().unwrap() as usize, listed.len());
            checked += listed.len();
        }
    }
    println!("{checked} listed octs, each marked, and none other");
    assert!(checked > 0, "no oct was listed");
}

#[test]
fn every_tool_button_is_found_where_it_is_drawn() {
    for which in Which::ALL {
        let tools = which.example().tools;
        for window in [(1280, 720), (1920, 1080), (640, 360)] {
            let areas = ui::bar(&tools, window);
            assert_eq!(areas.len(), tools.len());
            for (at, area) in areas.iter().enumerate() {
                assert!(
                    area.x >= 0 && area.y >= 0,
                    "{which:?} {window:?}: off the window"
                );
                assert!(area.x + i64::from(area.wide) <= i64::from(window.0));
                assert!(area.y + i64::from(area.high) <= i64::from(window.1));
                let middle = (
                    area.x as f32 + area.wide as f32 / 2.0,
                    area.y as f32 + area.high as f32 / 2.0,
                );
                assert_eq!(ui::pressed(&tools, window, middle), Some(at));
                if let Some(next) = areas.get(at + 1) {
                    assert!(area.x + i64::from(area.wide) < next.x, "buttons overlap");
                }
            }
            assert_eq!(ui::pressed(&tools, window, (2.0, 2.0)), None);
        }
    }
}

#[test]
fn every_letter_the_demo_writes_has_a_glyph() {
    let mut text: Vec<String> = ui::HELP.iter().map(|line| (*line).to_owned()).collect();
    for which in Which::ALL {
        let example = which.example();
        text.push(example.name.to_owned());
        for (at, tool) in example.tools.iter().enumerate() {
            text.push(ui::label(at, tool));
        }
    }
    text.push(format!(
        "FIRE \u{b7} 144 FPS \u{b7} ACTIVE 12 OCTS (0.25%) \u{b7} PAUSED \u{b7} FLYING \u{b7} {}",
        ui::HINT
    ));
    for line in &text {
        for letter in line.chars() {
            if letter != ' ' {
                assert_ne!(
                    font::glyph(letter),
                    [0; 7],
                    "{letter:?} in {line:?} has no glyph"
                );
            }
        }
    }
    let mut pixels = vec![0_u32; 64 * 16];
    let mut canvas = font::Canvas {
        pixels: &mut pixels,
        wide: 64,
        high: 16,
    };
    canvas.text((1, 1), "GRIT", 0x00FF_FFFF, 1);
    assert!(pixels.contains(&0x00FF_FFFF));
}

#[test]
fn the_valleys_water_falls_where_the_nature_rule_marks_it_and_nowhere_else() {
    use grit::render3::occupancy::watered;
    use grit::rule3::nature::{EARTH, FALLING, WATER};
    let grid = Grid::solid(16, 16, 16).unwrap();
    let example = Which::Valley.example();
    let kinds = grit::view3::Kinds::named(example.rule).unwrap();
    assert_eq!(
        example.table.material(WATER).falls,
        FALLING.trailing_zeros() + 1,
        "the valley's water does not name the rule's falling bit"
    );
    let mut cells = vec![0_u32; grid.cells()];
    let mut marked = 0;
    for z in 0..4_u32 {
        for y in 0..4_u32 {
            for x in 0..4_u32 {
                let at = ((z * grid.deep + y) * grid.wide + x) as usize;
                let fell = (x + 2 * y + 3 * z) % 3 == 0;
                marked += u32::from(fell);
                cells[at] = (WATER << kinds.shift()) | if fell { FALLING } else { 0 };
            }
        }
    }
    cells[((grid.deep - 1) * grid.wide + 8) as usize] = EARTH << kinds.shift();
    let water = watered(grid, kinds, &example.table, &cells);
    let spray = water.get(1).copied().unwrap_or(0.0);
    println!("one brick of water, {marked} of 64 cells marked: spray share {spray}");
    assert!(marked > 0 && marked < 64);
    assert!(
        (spray - marked as f32 / 64.0).abs() < 1.0e-6,
        "spray share {spray}, the marked share is {}",
        marked as f32 / 64.0
    );
}

#[test]
fn the_glass_box_holds_most_of_the_valley_and_leaves_its_peak_out() {
    let grid = Grid::solid(64, 64, 64).unwrap();
    let example = Which::Valley.example();
    let cells = (example.scene)(grid);
    let kinds = grit::view3::Kinds::named(example.rule).unwrap();
    let visible = example.table.visible();
    let top = session::glass_top(grid, kinds, visible, &cells);
    let mut tops = Vec::new();
    for z in 0..grid.tall {
        for x in 0..grid.wide {
            let seen = (0..grid.deep).find(|&y| {
                let word = cells[((z * grid.deep + y) * grid.wide + x) as usize];
                let kind = (word >> kinds.shift()) & kinds.mask();
                (visible >> kind) & 1 == 1
            });
            if let Some(y) = seen {
                tops.push(y);
            }
        }
    }
    let inside = tops
        .iter()
        .filter(|&&y| y as f32 >= top + session::GLASS_MARGIN as f32)
        .count();
    let highest = tops.iter().copied().min().unwrap();
    println!(
        "64^3 valley: the glass begins {top} cells down; {inside} of {} columns lie a margin inside it; the highest cell is {highest} down",
        tops.len()
    );
    assert!(top > 0.0, "the glass reaches the grid's top");
    assert!(
        inside as f32 >= tops.len() as f32 * session::GLASS_SHARE,
        "the glass leaves out more columns than it should"
    );
    assert!(
        (highest as f32) < top,
        "the peak does not stand out of the glass"
    );
}

#[test]
fn a_frame_shown_from_the_device_steps_the_world_as_a_frame_read_back_does() {
    let Some(host) = device("grit3d present path") else {
        return;
    };
    let window = Screen { wide: 37, high: 29 };
    let mut shown = Session::open_in(&host, setup(Which::Valley), Some(window)).unwrap();
    let mut read = Session::open(&host, setup(Which::Valley)).unwrap();
    let side = SIDE as f32;
    let camera = Camera::looking(
        [side * 1.5, -side * 0.8, side * 1.5],
        [side * 0.5, side * 0.7, side * 0.5],
        0.62,
    );
    let mut burning = 0;
    for frame in 0..140 {
        shown.aim(&camera, false, frame == 0).unwrap();
        read.aim(&camera, false, frame == 0).unwrap();
        let pending = shown.submit(true, Steps::KEPT).unwrap();
        shown.land(pending).unwrap();
        read.frame(true, Steps::KEPT, false).unwrap();
        assert_eq!(shown.steps, read.steps, "frame {frame}: steps");
        let phases = (
            shown.life.as_ref().map(|life| (life.phase, life.round)),
            read.life.as_ref().map(|life| (life.phase, life.round)),
        );
        assert_eq!(phases.0, phases.1, "frame {frame}: the cycle");
        if phases
            .0
            .is_some_and(|(phase, _)| phase == life::Phase::Burning)
        {
            burning += 1;
        }
        if frame % 20 == 19 {
            assert!(
                shown.state().unwrap() == read.state().unwrap(),
                "frame {frame}: the worlds differ"
            );
        }
    }
    assert!(burning > 0, "the fire never burned in the frames tested");
    println!(
        "140 frames, {burning} burning: the world shown from the device equals the one read back"
    );
}

fn sparse_setup(which: Which) -> Setup {
    Setup {
        sparse: true,
        ..setup(which)
    }
}

fn picture(session: &mut Session<'_>, camera: &Camera, at: u32, moving: bool) -> Vec<u32> {
    session.aim(camera, false, at == 0).unwrap();
    let steps = Steps {
        refill: at == 0,
        ..Steps::ALL
    };
    session.frame(moving, steps, true).unwrap().1
}

fn landed(session: &mut Session<'_>, at: u32) -> Option<(u32, u32)> {
    let screen = session.build.screen;
    (0..screen.pixels() as u32).find_map(|step| {
        let pixel = (
            (screen.wide / 2 + at + step) % screen.wide,
            (screen.high / 2 + step / screen.wide) % screen.high,
        );
        session.under(pixel).unwrap()
    })
}

#[test]
fn every_scene_run_sparse_draws_and_steps_as_the_dense_one_with_its_tools() {
    let Some(host) = device("grit3d sparse") else {
        return;
    };
    let mut wrong = Vec::new();
    for which in DENSE {
        let mut dense = Session::open(&host, setup(which)).unwrap();
        let mut sparse = Session::open(&host, sparse_setup(which)).unwrap();
        assert!(sparse.pool.is_some() && dense.pool.is_none());
        let camera = looking(&dense);
        let mut strokes = 0;
        for at in 0..24_u32 {
            if at % 4 == 2 {
                let tool = dense.example.tools[(at as usize / 4) % dense.example.tools.len()];
                let seen = landed(&mut dense, at);
                assert_eq!(seen, landed(&mut sparse, at), "{which:?} frame {at}: hits");
                if let Some(seen) = seen {
                    let want = dense.stroke(seen, tool).unwrap();
                    let got = sparse.stroke(seen, tool).unwrap();
                    assert_eq!(got, want, "{which:?} frame {at}: the stroke");
                    strokes += usize::from(!want.is_empty());
                }
            }
            let want = picture(&mut dense, &camera, at, true);
            let got = picture(&mut sparse, &camera, at, true);
            if got != want {
                let differ = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                wrong.push(format!("{which:?} frame {at}: {differ} pixels differ"));
                break;
            }
            if at % 8 == 7 && sparse.state().unwrap() != dense.state().unwrap() {
                wrong.push(format!("{which:?} frame {at}: the worlds differ"));
                break;
            }
        }
        let used = {
            let pool = sparse.pool.as_ref().unwrap();
            pool.bricks.check(&mut sparse.held).unwrap()
        };
        println!(
            "{which:?}: 24 frames, {strokes} strokes through put_at, the sparse picture equal to the dense one; pool {} of {} slots held",
            used.held, used.capacity
        );
        assert!(strokes > 0, "{which:?}: no stroke landed");
    }
    assert!(
        wrong.is_empty(),
        "{}",
        wrong.join(
            "
"
        )
    );
}

#[test]
fn the_open_world_follows_the_camera_and_takes_the_tools_where_they_land() {
    let Some(host) = device("grit3d open") else {
        return;
    };
    let small = Setup {
        side: 16,
        ..setup(Which::Open)
    };
    let mut session = Session::open(&host, small).unwrap();
    let view = session.grid();
    let world = session.pool.as_ref().unwrap().bricks.grid();
    assert_eq!([view.wide, view.deep, view.tall], [64, LAND[1], 64]);
    assert_eq!(world, Grid::solid(LAND[0], LAND[1], LAND[2]).unwrap());
    let side = view.wide as f32;
    let camera = Camera::looking(
        [side * 1.2, -side * 0.5, side * 1.3],
        [side * 0.5, view.deep as f32 * 0.6, side * 0.5],
        0.62,
    );
    let mut first = Vec::new();
    for at in 0..8 {
        first = picture(&mut session, &camera, at, false);
    }
    let sky = first[0];
    let land = first.iter().filter(|&&pixel| pixel != sky).count();
    assert!(
        land > first.len() / 4,
        "the open world shows little: {land}"
    );

    let before = session.pool.as_ref().unwrap().corner;
    assert_eq!(session.follow([side * 0.5, 0.0, side * 0.5]).unwrap(), None);
    let by = session
        .follow([side * 0.5 + 40.0, 0.0, side * 0.5 - 30.0])
        .unwrap()
        .expect("the window stays put when the camera leaves its middle");
    let after = session.pool.as_ref().unwrap().corner;
    for axis in [0, 2] {
        let side = [world.wide, world.deep, world.tall][axis];
        assert_eq!(
            (before[axis] as f32 + by[axis] + side as f32) as u32 % side,
            after[axis]
        );
    }
    assert_eq!(by[1], 0.0);
    let moved = picture(&mut session, &camera, 1, false);

    let middle = [after[0] + view.wide / 2, after[2] + view.tall / 2];
    let mut fresh = Session::open(
        &host,
        Setup {
            at: Some(middle),
            ..small
        },
    )
    .unwrap();
    assert_eq!(fresh.pool.as_ref().unwrap().corner, after);
    let want = picture(&mut fresh, &camera, 0, false);
    let differ = moved.iter().zip(&want).filter(|(a, b)| a != b).count();
    assert_eq!(
        differ, 0,
        "the moved window draws other pixels than a window opened there"
    );

    let tool = session.example.tools[0];
    let seen = landed(&mut session, 0).expect("no pixel sees the ground");
    let writes = session.stroke(seen, tool).unwrap();
    assert!(!writes.is_empty());
    let cells = session.state().unwrap();
    for &(cell, word) in &writes {
        let point = session.world_point(cell);
        let at = grit::geometry::Solid::cell(world, point) as usize;
        assert_eq!(cells[at], word, "the stroke at {point:?} of the world");
    }
    for at in 0..4 {
        picture(&mut session, &camera, at + 2, true);
    }
    let used = {
        let pool = session.pool.as_ref().unwrap();
        pool.bricks.check(&mut session.held).unwrap()
    };
    println!(
        "open world {}x{}x{} through a {}x{}x{} window: moved by {by:?} to {after:?}, {land} pixels of land, a stroke of {} cells through put_at, pool {} of {} slots held",
        world.wide,
        world.deep,
        world.tall,
        view.wide,
        view.deep,
        view.tall,
        writes.len(),
        used.held,
        used.capacity
    );
}
