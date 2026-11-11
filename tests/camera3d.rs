#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

#[path = "../src/bin/grit3d/camera.rs"]
#[allow(dead_code)]
mod camera;
#[path = "../src/bin/grit3d/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/bin/grit3d/life.rs"]
#[allow(dead_code)]
mod life;
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

use std::f32::consts::FRAC_PI_2;

use camera::{
    BELOW, FAR_SIDES, FAST, FLOOR_GAP, FLY, LONGEST, Mode, NEAR, Rig, View, WALK, eye_box,
};
use grit::render3::Screen;
use grit::sand::Grid;
use scenes::Which;
use ui::{Action, BINDINGS, HELP, HINT, Input};

const WINDOW: Screen = Screen {
    wide: 1280,
    high: 720,
};

/// How far, in window pixels, a point a zoom closes in on may drift from
/// under the cursor.
const PIXELS: f32 = 0.05;

fn grid(side: u32) -> Grid {
    Grid::solid(side, side, side).unwrap()
}

struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1_u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn below(&mut self, count: u64) -> u64 {
        self.next() % count
    }
}

fn near(left: [f32; 3], right: [f32; 3], within: f32) -> bool {
    (0..3).all(|axis| (left[axis] - right[axis]).abs() <= within)
}

fn home(which: Which, side: u32) -> View {
    View::framed(which, 0, grid(side)).held(Mode::Orbit, grid(side))
}

/// The camera a view gives meets every bound the renderer keeps: kept
/// leaves it where it is, its rays fit the exact march, and the eye stays
/// above the floor.
fn check(view: &View, mode: Mode, grid: Grid, what: &str) {
    let camera = view.camera();
    assert!(
        camera.at.iter().all(|at| at.is_finite()) && view.far.is_finite(),
        "{what}: {view:?}"
    );
    assert_eq!(camera.kept(grid), camera, "{what}: kept moves {view:?}");
    assert!(
        camera.rays(grid, WINDOW).fit(grid, WINDOW),
        "{what}: the rays do not fit {view:?}"
    );
    assert!(
        camera.at[1] <= grid.deep as f32 - FLOOR_GAP + 1e-3,
        "{what}: the eye is under the floor {view:?}"
    );
    let (low, high) = match mode {
        Mode::Orbit => (-BELOW, FRAC_PI_2),
        Mode::Fly => (-FRAC_PI_2, FRAC_PI_2),
    };
    assert!(
        view.pitch >= low - 1e-6 && view.pitch <= high + 1e-6,
        "{what}: pitch {} out of range",
        view.pitch
    );
    assert!(view.far >= NEAR - 1e-3, "{what}: far {}", view.far);
}

#[test]
fn a_pan_moves_the_target_across_the_view_and_keeps_the_grabbed_point_under_the_cursor() {
    let mut dice = Dice(0x9E37_79B9_7F4A_7C15);
    for which in Which::ALL {
        let view = home(which, 256);
        for _ in 0..50 {
            let by = (dice.between(-300.0, 300.0), dice.between(-200.0, 200.0));
            let panned = view.panned(WINDOW, by);
            let [_, right, up] = view.camera().basis();
            let per_pixel = 2.0 * view.far * view.camera().spread(WINDOW);
            let expected = [0, 1, 2].map(|axis| {
                view.target[axis] - right[axis] * by.0 * per_pixel + up[axis] * by.1 * per_pixel
            });
            assert!(
                near(panned.target, expected, 1e-2),
                "{which:?}: {:?} against {expected:?}",
                panned.target
            );
            assert_eq!(
                (panned.yaw, panned.pitch, panned.far),
                (view.yaw, view.pitch, view.far)
            );
            let cursor = (dice.between(100.0, 1180.0), dice.between(100.0, 620.0));
            let grabbed = view.anchor(WINDOW, cursor, view.target);
            let now = panned.pixel(WINDOW, grabbed).unwrap();
            let moved = (cursor.0 + by.0, cursor.1 + by.1);
            assert!(
                (now.0 - moved.0).abs() < PIXELS && (now.1 - moved.1).abs() < PIXELS,
                "{which:?}: the grabbed point is at {now:?}, the cursor at {moved:?}"
            );
        }
    }
}

