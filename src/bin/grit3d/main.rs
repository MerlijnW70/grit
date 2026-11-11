mod font;
mod life;
mod scenes;
mod session;
mod ui;
mod valley;

use std::time::{Duration, Instant};

use grit::Host;
use grit::render3::{self, Camera, Screen, Steps, stage};

use font::Canvas;
use scenes::Which;
use session::{Failed, Session, Setup, VIEW};

/// The frames of a timed run: warm-up, then timed.
const WARM: usize = 20;
const TIMED: usize = 60;

/// How long the demo waits without input before it orbits again.
const IDLE: Duration = Duration::from_secs(20);

/// How fast the orbit turns, in radians a second.
const TURN: f32 = 0.07;

/// The longest frame, on the device, a quality may take and still be kept.
const BUDGET: Duration = Duration::from_millis(26);

/// How a device draws: the side of the world, the window's pixels a
/// rendered pixel on each side, the sun rays of a face, and how many frames
/// apart the sun through the gas is found again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Quality {
    name: &'static str,
    side: u32,
    scale: u32,
    suns: u32,
    sunlit_every: u32,
}

const QUALITIES: [Quality; 3] = [
    Quality {
        name: "full",
        side: 256,
        scale: 1,
        suns: 2,
        sunlit_every: 1,
    },
    Quality {
        name: "half",
        side: 128,
        scale: 2,
        suns: 1,
        sunlit_every: 4,
    },
    Quality {
        name: "third",
        side: 128,
        scale: 3,
        suns: 1,
        sunlit_every: 8,
    },
];

/// A device with less memory of its own than this starts at the second
/// quality.
const SMALL_HEAP: u64 = 4 << 30;

#[derive(Debug, Clone)]
struct Asked {
    which: Which,
    side: Option<u32>,
    window: Screen,
    frames: Option<u32>,
    shot: Option<String>,
    camera: Option<usize>,
    measure: bool,
    suns: Option<u32>,
    quality: Option<usize>,
    dusk: Option<f32>,
    smooth: bool,
    clean: bool,
    active: bool,
    help: bool,
    gas: bool,
    tool: usize,
    skip: u32,
    trace: Option<u32>,
    cycle: bool,
    generate: Option<u32>,
}

