use std::f32::consts::FRAC_PI_2;

use grit::render3::{Camera, REACH, Screen};
use grit::sand::Grid;

use super::scenes::Which;
use super::session::VIEW;

/// How far a camera may tilt above the horizon while it orbits, in radians;
/// the floor may stop it sooner.
pub const BELOW: f32 = 1.2;

/// The closest an orbit stands to its target, in cells.
pub const NEAR: f32 = 2.0;

/// The farthest an orbit stands from its target, in sides of the world.
pub const FAR_SIDES: f32 = 4.0;

/// How far above the floor under the world the eye stays, in cells.
pub const FLOOR_GAP: f32 = 1.0;

/// How far inside [`Camera::kept`]'s box the eye stays, in cells.
pub const MARGIN: f32 = 1.0;

/// How much a dragged pixel turns the camera, in radians.
pub const TURN: f32 = 0.006;

/// How fast W, A, S, D, Q and E move the target of an orbit, in its
/// distance a second.
pub const WALK: f32 = 0.8;

/// How fast they fly the eye, in sides of the world a second.
pub const FLY: f32 = 0.2;

/// How much faster they go with Shift held.
pub const FAST: f32 = 4.0;

/// What one notch of the wheel leaves of the distance, zooming in.
pub const ZOOM: f32 = 0.85;

/// The most notches of the wheel one frame takes.
pub const MOST_NOTCHES: f32 = 4.0;

/// How long the shown camera takes to cover all but a share of `1 / e` of
/// the way to the one asked for, in seconds.
pub const SMOOTH: f32 = 0.06;

/// The longest frame time the camera moves by, in seconds.
pub const LONGEST: f32 = 0.1;

/// How the camera moves: around a target, or flying on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Orbit,
    Fly,
}

/// Where a camera looks, how far it stands from that point, and its turn
/// and tilt. The eye stands `far` behind `target`, against its forward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub far: f32,
}