#[test]
fn the_keys_move_an_orbit_over_the_ground_and_a_flight_along_its_view() {
    let side = 256.0;
    let view = home(Which::Valley, 256);
    let seconds = 0.05;
    let ground = [view.yaw.cos(), 0.0, view.yaw.sin()];
    let right = [-view.yaw.sin(), 0.0, view.yaw.cos()];
    let up = [0.0, -1.0, 0.0];
    let cases = [
        ([0.0, 0.0, 1.0], ground),
        ([0.0, 0.0, -1.0], ground.map(|value| -value)),
        ([1.0, 0.0, 0.0], right),
        ([-1.0, 0.0, 0.0], right.map(|value| -value)),
        ([0.0, 1.0, 0.0], up),
        ([0.0, -1.0, 0.0], up.map(|value| -value)),
    ];
    for (push, way) in cases {
        for fast in [false, true] {
            let speed = WALK * view.far * if fast { FAST } else { 1.0 };
            let walked = view.walked(Mode::Orbit, push, fast, seconds, side);
            let expected = [0, 1, 2].map(|axis| view.target[axis] + way[axis] * speed * seconds);
            assert!(near(walked.target, expected, 1e-3), "orbit {push:?}");
            assert_eq!(walked.far, view.far);
            assert_eq!(
                walked.eye()[1] - view.eye()[1],
                walked.target[1] - view.target[1]
            );
        }
    }
    let forward = view.forward();
    for (push, way) in [
        ([0.0, 0.0, 1.0], forward),
        ([1.0, 0.0, 0.0], right),
        ([0.0, 1.0, 0.0], up),
    ] {
        let walked = view.walked(Mode::Fly, push, false, seconds, side);
        let speed = FLY * side;
        let expected = [0, 1, 2].map(|axis| view.eye()[axis] + way[axis] * speed * seconds);
        assert!(near(walked.eye(), expected, 1e-2), "fly {push:?}");
    }
    let ahead = view.walked(Mode::Orbit, [0.0, 0.0, 1.0], false, seconds, side);
    let along = camera::dot(
        [
            ahead.target[0] - view.target[0],
            0.0,
            ahead.target[2] - view.target[2],
        ],
        ground,
    );
    assert!(along > 0.0, "W moves the target backwards");
}

#[test]
fn the_rig_moves_by_the_keys_through_the_same_path_the_window_calls() {
    let mut rig = Rig::new(home(Which::Sand, 128), grid(128));
    let before = rig.goal;
    rig.walk([0.0, 0.0, 1.0], false, 0.04);
    let ground = [before.yaw.cos(), 0.0, before.yaw.sin()];
    let moved = [0, 1, 2].map(|axis| rig.goal.target[axis] - before.target[axis]);
    let expected = WALK * before.far * 0.04;
    assert!(
        (camera::dot(moved, ground) - expected).abs() < 1e-3,
        "W moved the goal {moved:?}"
    );
    for _ in 0..200 {
        rig.follow(1.0 / 60.0);
    }
    assert!(near(rig.shown.target, rig.goal.target, 1e-3));
}