fn asked() -> Result<Asked, Failed> {
    let mut held = Asked {
        which: Which::Valley,
        side: None,
        window: Screen {
            wide: 1280,
            high: 720,
        },
        frames: None,
        shot: None,
        camera: None,
        measure: false,
        suns: None,
        quality: None,
        dusk: None,
        smooth: true,
        clean: false,
        active: false,
        help: false,
        gas: true,
        tool: 0,
        skip: 0,
        trace: None,
        cycle: false,
        generate: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(one) = args.next() {
        let mut value = || -> Result<String, Failed> {
            args.next()
                .ok_or_else(|| format!("{one} needs a value").into())
        };
        match one.as_str() {
            "--scene" | "--rule" => {
                let name = value()?;
                held.which = Which::named(&name).ok_or(format!("no scene {name}"))?;
            }
            "--size" => held.side = Some(value()?.parse()?),
            "--width" => held.window.wide = value()?.parse()?,
            "--height" => held.window.high = value()?.parse()?,
            "--frames" => held.frames = Some(value()?.parse()?),
            "--shot" => held.shot = Some(value()?),
            "--camera" => held.camera = Some(value()?.parse()?),
            "--suns" => held.suns = Some(value()?.parse()?),
            "--quality" => {
                let name = value()?;
                held.quality = Some(
                    QUALITIES
                        .iter()
                        .position(|quality| quality.name == name)
                        .ok_or(format!("no quality {name}; full, half or third"))?,
                );
            }
            "--dusk" => held.dusk = Some(value()?.parse()?),
            "--smoothing" => held.smooth = value()? != "off",
            "--gas" => held.gas = value()? != "off",
            "--tool" => held.tool = value()?.parse()?,
            "--skip" => held.skip = value()?.parse()?,
            "--cycle" => held.cycle = true,
            "--trace" => held.trace = Some(value()?.parse()?),
            "--generate" => held.generate = Some(value()?.parse()?),
            "--clean" => held.clean = true,
            "--active" => held.active = true,
            "--help-panel" => held.help = true,
            "--measure" => held.measure = true,
            other => {
                return Err(format!(
                    "unknown argument {other}; grit3d takes --scene valley|sand|fire|code, --size, --width, --height, --frames N, --shot FILE.png, --camera N, --suns 1-4, --quality full|half|third, --dusk 0-1, --smoothing on|off, --gas on|off, --tool N, --clean, --active, --help-panel, --measure, --skip STEPS, --trace STEPS, --cycle and --generate SIDE"
                )
                .into());
            }
        }
    }
    Ok(held)
}

fn main() -> Result<(), Failed> {
    let asked = asked()?;
    if let Some(side) = asked.generate {
        return generated(side);
    }
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    if asked.measure {
        return measured(&host, &asked);
    }
    if let Some(steps) = asked.trace {
        return traced(&host, &asked, steps);
    }
    shown(&host, &asked)
}

/// Times the valley's generator at a side of `side`.
fn generated(side: u32) -> Result<(), Failed> {
    let grid = grit::sand::Grid::solid(side, side, side)?;
    let mut times = Vec::new();
    let mut made = None;
    for _ in 0..3 {
        let clock = Instant::now();
        made = Some(valley::made(grid, valley::SEED));
        times.push(clock.elapsed());
    }
    let made = made.ok_or("no valley was made")?;
    let hash = made
        .cells
        .iter()
        .fold(0xCBF2_9CE4_8422_2325_u64, |hash, &word| {
            (hash ^ u64::from(word)).wrapping_mul(0x0100_0000_01B3)
        });
    let wet = made.wet.iter().filter(|&&wet| wet > 0).count();
    let peak = made.ground.iter().max().copied().unwrap_or(0);
    let (least, middle) = milliseconds(&mut times);
    println!(
        "valley {side}^3 seed {:#x}: generated in {least:.0} ms at least, {middle:.0} ms the median of 3; world hash {hash:#018x}; peak {peak}, snowline {}, lake level {}, {wet} columns hold water",
        valley::SEED,
        made.snowline,
        made.level
    );
    Ok(())
}

/// The quality to draw at: the one asked for, or the first, from the one
/// the device's memory suggests, whose frame keeps within [`BUDGET`].
fn chosen(host: &Host, asked: &Asked) -> Result<Quality, Failed> {
    if let Some(at) = asked.quality {
        return Ok(QUALITIES.get(at).copied().unwrap_or(QUALITIES[0]));
    }
    let start = usize::from(host.heap() < SMALL_HEAP);
    for quality in QUALITIES.iter().skip(start) {
        let mut session = Session::cold(host, setup(asked, *quality))?;
        let camera = orbit_camera(&Orbit::start(asked.which, session.grid()), session.grid());
        let mut times = Vec::new();
        for at in 0..8 {
            session.aim(&camera, false, at == 0)?;
            let _ = session.frame(true, steps(&session, *quality, at == 0), false)?;
            if let Some(time) = session.held.on_device() {
                times.push(time);
            }
        }
        times.sort_unstable();
        let middle = times.get(times.len() / 2).copied().unwrap_or_default();
        println!(
            "quality {}: {:.2} ms a frame on the device",
            quality.name,
            middle.as_secs_f64() * 1000.0
        );
        if middle <= BUDGET {
            return Ok(*quality);
        }
    }
    Ok(QUALITIES.last().copied().unwrap_or(QUALITIES[0]))
}

fn setup(asked: &Asked, quality: Quality) -> Setup {
    let scale = quality.scale.max(1);
    Setup {
        which: asked.which,
        side: asked.side.unwrap_or(quality.side),
        screen: Screen {
            wide: (asked.window.wide / scale).max(1),
            high: (asked.window.high / scale).max(1),
        },
        suns: asked.suns.unwrap_or(quality.suns),
    }
}

fn steps(session: &Session<'_>, quality: Quality, first: bool) -> Steps {
    let gas = session.example.table.gases() != 0;
    let drawn = session.steps;
    Steps {
        refill: first,
        lamp: true,
        sunlit: gas && (first || drawn.is_multiple_of(quality.sunlit_every.max(1))),
    }
}

/// A slow orbit around the middle of the world: where it looks, how far it
/// stands, and its turn and tilt.
#[derive(Debug, Clone, Copy)]
struct Orbit {
    target: [f32; 3],
    yaw: f32,
    pitch: f32,
    far: f32,
}

impl Orbit {
    fn start(which: Which, grid: grit::sand::Grid) -> Self {
        if which == Which::Valley {
            return framed(which, 0, grid);
        }
        let side = grid.wide as f32;
        Self {
            target: [side * 0.5, side * 0.72, side * 0.5],
            yaw: -2.35,
            pitch: 0.6,
            far: side * 2.25,
        }
    }
}

fn orbit_camera(orbit: &Orbit, grid: grit::sand::Grid) -> Camera {
    let (sin_yaw, cos_yaw) = orbit.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = orbit.pitch.sin_cos();
    let at = [
        orbit.target[0] - cos_pitch * cos_yaw * orbit.far,
        orbit.target[1] - sin_pitch * orbit.far,
        orbit.target[2] - cos_pitch * sin_yaw * orbit.far,
    ];
    Camera {
        at,
        yaw: orbit.yaw,
        pitch: orbit.pitch,
        view: VIEW,
    }
    .kept(grid)
}

/// The fixed framings of each scene, for pictures: an orbit's turn, tilt
/// and distance in sides of the world, and its target's height.
fn framed(which: Which, camera: usize, grid: grit::sand::Grid) -> Orbit {
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
    Orbit {
        target: [across[0] * side, height * side, across[1] * side],
        yaw,
        pitch,
        far: far * side,
    }
}

/// Where each pixel of a line `to` long reads a line `from` long: the
/// first pixel, the next, and the share of the next, out of 256.
fn taps(from: u32, to: u32) -> Vec<(usize, usize, u32)> {
    let scale = from as f32 / to.max(1) as f32;
    (0..to)
        .map(|at| {
            let place = ((at as f32 + 0.5) * scale - 0.5).max(0.0);
            let low = (place as u32).min(from.saturating_sub(1));
            let high = (low + 1).min(from.saturating_sub(1));
            let share = ((place - low as f32) * 256.0).round().clamp(0.0, 256.0) as u32;
            (low as usize, high as usize, share)
        })
        .collect()
}

/// Two pixels, `0x00RRGGBB`, blended by `share` out of 256 toward `b`.
const fn blend(a: u32, b: u32, share: u32) -> u32 {
    let keep = 256 - share;
    let red_blue = ((a & 0x00FF_00FF) * keep + (b & 0x00FF_00FF) * share) >> 8;
    let green = ((a & 0x0000_FF00) * keep + (b & 0x0000_FF00) * share) >> 8;
    (red_blue & 0x00FF_00FF) | (green & 0x0000_FF00)
}

/// `picture`, drawn on `from`, stretched over `to` and blended between the
/// four pixels around each.
fn upscaled(picture: &[u32], from: Screen, to: Screen) -> Vec<u32> {
    if from == to {
        return picture.to_vec();
    }
    let across = taps(from.wide, to.wide);
    let down = taps(from.high, to.high);
    let wide = from.wide as usize;
    let get = |x: usize, y: usize| picture.get(y * wide + x).copied().unwrap_or(0);
    let mut out = Vec::with_capacity(to.pixels());
    for &(top, bottom, share_y) in &down {
        for &(left, right, share_x) in &across {
            let upper = blend(get(left, top), get(right, top), share_x);
            let lower = blend(get(left, bottom), get(right, bottom), share_x);
            out.push(blend(upper, lower, share_y));
        }
    }
    out
}

/// How many frames of a timed cycle are shown before the timing starts,
/// and how many after its restart are timed on their own.
const SETTLING: u32 = 30;
const RESTARTING: usize = 40;

/// The frame times of a whole cycle of a scene that lives on its own,
/// from the [`SETTLING`]th frame to its restart: the wall time between
/// frames in the window, the device time of each frame, and the frames of
/// each phase; then the frames right after the restart, on their own.
#[derive(Debug, Default)]
struct Timing {
    shown: u32,
    wall: Vec<Duration>,
    device: Vec<Duration>,
    stepping: Vec<Duration>,
    phases: [u32; 3],
    restart: Option<(u32, Vec<Duration>, Vec<Duration>)>,
}

impl Timing {
    /// Takes a frame: the wall time since the frame before, its device
    /// time, the cycle as it stands after it, whether it stepped the engine
    /// and whether the world restarted in it; true once the frames after
    /// the restart are timed too.
    fn add(
        &mut self,
        (wall, device): (Duration, Option<Duration>),
        life: Option<&life::Cycle>,
        (moving, restarted): (bool, bool),
        steps: u32,
    ) -> bool {
        self.shown += 1;
        if self.shown <= SETTLING {
            return false;
        }
        if let Some((_, walls, devices)) = self.restart.as_mut() {
            walls.push(wall);
            devices.extend(device);
            return walls.len() > RESTARTING;
        }
        self.wall.push(wall);
        self.device.extend(device);
        if moving {
            self.stepping.extend(device);
        }
        let phase = match life.map(|life| life.phase) {
            Some(life::Phase::Burning) => 1,
            Some(life::Phase::After) => 2,
            _ => 0,
        };
        if let Some(slot) = self.phases.get_mut(phase) {
            *slot += 1;
        }
        if restarted {
            self.restart = Some((steps, Vec::new(), Vec::new()));
        }
        false
    }

    fn report(&mut self) {
        let line = |name: &str, times: &mut Vec<Duration>| {
            times.sort_unstable();
            let ms = |at: Option<&Duration>| at.copied().unwrap_or_default().as_secs_f64() * 1000.0;
            println!(
                "cycle {name}: {} frames, min {:.2} ms, median {:.2} ms, 99th {:.2} ms, worst {:.2} ms",
                times.len(),
                ms(times.first()),
                ms(times.get(times.len() / 2)),
                ms(times.get(times.len() * 99 / 100)),
                ms(times.last()),
            );
        };
        line("window", &mut self.wall);
        line("device", &mut self.device);
        line("device, frames with an engine step", &mut self.stepping);
        println!(
            "cycle frames: calm {}, burning {}, after {}",
            self.phases[0], self.phases[1], self.phases[2]
        );
        if let Some((steps, walls, devices)) = &self.restart {
            let worst = |times: &[Duration]| {
                times
                    .iter()
                    .max()
                    .copied()
                    .unwrap_or_default()
                    .as_secs_f64()
                    * 1000.0
            };
            println!(
                "restart at engine step {steps}: of the {} frames after it, the worst took {:.2} ms in the window and {:.2} ms on the device",
                walls.len(),
                worst(walls),
                worst(devices)
            );
        }
    }
}

/// What the demo is doing between frames.
struct Play {
    orbit: Orbit,
    manual: bool,
    last_input: Instant,
    tool: usize,
    paused: bool,
    active: bool,
    help: bool,
    pressed_at: Option<(f32, f32)>,
    dragging: bool,
    last_mouse: Option<(f32, f32)>,
}

#[allow(clippy::too_many_lines)]
fn shown(host: &Host, asked: &Asked) -> Result<(), Failed> {
    let quality = chosen(host, asked)?;
    let window_screen = asked.window;
    let mut session = Session::open(host, setup(asked, quality))?;
    if !asked.gas {
        no_gas(&mut session)?;
    }
    if let Some(dusk) = asked.dusk {
        session.dusk = dusk;
    }
    session.advance(asked.skip)?;
    println!(
        "{} width {} {} {}^3 window {}x{} drawn at {}x{} quality {} suns {}",
        host.name(),
        host.width(),
        session.example.name,
        session.grid().wide,
        window_screen.wide,
        window_screen.high,
        session.build.screen.wide,
        session.build.screen.high,
        quality.name,
        session.build.suns
    );
    let fixed = asked
        .camera
        .map(|at| framed(asked.which, at, session.grid()));
    let mut play = Play {
        orbit: fixed.unwrap_or_else(|| Orbit::start(asked.which, session.grid())),
        manual: fixed.is_some(),
        last_input: Instant::now(),
        tool: asked.tool,
        paused: false,
        active: asked.active,
        help: asked.help,
        pressed_at: None,
        dragging: false,
        last_mouse: None,
    };
    let mut window = minifb::Window::new(
        "grit3d",
        window_screen.wide as usize,
        window_screen.high as usize,
        minifb::WindowOptions::default(),
    )?;
    window.set_target_fps(0);
    let mut shown = 0_u32;
    let mut fresh = true;
    let mut picture = Vec::new();
    let mut clock = Instant::now();
    let mut counted = 0_u32;
    let mut rate = 0.0;
    let mut active_count = 0_u32;
    let mut last_frame = Instant::now();
    let mut steady: Option<(Instant, u32)> = None;
    let mut owed = 0.0_f32;
    let mut timing = Timing::default();
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let now = Instant::now();
        let elapsed = now.duration_since(last_frame);
        let spent = elapsed.as_secs_f32().min(0.1);
        last_frame = now;
        let mut step = false;
        let mut switch_to = None;
        let mut restart = false;
        {
            use minifb::{Key, KeyRepeat, MouseButton, MouseMode};
            let mut touched = false;
            let pressed = |key| window.is_key_pressed(key, KeyRepeat::No);
            for (at, key) in [Key::Key0, Key::Key1, Key::Key2, Key::Key3]
                .into_iter()
                .enumerate()
            {
                if pressed(key) {
                    switch_to = Which::ALL.get(at).copied();
                    touched = true;
                }
            }
            for (at, key) in [Key::Key4, Key::Key5, Key::Key6, Key::Key7, Key::Key8]
                .into_iter()
                .enumerate()
                .take(session.example.tools.len())
            {
                if pressed(key) {
                    play.tool = at;
                    touched = true;
                }
            }
            if pressed(Key::R) {
                restart = true;
                touched = true;
            }
            if pressed(Key::Space) {
                play.paused = !play.paused;
                touched = true;
            }
            if pressed(Key::N) {
                step = true;
                touched = true;
            }
            if pressed(Key::A) {
                play.active = !play.active;
                touched = true;
            }
            if pressed(Key::H) {
                play.help = !play.help;
                touched = true;
            }
            if let Some((_, wheel)) = window.get_scroll_wheel()
                && wheel != 0.0
            {
                play.orbit.far = (play.orbit.far * (1.0 - wheel * 0.08))
                    .clamp(8.0, session.grid().wide as f32 * 3.6);
                touched = true;
            }
            let mouse = window.get_mouse_pos(MouseMode::Discard);
            let left = window.get_mouse_down(MouseButton::Left);
            let right = window.get_mouse_down(MouseButton::Right);
            let window_size = (window_screen.wide, window_screen.high);
            let tools = session.example.tools.clone();
            match (left, play.pressed_at, mouse) {
                (true, None, Some(at)) => {
                    play.pressed_at = Some(at);
                    play.dragging = false;
                    play.last_mouse = Some(at);
                    touched = true;
                }
                (true, Some(start), Some(at)) => {
                    if (at.0 - start.0).abs() + (at.1 - start.1).abs() > 5.0 {
                        play.dragging = true;
                    }
                    if play.dragging
                        && let Some(before) = play.last_mouse
                    {
                        play.orbit.yaw += (at.0 - before.0) * 0.006;
                        play.orbit.pitch =
                            (play.orbit.pitch + (at.1 - before.1) * 0.006).clamp(0.05, 1.45);
                    }
                    play.last_mouse = Some(at);
                    touched = true;
                }
                (false, Some(start), _) => {
                    if !play.dragging {
                        if let Some(tool) = ui::pressed(&tools, window_size, start) {
                            play.tool = tool;
                        } else {
                            use_tool(&mut session, start, quality, play.tool)?;
                        }
                    }
                    play.pressed_at = None;
                    play.dragging = false;
                    touched = true;
                }
                _ => {}
            }
            if right
                && let Some(at) = mouse
                && ui::pressed(&tools, window_size, at).is_none()
            {
                use_tool(&mut session, at, quality, play.tool)?;
                touched = true;
            }
            if touched {
                play.manual = true;
                play.last_input = Instant::now();
            }
        }
        if let Some(which) = switch_to.or(restart.then_some(session.setup.which)) {
            let mut wanted = setup(asked, quality);
            wanted.which = which;
            drop(session);
            session = Session::open(host, wanted)?;
            if let Some(dusk) = asked.dusk {
                session.dusk = dusk;
            }
            play.tool = 0;
            if let Some(which) = switch_to {
                play.orbit = Orbit::start(which, session.grid());
            }
            fresh = true;
        }
        if fixed.is_none() && play.manual && play.last_input.elapsed() >= IDLE {
            play.manual = false;
        }
        if !play.manual {
            play.orbit.yaw += TURN * spent;
        }
        let camera = orbit_camera(&play.orbit, session.grid());
        let mut light_steps = steps(&session, quality, fresh);
        light_steps.refill = fresh;
        if !asked.smooth {
            session.aim(&camera, play.active, true)?;
        } else {
            session.aim(&camera, play.active, fresh)?;
        }
        if let Some(pace) = session.example.pace {
            owed = (owed + pace * spent).min(1.0);
        } else {
            owed = 1.0;
        }
        let due = owed >= 1.0;
        if due && !play.paused {
            owed -= 1.0;
        }
        let moving = (!play.paused && due) || step;
        let (_, got) = session.frame(moving, light_steps, true)?;
        fresh = std::mem::take(&mut session.restarted);
        if asked.cycle
            && timing.add(
                (elapsed, session.held.on_device()),
                session.life.as_ref(),
                (moving, fresh),
                session.steps,
            )
        {
            timing.report();
            break;
        }
        let mut frame = if session.build.screen == window_screen {
            got
        } else {
            upscaled(&got, session.build.screen, window_screen)
        };
        counted += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            rate = f64::from(counted) / clock.elapsed().as_secs_f64();
            clock = Instant::now();
            counted = 0;
            if play.active {
                active_count = session.active()?;
            }
        }
        if !asked.clean {
            let mut canvas = Canvas {
                pixels: &mut frame,
                wide: window_screen.wide,
                high: window_screen.high,
            };
            let mut line = format!("{}  \u{b7}  {rate:.0} FPS  \u{b7}  ", session.example.name);
            if play.active {
                let octs = session.grid().blocks().max(1);
                line.push_str(&format!(
                    "ACTIVE {active_count} OCTS ({:.2}%)  \u{b7}  ",
                    f64::from(active_count) * 100.0 / f64::from(octs)
                ));
            }
            if play.paused {
                line.push_str("PAUSED  \u{b7}  ");
            }
            line.push_str("DRAG TO ORBIT, CLICK TO USE THE TOOL, H FOR HELP");
            ui::draw_corner(&mut canvas, &line);
            ui::draw_bar(&mut canvas, &session.example.tools, play.tool);
            if play.help {
                ui::draw_help(&mut canvas);
            }
        }
        window.update_with_buffer(
            &frame,
            window_screen.wide as usize,
            window_screen.high as usize,
        )?;
        picture = frame;
        shown += 1;
        if shown == 30 {
            steady = Some((Instant::now(), shown));
        }
        if asked.frames.is_some_and(|last| shown >= last) {
            break;
        }
    }
    if let Some((since, from)) = steady {
        let frames = shown - from;
        if frames > 0 {
            println!(
                "{frames} frames after the 30th in {:.2} s: {:.1} frames a second in the window",
                since.elapsed().as_secs_f64(),
                f64::from(frames) / since.elapsed().as_secs_f64()
            );
        }
    }
    if let Some(path) = &asked.shot {
        std::fs::write(path, render3::png(&picture, window_screen))?;
        println!(
            "{path}: {} frames shown, engine step {}, {}x{}",
            shown, session.steps, window_screen.wide, window_screen.high
        );
    }
    Ok(())
}

