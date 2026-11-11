#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

#[path = "../src/bin/grit3d/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/bin/grit3d/scenes.rs"]
#[allow(dead_code)]
mod scenes;
#[path = "../src/bin/grit3d/session.rs"]
#[allow(dead_code)]
mod session;
#[path = "../src/bin/grit3d/ui.rs"]
#[allow(dead_code)]
mod ui;

use grit::render3::{Camera, Screen, Steps};
use grit::rule3::Rule;
use grit::rule3::code::Code;
use grit::rule3::heat::Heat;
use grit::rule3::sand::Sand;
use grit::sand::Grid;
use grit::sand3::{self, Grid3};

use scenes::Which;
use session::{Session, Setup};

const SIDE: u32 = 32;

fn setup(which: Which) -> Setup {
    Setup {
        which,
        side: SIDE,
        screen: Screen { wide: 16, high: 12 },
        suns: 1,
    }
}

/// The world of `which` after its warm-up, found on the CPU.
fn warmed(which: Which) -> Vec<u32> {
    let example = which.example();
    let grid = Grid::solid(SIDE, SIDE, SIDE).unwrap();
    let mut cells = (example.scene)(grid);
    for frame in 0..example.warm {
        cells = stepped(which, frame, &cells, &example.params);
    }
    cells
}

fn stepped(which: Which, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
    let grid = Grid3::new(SIDE, SIDE, SIDE).unwrap();
    fn one<R: Rule>(rule: &R, grid: Grid3, frame: u32, cells: &[u32], params: &[u32]) -> Vec<u32> {
        sand3::reference(rule, grid, frame, cells, params)
    }
    match which {
        Which::Sand => one(&Sand, grid, frame, cells, params),
        Which::Fire => one(&Heat, grid, frame, cells, params),
        Which::Code => one(&Code, grid, frame, cells, params),
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
    for which in Which::ALL {
        let mut session = Session::open(&host, setup(which)).unwrap();
        let mut cells = session.state().unwrap();
        let camera = looking(&session);
        let params = session.example.params.clone();
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
    for which in Which::ALL {
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
    text.push(
        "FIRE \u{b7} 144 FPS \u{b7} ACTIVE 12 OCTS (0.25%) \u{b7} PAUSED \u{b7} DRAG TO ORBIT, CLICK TO USE THE TOOL, H FOR HELP"
            .to_owned(),
    );
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