#[test]
fn each_moving_key_moves_the_target_the_way_the_help_says() {
    let start = Rig::new(home(Which::Valley, 256), grid(256));
    let view = start.goal;
    let ground = [view.yaw.cos(), 0.0, view.yaw.sin()];
    let right = [-view.yaw.sin(), 0.0, view.yaw.cos()];
    let up = [0.0, -1.0, 0.0];
    let flip = |way: [f32; 3]| way.map(|value| -value);
    let seconds = 0.05;
    for (input, way) in [
        (Input::W, ground),
        (Input::S, flip(ground)),
        (Input::D, right),
        (Input::A, flip(right)),
        (Input::E, up),
        (Input::Q, flip(up)),
    ] {
        let action = ui::meaning(input).unwrap();
        assert!(action.held(), "{input:?} must last while held");
        let mut rig = start;
        rig.walk(action.push(), false, seconds);
        let moved = [0, 1, 2].map(|axis| rig.goal.target[axis] - view.target[axis]);
        let expected = WALK * view.far * seconds;
        assert!(
            (camera::dot(moved, way) - expected).abs() < 1e-3,
            "{input:?} moved the target by {moved:?}, not {expected} along {way:?}"
        );
        assert!(
            near(moved, way.map(|value| value * expected), 1e-3),
            "{input:?} moved the target sideways: {moved:?}"
        );
    }
    let mut flying = start;
    flying.switch(None);
    let eye = flying.goal.eye();
    flying.walk(ui::meaning(Input::W).unwrap().push(), false, seconds);
    let gone = [0, 1, 2].map(|axis| flying.goal.eye()[axis] - eye[axis]);
    assert!(
        camera::dot(gone, view.forward()) > 0.99 * FLY * 256.0 * seconds,
        "W does not fly ahead: {gone:?}"
    );
}

#[test]
fn a_zoom_keeps_the_point_under_the_cursor_where_it_was() {
    let mut dice = Dice(0x2545_F491_4F6C_DD1D);
    let mut checked = 0;
    let mut bounded = 0;
    for which in Which::ALL {
        for side in [64, 256] {
            let grid = grid(side);
            let mut rig = Rig::new(home(which, side), grid);
            let mut here = 0;
            while here < 200 {
                let cursor = (dice.between(0.0, 1280.0), dice.between(0.0, 720.0));
                let view = rig.shown;
                let ray = view.ray(WINDOW, cursor);
                let depth = dice.between(NEAR * 2.0, view.far * 1.5);
                let picked = [0, 1, 2]
                    .map(|axis| view.eye()[axis] + ray[axis] * depth + dice.between(-0.5, 0.5));
                if !picked.iter().all(|&at| at > 0.0 && at < side as f32) {
                    continue;
                }
                let anchor = view.anchor(WINDOW, cursor, picked);
                let notches = dice.between(-3.0, 3.0);
                let free = rig.goal.zoomed(notches, Some(anchor), grid);
                rig.zoom(notches, Some(anchor));
                let held = rig.goal != free;
                bounded += usize::from(held);
                for frame in 0..40 {
                    rig.follow(if frame % 7 == 0 { 0.09 } else { 1.0 / 144.0 });
                    if held {
                        continue;
                    }
                    let shown = rig.shown.pixel(WINDOW, anchor).unwrap();
                    assert!(
                        (shown.0 - cursor.0).abs() < PIXELS && (shown.1 - cursor.1).abs() < PIXELS,
                        "{which:?} {side}: the zoom moved the point to {shown:?} from {cursor:?}"
                    );
                }
                checked += usize::from(!held);
                here += 1;
                rig.goal = rig.shown;
            }
        }
    }
    println!("{checked} zooms kept the point under the cursor; {bounded} reached a bound");
    assert!(
        bounded * 4 < checked,
        "too few zooms were free of the bounds"
    );
}

#[test]
fn a_zoom_over_empty_space_closes_in_on_the_target() {
    let view = home(Which::Valley, 256);
    let zoomed = view.zoomed(2.0, None, grid(256));
    assert_eq!(zoomed.target, view.target);
    assert!((zoomed.far - view.far * camera::ZOOM.powi(2)).abs() < 1e-3);
    let out = view.zoomed(-2.0, None, grid(256));
    assert!(out.far > view.far);
}

#[test]
fn the_distance_reaches_a_single_tree_and_the_whole_diorama() {
    let side = 256;
    let mut rig = Rig::new(home(Which::Valley, side), grid(side));
    for _ in 0..200 {
        rig.zoom(4.0, None);
    }
    assert!((rig.goal.far - NEAR).abs() < 1e-3, "{}", rig.goal.far);
    for _ in 0..200 {
        rig.zoom(-4.0, None);
    }
    assert!(
        rig.goal.far >= 3.0 * side as f32,
        "the farthest view stands {} cells off",
        rig.goal.far
    );
    assert!(rig.goal.far <= FAR_SIDES * side as f32 + 1e-3);
    check(&rig.goal, Mode::Orbit, grid(side), "far out");
}