/// Draws no gas: every kind the table draws as gas is drawn as nothing.
fn no_gas(session: &mut Session<'_>) -> Result<(), Failed> {
    let gases = session.example.table.gases();
    for kind in 0..32 {
        if gases & (1 << kind) != 0 {
            session.example.table = session
                .example
                .table
                .with(kind, grit::render3::Material::EMPTY);
        }
    }
    session.held.write_at(
        grit::layout::PIXELS as usize,
        session.build.layout().table(),
        &session.example.table.words(),
    )?;
    Ok(())
}

/// Runs the scene's engine and its own cycle for `steps` steps with no
/// picture, and prints its fire and smoke now and then.
fn traced(host: &Host, asked: &Asked, steps: u32) -> Result<(), Failed> {
    use grit::rule3::nature::{LEAVES, WOOD, kind_of};
    let fuel = |cells: &[u32]| {
        cells
            .iter()
            .filter(|&&word| [WOOD, LEAVES].contains(&kind_of(word)))
            .count()
    };
    let quality = QUALITIES[0];
    let mut session = Session::open(host, setup(asked, quality))?;
    let before = fuel(&session.state()?);
    let clock = Instant::now();
    let mut last = None;
    for at in 0..steps {
        session.advance(1)?;
        let phase = session.life.as_ref().map(|life| (life.phase, life.round));
        if at % 64 == 0 || phase != last {
            let (fire, smoke) = session.counts()?;
            println!(
                "step {} {phase:?} fire {fire} smoke {smoke} at {:.1} s",
                session.steps,
                clock.elapsed().as_secs_f64()
            );
        }
        if phase.is_some_and(|(phase, _)| phase == life::Phase::After) && last != phase {
            let after = fuel(&session.state()?);
            println!(
                "fuel {before} before the fire, {after} after it: {} burned",
                before.saturating_sub(after)
            );
        }
        last = phase;
    }
    Ok(())
}