fn added(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn less(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn times(value: [f32; 3], by: f32) -> [f32; 3] {
    [value[0] * by, value[1] * by, value[2] * by]
}

/// The dot product of `left` and `right`.
#[must_use]
pub fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn between(from: f32, to: f32, share: f32) -> f32 {
    from + (to - from) * share
}

fn between3(from: [f32; 3], to: [f32; 3], share: f32) -> [f32; 3] {
    [0, 1, 2].map(|axis| {
        between(
            grit::render3::of(from, axis),
            grit::render3::of(to, axis),
            share,
        )
    })
}

/// The box the eye stays in on `grid`: inside [`Camera::kept`]'s box by
/// [`MARGIN`], and above the floor by [`FLOOR_GAP`].
#[must_use]
pub fn eye_box(grid: Grid) -> ([f32; 3], [f32; 3]) {
    let sides = [grid.wide, grid.deep, grid.tall].map(|side| side as f32);
    let reach = REACH as f32;
    let low = sides.map(|side| side - reach + 1.0 + MARGIN);
    let mut high = [reach - 1.0 - MARGIN; 3];
    high[1] = high[1].min(sides[1] - FLOOR_GAP);
    (low, high)
}

/// The box an orbit's target stays in on `grid`: the eye's box, [`NEAR`]
/// inside it, so an eye can always stand that far from the target.
#[must_use]
pub fn target_box(grid: Grid) -> ([f32; 3], [f32; 3]) {
    let (low, high) = eye_box(grid);
    (low.map(|at| at + NEAR), high.map(|at| at - NEAR))
}

fn clamped(point: [f32; 3], (low, high): ([f32; 3], [f32; 3])) -> [f32; 3] {
    [0, 1, 2].map(|axis| {
        let at = grit::render3::of(point, axis);
        let low = grit::render3::of(low, axis);
        let high = grit::render3::of(high, axis);
        at.clamp(low, high.max(low))
    })
}

/// How far from `from` along `way` a point stays within `bounds`.
fn room(from: [f32; 3], way: [f32; 3], (low, high): ([f32; 3], [f32; 3])) -> f32 {
    (0..3).fold(f32::INFINITY, |most, axis| {
        let along = grit::render3::of(way, axis);
        let at = grit::render3::of(from, axis);
        let left = if along > 0.0 {
            (grit::render3::of(high, axis) - at) / along
        } else if along < 0.0 {
            (grit::render3::of(low, axis) - at) / along
        } else {
            f32::INFINITY
        };
        most.min(left.max(0.0))
    })
}

impl View {
    /// The fixed framings of each scene, for pictures: an orbit's turn,
    /// tilt and distance in sides of the world, and its target's height.
    /// Framing 0 is where the demo starts and where R takes it back.
    #[must_use]
    pub fn framed(which: Which, camera: usize, grid: Grid) -> Self {
        let side = grid.wide as f32;
        let (yaw, pitch, far, height, across) = match (which, camera) {
            (Which::Valley, 0) => (-2.35, 0.56, 1.6, 0.7, [0.5, 0.5]),
            (Which::Valley, 1) => (-2.75, 0.3, 0.6, 0.66, [0.5, 0.56]),
            (Which::Valley, 3) => (-2.0, 0.42, 0.42, 0.66, [0.6, 0.36]),
            (Which::Valley, 5) => (-2.35, 0.5, 2.3, 0.56, [0.5, 0.5]),
            (Which::Valley, 6) => (-2.0, 0.95, 0.35, 0.6, [0.33, 0.47]),
            (Which::Valley, 7) => (-2.3, 0.36, 0.42, 0.78, [0.74, 0.62]),
            (_, 2) => (-2.35, 0.62, 3.0, 0.72, [0.5, 0.5]),
            (_, 4) => (-0.95, 0.16, 1.9, 0.62, [0.5, 0.5]),
            (Which::Sand, 0) => (-2.3, 0.6, 2.2, 0.74, [0.5, 0.5]),
            (Which::Sand, 1) => (-0.9, 0.42, 1.2, 0.8, [0.45, 0.45]),
            (Which::Fire, 0) => (-2.35, 0.58, 2.25, 0.7, [0.5, 0.5]),
            (Which::Fire, 1) => (-1.75, 0.36, 1.0, 0.72, [0.62, 0.45]),
            (Which::Code, 0) => (-2.4, 0.5, 2.0, 0.72, [0.5, 0.5]),
            (Which::Code, 1) => (-1.2, 0.3, 0.8, 0.78, [0.45, 0.5]),
            (Which::Valley, _) => (-2.0, 0.3, 0.6, 0.7, [0.5, 0.6]),
            (Which::Sand, _) => (2.45, 0.3, 0.75, 0.82, [1.0, 0.45]),
            (Which::Fire, _) => (-2.0, 0.3, 0.5, 0.72, [0.64, 0.44]),
            (Which::Code, _) => (-1.6, 0.15, 0.45, 0.8, [0.5, 0.6]),
        };
        Self {
            target: [across[0] * side, height * side, across[1] * side],
            yaw,
            pitch,
            far: far * side,
        }
    }

    /// The unit vector the camera looks along.
    #[must_use]
    pub fn forward(&self) -> [f32; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        [cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw]
    }

    /// Where the camera stands.
    #[must_use]
    pub fn eye(&self) -> [f32; 3] {
        less(self.target, times(self.forward(), self.far))
    }

    /// The renderer's camera of this view.
    #[must_use]
    pub fn camera(&self) -> Camera {
        Camera {
            at: self.eye(),
            yaw: self.yaw,
            pitch: self.pitch,
            view: VIEW,
        }
    }

    /// This view held within what `mode` allows on `grid`: an orbit keeps
    /// its target, pitch and distance in their ranges and its eye above
    /// the floor and in its box; a flight keeps its eye in its box.
    #[must_use]
    pub fn held(mut self, mode: Mode, grid: Grid) -> Self {
        let farthest = (FAR_SIDES * grid.wide.max(grid.tall).max(grid.deep) as f32).max(NEAR);
        self.far = self.far.clamp(NEAR, farthest);
        let eyes = eye_box(grid);
        match mode {
            Mode::Orbit => {
                self.target = clamped(self.target, target_box(grid));
                self.pitch = self.pitch.clamp(-BELOW, FRAC_PI_2);
                let floor = eyes.1[1];
                let least = ((self.target[1] - floor) / self.far)
                    .clamp(-1.0, 1.0)
                    .asin();
                self.pitch = self.pitch.max(least);
                let back = times(self.forward(), -1.0);
                self.far = self.far.min(room(self.target, back, eyes)).max(NEAR);
            }
            Mode::Fly => {
                self.pitch = self.pitch.clamp(-FRAC_PI_2, FRAC_PI_2);
                let eye = clamped(self.eye(), eyes);
                self.target = added(eye, times(self.forward(), self.far));
            }
        }
        self
    }

    /// Turned by a drag of `(across, down)` pixels: around the target in an
    /// orbit, around the eye in a flight.
    #[must_use]
    pub fn turned(mut self, mode: Mode, (across, down): (f32, f32)) -> Self {
        let eye = self.eye();
        self.yaw += across * TURN;
        self.pitch += down * TURN;
        if mode == Mode::Fly {
            self.pitch = self.pitch.clamp(-FRAC_PI_2, FRAC_PI_2);
            self.target = added(eye, times(self.forward(), self.far));
        }
        self
    }

    /// Moved across the view by a drag of `(across, down)` pixels of
    /// `window`, so the point at the target's distance under the cursor
    /// stays under it.
    #[must_use]
    pub fn panned(mut self, window: Screen, (across, down): (f32, f32)) -> Self {
        let [_, right, up] = self.camera().basis();
        let per_pixel = 2.0 * self.far * self.camera().spread(window);
        let by = added(
            times(right, -across * per_pixel),
            times(up, down * per_pixel),
        );
        self.target = added(self.target, by);
        self
    }

    /// Moved by `push`, right, up and forward, each between -1 and 1, for
    /// `seconds`: an orbit's target over the ground, as the view faces, and
    /// up and down; a flight's eye along its own forward and right, and up
    /// and down.
    #[must_use]
    pub fn walked(
        mut self,
        mode: Mode,
        push: [f32; 3],
        fast: bool,
        seconds: f32,
        side: f32,
    ) -> Self {
        let seconds = seconds.clamp(0.0, LONGEST);
        let speed = match mode {
            Mode::Orbit => WALK * self.far,
            Mode::Fly => FLY * side,
        } * if fast { FAST } else { 1.0 };
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let right = [-sin_yaw, 0.0, cos_yaw];
        let ahead = match mode {
            Mode::Orbit => [cos_yaw, 0.0, sin_yaw],
            Mode::Fly => self.forward(),
        };
        let up = [0.0, -1.0, 0.0];
        let way = added(
            added(times(right, push[0]), times(up, push[1])),
            times(ahead, push[2]),
        );
        self.target = added(self.target, times(way, speed * seconds));
        self
    }

    /// Zoomed in by `notches` of the wheel, out when negative, toward
    /// `toward` when given and toward the target when not: the eye and the
    /// target both close in on that point, so it keeps its place on the
    /// screen.
    #[must_use]
    pub fn zoomed(mut self, notches: f32, toward: Option<[f32; 3]>, grid: Grid) -> Self {
        let farthest = (FAR_SIDES * grid.wide.max(grid.tall).max(grid.deep) as f32).max(NEAR);
        let notches = notches.clamp(-MOST_NOTCHES, MOST_NOTCHES);
        let mut share = ZOOM.powf(notches);
        if self.far > 0.0 {
            share = share.clamp(NEAR / self.far, farthest / self.far);
        }
        if let Some(point) = toward {
            self.target = added(point, times(less(self.target, point), share));
        }
        self.far *= share;
        self
    }

    /// The direction of the ray through `cursor`, in pixels of `window`.
    #[must_use]
    pub fn ray(&self, window: Screen, (x, y): (f32, f32)) -> [f32; 3] {
        let camera = self.camera();
        let [forward, right, up] = camera.basis();
        let spread = camera.spread(window);
        let across = (2.0 * x - window.wide as f32) * spread;
        let down = (window.high as f32 - 2.0 * y) * spread;
        added(forward, added(times(right, across), times(up, down)))
    }

    /// Where `point` lies on `window`, in pixels, when it lies ahead.
    #[allow(dead_code)]
    #[must_use]
    pub fn pixel(&self, window: Screen, point: [f32; 3]) -> Option<(f32, f32)> {
        let camera = self.camera();
        let [forward, right, up] = camera.basis();
        let spread = camera.spread(window);
        let away = less(point, self.eye());
        let depth = dot(away, forward);
        if depth <= 0.0 || spread <= 0.0 {
            return None;
        }
        let across = dot(away, right) / (depth * spread);
        let down = dot(away, up) / (depth * spread);
        Some((
            (across + window.wide as f32) / 2.0,
            (window.high as f32 - down) / 2.0,
        ))
    }

    /// The point on the ray through `cursor` as far ahead as `picked`: the
    /// point a zoom closes in on so what lies under the cursor stays there.
    #[must_use]
    pub fn anchor(&self, window: Screen, cursor: (f32, f32), picked: [f32; 3]) -> [f32; 3] {
        let forward = self.forward();
        let ray = self.ray(window, cursor);
        let depth = dot(less(picked, self.eye()), forward);
        let along = dot(ray, forward);
        if depth <= 0.0 || along <= 0.0 {
            return picked;
        }
        added(self.eye(), times(ray, depth / along))
    }
}

/// The camera the demo shows: the view the user asks for, and the view
/// shown, which follows it smoothly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rig {
    pub mode: Mode,
    pub goal: View,
    pub shown: View,
    grid: Grid,
}

impl Rig {
    /// A rig at `home` on `grid`, orbiting and still.
    #[must_use]
    pub fn new(home: View, grid: Grid) -> Self {
        let home = home.held(Mode::Orbit, grid);
        Self {
            mode: Mode::Orbit,
            goal: home,
            shown: home,
            grid,
        }
    }

    /// The rig back at `home` on `grid`, at once.
    pub fn reset(&mut self, home: View, grid: Grid) {
        *self = Self::new(home, grid);
    }

    /// The renderer's camera of the shown view.
    #[must_use]
    pub fn camera(&self) -> Camera {
        self.shown.camera()
    }

    fn asked(&mut self, view: View) {
        self.goal = view.held(self.mode, self.grid);
    }

    /// Turns by a drag of `by` pixels.
    pub fn turn(&mut self, by: (f32, f32)) {
        self.asked(self.goal.turned(self.mode, by));
    }

    /// Pans by a drag of `by` pixels of `window`.
    pub fn pan(&mut self, window: Screen, by: (f32, f32)) {
        self.asked(self.goal.panned(window, by));
    }

    /// Moves by `push` for `seconds`; see [`View::walked`].
    pub fn walk(&mut self, push: [f32; 3], fast: bool, seconds: f32) {
        if push == [0.0; 3] {
            return;
        }
        let side = self.grid.wide as f32;
        self.asked(self.goal.walked(self.mode, push, fast, seconds, side));
    }

    /// Zooms by `notches` toward `toward`; see [`View::zoomed`].
    pub fn zoom(&mut self, notches: f32, toward: Option<[f32; 3]>) {
        self.asked(self.goal.zoomed(notches, toward, self.grid));
    }

    /// Flies from an orbit, or orbits again from a flight: around the point
    /// straight ahead, at the distance of `ahead` when the middle of the
    /// screen sees a cell there.
    pub fn switch(&mut self, ahead: Option<[f32; 3]>) {
        match self.mode {
            Mode::Orbit => {
                self.mode = Mode::Fly;
                self.goal = self.goal.held(Mode::Fly, self.grid);
                self.shown = self.shown.held(Mode::Fly, self.grid);
            }
            Mode::Fly => {
                self.mode = Mode::Orbit;
                for view in [&mut self.goal, &mut self.shown] {
                    let eye = view.eye();
                    let forward = view.forward();
                    let depth = ahead.map_or(view.far, |point| dot(less(point, eye), forward));
                    let far = if depth > NEAR { depth } else { view.far };
                    *view = View {
                        target: added(eye, times(forward, far)),
                        far,
                        ..*view
                    }
                    .held(Mode::Orbit, self.grid);
                }
            }
        }
    }

    /// One frame of `seconds` with the moving keys pushing by `push`: the
    /// keys move the goal, and the shown view follows it. Nothing else
    /// moves the camera from frame to frame.
    pub fn tick(&mut self, push: [f32; 3], fast: bool, seconds: f32) {
        self.walk(push, fast, seconds);
        self.follow(seconds);
    }

    /// Moves the shown view toward the goal for a frame of `seconds`, by a
    /// share that depends only on the time, so the camera moves alike at
    /// any frame rate, and never by more than [`LONGEST`] allows.
    pub fn follow(&mut self, seconds: f32) {
        let seconds = seconds.clamp(0.0, LONGEST);
        let share = 1.0 - (-seconds / SMOOTH).exp();
        let from = self.shown;
        let to = self.goal;
        let yaw = between(from.yaw, to.yaw, share);
        let pitch = between(from.pitch, to.pitch, share);
        let far = between(from.far, to.far, share);
        let mut next = View {
            target: between3(from.target, to.target, share),
            yaw,
            pitch,
            far,
        };
        if self.mode == Mode::Fly {
            let eye = between3(from.eye(), to.eye(), share);
            next.target = added(eye, times(next.forward(), far));
        }
        self.shown = next.held(self.mode, self.grid);
    }
}