#[test]
fn the_pitch_reaches_straight_down_and_never_flips() {
    let side = 256;
    let mut rig = Rig::new(home(Which::Valley, side), grid(side));
    let right = rig.goal.camera().basis()[1];
    for _ in 0..100 {
        rig.turn((0.0, 400.0));
        check(&rig.goal, Mode::Orbit, grid(side), "down");
    }
    assert!(
        (rig.goal.pitch - FRAC_PI_2).abs() < 1e-6,
        "{}",
        rig.goal.pitch
    );
    let [forward, now_right, _] = rig.goal.camera().basis();
    assert!(forward[1] > 0.999, "not straight down: {forward:?}");
    assert!(near(right, now_right, 1e-6), "the view flipped");
    for _ in 0..100 {
        rig.turn((0.0, -400.0));
        check(&rig.goal, Mode::Orbit, grid(side), "up");
    }
    assert!(rig.goal.pitch < 0.0, "the pitch stopped above the horizon");
    let floor = grid(side).deep as f32 - FLOOR_GAP;
    assert!(rig.goal.eye()[1] <= floor + 1e-3);
    rig.zoom(-4.0, None);
    for _ in 0..30 {
        rig.zoom(-4.0, None);
        rig.turn((0.0, -400.0));
        check(&rig.goal, Mode::Orbit, grid(side), "up and out");
        assert!(rig.goal.eye()[1] <= floor + 1e-3, "{:?}", rig.goal.eye());
    }
    let low = rig.goal;
    assert!(
        low.pitch > -BELOW + 1e-3,
        "far out, the floor limits the pitch: {}",
        low.pitch
    );
    assert!(
        (low.eye()[1] - floor).abs() < 1e-2,
        "the floor stops the eye: {:?}",
        low.eye()
    );
}

#[test]
fn flying_turns_around_the_eye_and_orbits_again_around_the_point_ahead() {
    let side = 256;
    let mut rig = Rig::new(home(Which::Fire, side), grid(side));
    let eye = rig.goal.eye();
    rig.switch(None);
    assert_eq!(rig.mode, Mode::Fly);
    assert!(near(rig.goal.eye(), eye, 1e-3));
    rig.turn((120.0, -80.0));
    assert!(near(rig.goal.eye(), eye, 1e-2), "looking moved the eye");
    rig.walk([0.0, 0.0, 1.0], false, LONGEST);
    let flown = rig.goal.eye();
    let ahead = rig.goal.forward();
    let gone = [0, 1, 2].map(|axis| flown[axis] - eye[axis]);
    assert!(camera::dot(gone, ahead) > 0.0);
    for _ in 0..300 {
        rig.follow(1.0 / 60.0);
    }
    let shown_eye = rig.shown.eye();
    let point = [0, 1, 2].map(|axis| shown_eye[axis] + ahead[axis] * 40.0);
    rig.switch(Some(point));
    assert_eq!(rig.mode, Mode::Orbit);
    assert!(near(rig.shown.eye(), shown_eye, 1e-2), "the eye jumped");
    assert!(
        near(rig.shown.target, point, 1e-2),
        "{:?}",
        rig.shown.target
    );
    assert!((rig.shown.far - 40.0).abs() < 1e-2);
}

#[test]
fn a_flight_may_enter_the_world() {
    let side = 64;
    let mut rig = Rig::new(home(Which::Code, side), grid(side));
    rig.switch(None);
    for _ in 0..200 {
        rig.walk([0.0, 0.0, 1.0], true, LONGEST);
        rig.follow(LONGEST);
        let eye = rig.shown.eye();
        check(&rig.shown, Mode::Fly, grid(side), "flying in");
        if eye.iter().all(|&at| at > 1.0 && at < side as f32 - 1.0) {
            return;
        }
    }
    panic!("the flight never entered the world: {:?}", rig.shown.eye());
}