fn use_tool(
    session: &mut Session<'_>,
    (x, y): (f32, f32),
    quality: Quality,
    tool: usize,
) -> Result<(), Failed> {
    let scale = quality.scale.max(1) as f32;
    let (x, y) = (x / scale, y / scale);
    let screen = session.build.screen;
    if x < 0.0 || y < 0.0 || x as u32 >= screen.wide || y as u32 >= screen.high {
        return Ok(());
    }
    let Some(tool) = session.example.tools.get(tool).copied() else {
        return Ok(());
    };
    if let Some(seen) = session.under((x as u32, y as u32))? {
        session.stroke(seen, tool)?;
    }
    Ok(())
}

fn milliseconds(times: &mut [Duration]) -> (f64, f64) {
    times.sort_unstable();
    let least = times.first().copied().unwrap_or_default();
    let middle = times.get(times.len() / 2).copied().unwrap_or_default();
    (least.as_secs_f64() * 1000.0, middle.as_secs_f64() * 1000.0)
}

fn measured(host: &Host, asked: &Asked) -> Result<(), Failed> {
    let quality = chosen(host, asked)?;
    let mut session = Session::open(host, setup(asked, quality))?;
    if !asked.gas {
        no_gas(&mut session)?;
    }
    session.advance(asked.skip)?;
    let grid = session.grid();
    let orbit = asked.camera.map_or_else(
        || Orbit::start(asked.which, grid),
        |at| framed(asked.which, at, grid),
    );
    let camera = orbit_camera(&orbit, grid);
    let mut totals = Vec::with_capacity(TIMED);
    let mut parts: Vec<Vec<Duration>> = (0..2 + stage::COUNT)
        .map(|_| Vec::with_capacity(TIMED))
        .collect();
    let mut listed = Vec::with_capacity(TIMED);
    for at in 0..WARM + TIMED {
        let first = at == 0;
        session.aim(&camera, false, first)?;
        let used = steps(&session, quality, first);
        let _ = session.frame(true, used, false)?;
        if at < WARM {
            continue;
        }
        let times = session.times(true, used)?;
        totals.push(
            session
                .held
                .on_device()
                .ok_or("the device keeps no timestamps")?,
        );
        for (slot, time) in parts.iter_mut().zip(times) {
            slot.push(time);
        }
        listed.push(session.listed()?.len());
    }
    let (frame_min, frame_mid) = milliseconds(&mut totals);
    listed.sort_unstable();
    let mut line = format!(
        "{} | {} {}^3 | {} MiB on the device | {}x{} | quality {} suns {} | frames {WARM}..{} | listed octs median {} | frame {frame_min:.3}/{frame_mid:.3} ms",
        host.name(),
        session.example.name,
        grid.wide,
        session.bytes >> 20,
        session.build.screen.wide,
        session.build.screen.high,
        quality.name,
        session.build.suns,
        WARM + TIMED,
        listed.get(listed.len() / 2).copied().unwrap_or(0),
    );
    let names = ["engine", "bricks"]
        .into_iter()
        .chain(stage::NAMES)
        .collect::<Vec<_>>();
    for (name, times) in names.iter().zip(parts.iter_mut()) {
        let (least, middle) = milliseconds(times);
        line.push_str(&format!(" | {name} {least:.3}/{middle:.3}"));
    }
    line.push_str(&format!(
        " | {:.1} fps at the median frame",
        1000.0 / frame_mid.max(1e-6)
    ));
    println!("{line}");
    Ok(())
}