#[test]
fn the_camera_never_moves_on_its_own_and_r_takes_it_home() {
    for which in Which::ALL {
        let side = 128;
        let start = home(which, side);
        assert_eq!(start.target, View::framed(which, 0, grid(side)).target);
        let mut rig = Rig::new(start, grid(side));
        for _ in 0..10_000 {
            rig.tick([0.0; 3], false, 1.0 / 60.0);
        }
        assert_eq!(rig.shown, start, "{which:?}: the camera moved on its own");
        assert_eq!(rig.goal, start, "{which:?}: the camera moved on its own");
        rig.turn((50.0, 10.0));
        rig.walk([1.0, 0.0, 1.0], true, 0.1);
        rig.zoom(2.0, None);
        for _ in 0..600 {
            rig.follow(1.0 / 60.0);
        }
        let left = rig.shown;
        for _ in 0..10_000 {
            rig.tick([0.0; 3], true, 1.0 / 60.0);
        }
        assert!(
            near(rig.shown.eye(), left.eye(), 1e-3),
            "{which:?}: it drifted"
        );
        rig.switch(None);
        rig.reset(start, grid(side));
        assert_eq!(rig.mode, Mode::Orbit);
        assert_eq!((rig.shown, rig.goal), (start, start));
    }
}

#[test]
fn the_camera_moves_alike_at_any_frame_rate_and_a_long_frame_does_not_jump_it() {
    let side = 256;
    let start = Rig::new(home(Which::Valley, side), grid(side));
    let mut goal = start;
    goal.turn((200.0, 30.0));
    goal.zoom(3.0, None);
    let mut slow = goal;
    let mut quick = goal;
    for _ in 0..3 {
        slow.follow(1.0 / 30.0);
    }
    for _ in 0..12 {
        quick.follow(1.0 / 120.0);
    }
    assert!(
        near(slow.shown.eye(), quick.shown.eye(), 1e-2),
        "{:?} at 30 frames a second, {:?} at 120",
        slow.shown.eye(),
        quick.shown.eye()
    );
    assert!((slow.shown.yaw - quick.shown.yaw).abs() < 1e-4);
    let mut spike = goal;
    let mut capped = goal;
    spike.follow(3.0);
    capped.follow(LONGEST);
    assert_eq!(spike.shown, capped.shown, "a long frame moves further");
    let moved = (spike.shown.yaw - start.shown.yaw) / (goal.goal.yaw - start.shown.yaw);
    assert!(moved < 1.0, "a long frame jumps all the way: {moved}");
    let mut walking = start;
    let mut held = start;
    walking.walk([0.0, 0.0, 1.0], false, 2.0);
    held.walk([0.0, 0.0, 1.0], false, LONGEST);
    assert_eq!(walking.goal, held.goal, "a long frame walks further");
}

#[test]
fn every_camera_the_controls_make_is_one_the_renderer_keeps() {
    let mut dice = Dice(0xD1B5_4A32_D192_ED03);
    let mut checked = 0;
    for side in [32, 64, 256, 1024] {
        let grid = grid(side);
        for which in Which::ALL {
            let start = home(which, side);
            let mut rig = Rig::new(start, grid);
            check(&rig.shown, Mode::Orbit, grid, "home");
            for _ in 0..600 {
                match dice.below(9) {
                    0 => rig.turn((dice.between(-900.0, 900.0), dice.between(-900.0, 900.0))),
                    1 => rig.pan(
                        WINDOW,
                        (dice.between(-2000.0, 2000.0), dice.between(-2000.0, 2000.0)),
                    ),
                    2 => {
                        let push = [0; 3].map(|_| dice.below(3) as f32 - 1.0);
                        rig.walk(push, dice.below(2) == 0, dice.between(0.0, 5.0));
                    }
                    3 => {
                        let toward = (dice.below(3) != 0).then(|| {
                            [0; 3].map(|_| dice.between(-2.0 * side as f32, 3.0 * side as f32))
                        });
                        rig.zoom(dice.between(-30.0, 30.0), toward);
                    }
                    4 => {
                        let ahead = (dice.below(2) == 0)
                            .then(|| [0; 3].map(|_| dice.between(0.0, side as f32)));
                        rig.switch(ahead);
                    }
                    5 if dice.below(10) == 0 => rig.reset(start, grid),
                    _ => rig.follow(dice.between(0.0, 0.5)),
                }
                check(&rig.goal, rig.mode, grid, "goal");
                check(&rig.shown, rig.mode, grid, "shown");
                checked += 1;
            }
        }
    }
    println!("{checked} random inputs, each camera kept");
}

#[test]
fn every_control_the_help_names_is_handled_and_no_input_means_two_things() {
    let mut named = Vec::new();
    for line in HELP.iter().skip(1).filter(|line| !line.is_empty()) {
        let keys = line
            .split("  ")
            .next()
            .filter(|keys| keys.len() < line.len())
            .unwrap_or_else(|| panic!("{line:?} has no keys column"));
        for part in keys.split(" / ") {
            let words: Vec<&str> = part.split(' ').collect();
            if let [low, "-", high] = words.as_slice() {
                let (low, high): (u32, u32) = (low.parse().unwrap(), high.parse().unwrap());
                named.extend((low..=high).map(Input::Digit));
            } else if let Some(input) = Input::named(part) {
                named.push(input);
            } else {
                for word in words {
                    named.push(
                        Input::named(word)
                            .unwrap_or_else(|| panic!("{word:?} in {line:?} is no input")),
                    );
                }
            }
        }
    }
    for input in &named {
        assert!(
            ui::meaning(*input).is_some(),
            "{input:?} is named but not handled"
        );
    }
    for (at, (input, _)) in BINDINGS.iter().enumerate() {
        assert!(named.contains(input), "{input:?} is handled but not named");
        assert!(
            BINDINGS
                .iter()
                .skip(at + 1)
                .all(|(other, _)| other != input),
            "{input:?} has two meanings"
        );
    }
    for part in HINT.split(", ") {
        let keys = part.split(' ').next().unwrap();
        let inputs: Vec<Input> = Input::named(keys).map_or_else(
            || {
                keys.chars()
                    .map(|key| Input::named(&key.to_string()).unwrap())
                    .collect()
            },
            |input| vec![input],
        );
        for input in inputs {
            assert!(ui::meaning(input).is_some(), "the hint names {input:?}");
        }
    }
    let kept = [
        (Input::Digit(0), Action::Scene(0)),
        (Input::Digit(3), Action::Scene(3)),
        (Input::Digit(4), Action::Tool(0)),
        (Input::Digit(8), Action::Tool(4)),
        (Input::R, Action::Restart),
        (Input::Space, Action::Pause),
        (Input::N, Action::Step),
        (Input::H, Action::Help),
        (Input::O, Action::Overlay),
        (Input::A, Action::Left),
        (Input::W, Action::Forward),
        (Input::F, Action::Fly),
        (Input::Wheel, Action::Zoom),
        (Input::ShiftDrag, Action::Pan),
        (Input::MiddleDrag, Action::Pan),
    ];
    for (input, action) in kept {
        assert_eq!(ui::meaning(input), Some(action), "{input:?}");
    }
    let held: Vec<Action> = BINDINGS
        .iter()
        .map(|&(_, action)| action)
        .filter(|action| action.held())
        .collect();
    assert_eq!(held.len(), 7, "the moving keys and Shift last while held");
}

#[test]
fn every_framing_starts_within_the_bounds() {
    for side in [32, 128, 256, 1024] {
        for which in Which::ALL {
            for at in 0..8 {
                let view = View::framed(which, at, grid(side)).held(Mode::Orbit, grid(side));
                check(&view, Mode::Orbit, grid(side), "framing");
            }
        }
    }
    let (low, high) = eye_box(grid(256));
    assert!(low.iter().zip(high).all(|(&low, high)| low < high));
}
