mod camera;
mod font;
mod life;
mod open;
mod scenes;
mod session;
mod sparse;
mod ui;
mod valley;

use std::time::{Duration, Instant};

use grit::Host;
use grit::render3::present::{Presenter, windowed};
use grit::render3::{self, Screen, Steps, stage};

use camera::{Mode, Rig, View};
use font::Canvas;
use scenes::Which;
use session::{Failed, Session, Setup};

/// The frames of a timed run: warm-up, then timed.
const WARM: usize = 20;
const TIMED: usize = 60;

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
    readback: bool,
    timed: usize,
    shape: Option<[u32; 3]>,
    engine: bool,
    sparse: bool,
    open: bool,
    slots: Option<u32>,
    at: Option<[u32; 2]>,
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
        readback: false,
        timed: TIMED,
        shape: None,
        engine: false,
        sparse: false,
        open: false,
        slots: None,
        at: None,
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
            "--size" => {
                let text = value()?;
                let sides: Vec<&str> = text.split('x').collect();
                if let [wide, deep, tall] = sides.as_slice() {
                    let shape = [wide.parse()?, deep.parse()?, tall.parse()?];
                    held.side = Some(shape[0]);
                    held.shape = Some(shape);
                } else {
                    held.side = Some(text.parse()?);
                }
            }
            "--engine" => held.engine = true,
            "--sparse" => held.sparse = true,
            "--open" => {
                held.open = true;
                held.sparse = true;
                held.which = Which::Open;
            }
            "--at" => {
                let text = value()?;
                let parts: Vec<&str> = text.split(',').collect();
                let [x, z] = parts.as_slice() else {
                    return Err(format!("--at takes X,Z, not {text}").into());
                };
                held.at = Some([x.parse()?, z.parse()?]);
            }
            "--slots" => held.slots = Some(value()?.parse()?),
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
            "--readback" => held.readback = true,
            "--timed" => held.timed = value()?.parse()?,
            "--trace" => held.trace = Some(value()?.parse()?),
            "--generate" => held.generate = Some(value()?.parse()?),
            "--clean" => held.clean = true,
            "--active" => held.active = true,
            "--help-panel" => held.help = true,
            "--measure" => held.measure = true,
            other => {
                return Err(format!(
                    "unknown argument {other}; grit3d takes --scene valley|sand|fire|code|open, --size, --width, --height, --frames N, --shot FILE.png, --camera N, --suns 1-4, --quality full|half|third, --dusk 0-1, --smoothing on|off, --gas on|off, --tool N, --clean, --active, --help-panel, --measure, --skip STEPS, --trace STEPS, --cycle, --readback, --timed N, --engine, --sparse, --open, --at X,Z, --slots N and --generate SIDE; --size takes SIDE or WIDExDEEPxTALL"
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
    let opened = if asked.readback {
        Host::open()?
    } else {
        Host::open_presenting()?
    };
    let Some(host) = opened else {
        println!("no Vulkan device");
        return Ok(());
    };
    if asked.engine {
        return engined(&host, &asked);
    }
    if asked.which != Which::Open
        && asked
            .shape
            .is_some_and(|[wide, deep, tall]| wide != deep || deep != tall)
    {
        return Err(
            "a world that is not a cube runs only with --engine or as the open world".into(),
        );
    }
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
        let camera = home(asked, asked.which, session.grid()).camera();
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
        sparse: asked.sparse,
        at: asked.at,
        world: asked.shape.filter(|_| asked.which == Which::Open),
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

/// Where the camera of `which` starts, and where R and a switch to it take
/// it back: the framing `--camera` asks for, or the scene's first.
fn home(asked: &Asked, which: Which, grid: grit::sand::Grid) -> View {
    View::framed(which, asked.camera.unwrap_or(0), grid).held(Mode::Orbit, grid)
}

/// `picture`, drawn on `from`, stretched over `to` and blended between the
/// four pixels around each, as the device's window pass stretches it.
fn upscaled(picture: &[u32], from: Screen, to: Screen) -> Vec<u32> {
    if from == to {
        return picture.to_vec();
    }
    windowed(picture, from, to, &[])
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
}

/// A presenter on `window`, when the host's device can present to it.
#[cfg(windows)]
fn presenter_for<'h>(
    host: &'h Host,
    window: &minifb::Window,
    size: Screen,
) -> Option<Presenter<'h>> {
    if !host.presents() {
        return None;
    }
    let hwnd = window.get_window_handle() as isize;
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    match unsafe { Presenter::new(host, hwnd, module, size) } {
        Ok(made) => Some(made),
        Err(why) => {
            println!("the window is not presented from the device: {why}");
            None
        }
    }
}

#[cfg(not(windows))]
fn presenter_for<'h>(_: &'h Host, _: &minifb::Window, _: Screen) -> Option<Presenter<'h>> {
    None
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
    rig: Rig,
    tool: usize,
    paused: bool,
    active: bool,
    help: bool,
    pressed_at: Option<(f32, f32)>,
    dragging: bool,
    last_mouse: Option<(f32, f32)>,
    middle_at: Option<(f32, f32)>,
}

/// The key of a keyboard input; none for the mouse's.
fn key_of(input: ui::Input) -> Option<minifb::Key> {
    use minifb::Key;
    use ui::Input;
    match input {
        Input::Digit(digit) => [
            Key::Key0,
            Key::Key1,
            Key::Key2,
            Key::Key3,
            Key::Key4,
            Key::Key5,
            Key::Key6,
            Key::Key7,
            Key::Key8,
            Key::Key9,
        ]
        .get(digit as usize)
        .copied(),
        Input::W => Some(Key::W),
        Input::A => Some(Key::A),
        Input::S => Some(Key::S),
        Input::D => Some(Key::D),
        Input::Q => Some(Key::Q),
        Input::E => Some(Key::E),
        Input::F => Some(Key::F),
        Input::O => Some(Key::O),
        Input::R => Some(Key::R),
        Input::N => Some(Key::N),
        Input::H => Some(Key::H),
        Input::Space => Some(Key::Space),
        Input::Shift => Some(Key::LeftShift),
        Input::Escape => Some(Key::Escape),
        Input::Drag
        | Input::ShiftDrag
        | Input::MiddleDrag
        | Input::RightDrag
        | Input::Wheel
        | Input::Click
        | Input::ClickBar => None,
    }
}

/// Whether the key of `input` is down; Shift either side.
fn down(window: &minifb::Window, input: ui::Input) -> bool {
    match input {
        ui::Input::Shift => {
            window.is_key_down(minifb::Key::LeftShift)
                || window.is_key_down(minifb::Key::RightShift)
        }
        other => key_of(other).is_some_and(|key| window.is_key_down(key)),
    }
}

/// How much one notch of the wheel turns minifb's scroll value.
#[cfg(windows)]
const NOTCH: f32 = 12.0;
#[cfg(not(windows))]
const NOTCH: f32 = 1.0;

/// The point of the cell a window pixel sees in the last frame, at the
/// middle of the cell, when it sees one.
fn picked(
    session: &mut Session<'_>,
    (x, y): (f32, f32),
    quality: Quality,
) -> Result<Option<[f32; 3]>, Failed> {
    let scale = quality.scale.max(1) as f32;
    let (x, y) = (x / scale, y / scale);
    let screen = session.build.screen;
    if x < 0.0 || y < 0.0 || x as u32 >= screen.wide || y as u32 >= screen.high {
        return Ok(None);
    }
    let grid = session.grid();
    Ok(session
        .under((x as u32, y as u32))?
        .map(|(cell, _)| grit::geometry::Solid::point(grid, cell).map(|at| at as f32 + 0.5)))
}

#[allow(clippy::too_many_lines)]
fn shown(host: &Host, asked: &Asked) -> Result<(), Failed> {
    let quality = chosen(host, asked)?;
    let window_screen = asked.window;
    let on_device = (host.presents() && !asked.readback).then_some(window_screen);
    let mut session = Session::open_in(host, setup(asked, quality), on_device)?;
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
    let mut play = Play {
        rig: Rig::new(home(asked, asked.which, session.grid()), session.grid()),
        tool: asked.tool,
        paused: false,
        active: asked.active,
        help: asked.help,
        pressed_at: None,
        dragging: false,
        last_mouse: None,
        middle_at: None,
    };
    let mut window = minifb::Window::new(
        "grit3d",
        window_screen.wide as usize,
        window_screen.high as usize,
        minifb::WindowOptions::default(),
    )?;
    window.set_target_fps(0);
    let mut presenter = if on_device.is_some() {
        presenter_for(host, &window, window_screen)
    } else {
        None
    };
    println!(
        "the window is shown {}",
        presenter.as_ref().map_or_else(
            || "by reading each picture back".to_owned(),
            |held| format!("from the device, presenting in {} mode", held.mode())
        )
    );
    let mut overlay = ui::Overlay::new((window_screen.wide, window_screen.high));
    let mut drawn_ui: Option<(String, usize, bool)> = None;
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
    while window.is_open() {
        let now = Instant::now();
        let elapsed = now.duration_since(last_frame);
        let spent = elapsed.as_secs_f32().min(0.1);
        last_frame = now;
        let mut step = false;
        let mut switch_to = None;
        let mut restart = false;
        let mut quit = false;
        let mut push = [0.0_f32; 3];
        let mut fast = false;
        {
            use minifb::{KeyRepeat, MouseButton, MouseMode};
            use ui::{Action, Input};
            let mut fly = false;
            for (input, action) in ui::BINDINGS {
                let Some(key) = key_of(input) else {
                    continue;
                };
                let now = if action.held() {
                    down(&window, input)
                } else {
                    window.is_key_pressed(key, KeyRepeat::No)
                };
                if !now {
                    continue;
                }
                match action {
                    Action::Scene(at) => switch_to = Which::ALL.get(at).copied(),
                    Action::Tool(at) => {
                        if at < session.example.tools.len() {
                            play.tool = at;
                        }
                    }
                    Action::Forward
                    | Action::Back
                    | Action::Right
                    | Action::Left
                    | Action::Up
                    | Action::Down => {
                        for (slot, by) in push.iter_mut().zip(action.push()) {
                            *slot += by;
                        }
                    }
                    Action::Faster => fast = true,
                    Action::Fly => fly = true,
                    Action::Overlay => play.active = !play.active,
                    Action::Restart => restart = true,
                    Action::Pause => play.paused = !play.paused,
                    Action::Step => step = true,
                    Action::Help => play.help = !play.help,
                    Action::Quit => quit = true,
                    Action::Turn
                    | Action::Pan
                    | Action::Zoom
                    | Action::UseTool
                    | Action::KeepUsingTool
                    | Action::PickTool => {}
                }
            }
            let window_size = (window_screen.wide, window_screen.high);
            let mouse = window.get_mouse_pos(MouseMode::Discard);
            if fly {
                let middle = (
                    window_screen.wide as f32 / 2.0,
                    window_screen.high as f32 / 2.0,
                );
                let ahead = picked(&mut session, middle, quality)?;
                play.rig.switch(ahead);
            }
            if let Some((_, wheel)) = window.get_scroll_wheel()
                && wheel != 0.0
                && ui::meaning(Input::Wheel) == Some(Action::Zoom)
            {
                let anchor = match mouse {
                    Some(at) => picked(&mut session, at, quality)?
                        .map(|point| play.rig.shown.anchor(window_screen, at, point)),
                    None => None,
                };
                play.rig.zoom(wheel / NOTCH, anchor);
            }
            let shift = down(&window, Input::Shift);
            let left = window.get_mouse_down(MouseButton::Left);
            let middle = window.get_mouse_down(MouseButton::Middle);
            let right = window.get_mouse_down(MouseButton::Right);
            let tools = session.example.tools.clone();
            let dragged = |play: &mut Play, input: Input, by: (f32, f32)| match ui::meaning(input) {
                Some(Action::Turn) => play.rig.turn(by),
                Some(Action::Pan) => play.rig.pan(window_screen, by),
                _ => {}
            };
            match (left, play.pressed_at, mouse) {
                (true, None, Some(at)) => {
                    play.pressed_at = Some(at);
                    play.dragging = false;
                    play.last_mouse = Some(at);
                }
                (true, Some(start), Some(at)) => {
                    if (at.0 - start.0).abs() + (at.1 - start.1).abs() > 5.0 {
                        play.dragging = true;
                    }
                    if play.dragging
                        && let Some(before) = play.last_mouse
                    {
                        let input = if shift { Input::ShiftDrag } else { Input::Drag };
                        dragged(&mut play, input, (at.0 - before.0, at.1 - before.1));
                    }
                    play.last_mouse = Some(at);
                }
                (false, Some(start), _) => {
                    if !play.dragging {
                        if let Some(tool) = ui::pressed(&tools, window_size, start) {
                            if ui::meaning(Input::ClickBar) == Some(Action::PickTool) {
                                play.tool = tool;
                            }
                        } else if ui::meaning(Input::Click) == Some(Action::UseTool) {
                            use_tool(&mut session, start, quality, play.tool)?;
                        }
                    }
                    play.pressed_at = None;
                    play.dragging = false;
                }
                _ => {}
            }
            match (middle, mouse) {
                (true, Some(at)) => {
                    if let Some(before) = play.middle_at {
                        dragged(
                            &mut play,
                            Input::MiddleDrag,
                            (at.0 - before.0, at.1 - before.1),
                        );
                    }
                    play.middle_at = Some(at);
                }
                _ => play.middle_at = None,
            }
            if right
                && let Some(at) = mouse
                && ui::pressed(&tools, window_size, at).is_none()
                && ui::meaning(Input::RightDrag) == Some(Action::KeepUsingTool)
            {
                use_tool(&mut session, at, quality, play.tool)?;
            }
        }
        if quit {
            break;
        }
        if let Some(which) = switch_to.or(restart.then_some(session.setup.which)) {
            let mut wanted = setup(asked, quality);
            wanted.which = which;
            if let Some(held) = presenter.as_mut() {
                held.settle()?;
            }
            drop(session);
            session = Session::open_in(host, wanted, on_device)?;
            overlay = ui::Overlay::new((window_screen.wide, window_screen.high));
            drawn_ui = None;
            if let Some(dusk) = asked.dusk {
                session.dusk = dusk;
            }
            play.tool = 0;
            play.rig
                .reset(home(asked, which, session.grid()), session.grid());
            fresh = true;
        }
        play.rig.tick(push, fast, spent);
        if let Some(by) = session.follow(play.rig.look())? {
            play.rig.moved(by);
        }
        let camera = play.rig.camera();
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
        let last = asked.frames.is_some_and(|last| shown + 1 >= last);
        let line = corner_line(&session, rate, &play, active_count);
        if let Some(held) = presenter.as_mut() {
            if !asked.clean {
                let key = (line.clone(), play.tool, play.help);
                if drawn_ui.as_ref() != Some(&key) {
                    let tools = session.example.tools.clone();
                    let size = (window_screen.wide, window_screen.high);
                    let (tool, help) = (play.tool, play.help);
                    let rows = overlay.redrawn(ui::bands(&tools, size, help), |canvas| {
                        ui::draw_all(canvas, &line, &tools, tool, help);
                    });
                    session.overlay(&rows)?;
                    drawn_ui = Some(key);
                }
            }
            let pending = session.submit(moving, light_steps)?;
            if let Some(placed) = session.windowed {
                held.present(&session.held, placed)?;
            }
            session.land(pending)?;
            window.update();
            if last && asked.shot.is_some() {
                let colours = session
                    .held
                    .read(grit::layout::PIXELS as usize, session.build.screen.pixels())?;
                picture = upscaled(&colours, session.build.screen, window_screen);
            }
        } else {
            let (_, got) = session.frame(moving, light_steps, true)?;
            let mut frame = upscaled(&got, session.build.screen, window_screen);
            if !asked.clean {
                let mut canvas = Canvas {
                    pixels: &mut frame,
                    wide: window_screen.wide,
                    high: window_screen.high,
                };
                ui::draw_all(
                    &mut canvas,
                    &line,
                    &session.example.tools,
                    play.tool,
                    play.help,
                );
            }
            window.update_with_buffer(
                &frame,
                window_screen.wide as usize,
                window_screen.high as usize,
            )?;
            picture = frame;
        }
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
        counted += 1;
        if clock.elapsed().as_secs_f64() >= 0.5 {
            rate = f64::from(counted) / clock.elapsed().as_secs_f64();
            clock = Instant::now();
            counted = 0;
            if play.active {
                active_count = session.active()?;
            }
        }
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

/// The line the window shows at its top left.
fn corner_line(session: &Session<'_>, rate: f64, play: &Play, active_count: u32) -> String {
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
    if play.rig.mode == Mode::Fly {
        line.push_str("FLYING  \u{b7}  ");
    }
    line.push_str(ui::HINT);
    line
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
    let clock = Instant::now();
    let mut session = Session::open(host, setup(asked, quality))?;
    let opened = clock.elapsed();
    if !asked.gas {
        no_gas(&mut session)?;
    }
    session.advance(asked.skip)?;
    let grid = session.grid();
    let camera = home(asked, asked.which, grid).camera();
    let timed = asked.timed.max(1);
    let mut totals = Vec::with_capacity(timed);
    let mut parts: Vec<Vec<Duration>> = (0..2 + stage::COUNT)
        .map(|_| Vec::with_capacity(timed))
        .collect();
    let mut listed = Vec::with_capacity(timed);
    for at in 0..WARM + timed {
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
        listed.push(session.listed_count()?);
    }
    let (frame_min, frame_mid) = milliseconds(&mut totals);
    let frame_tail = totals
        .get((totals.len() * 99).div_ceil(100).saturating_sub(1))
        .copied()
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0;
    listed.sort_unstable();
    let mut line = format!(
        "{} | {} {}x{}x{} | opened in {:.1} s | {} MiB on the device | {}x{} | quality {} suns {} | frames {WARM}..{} | listed octs median {} | frame {frame_min:.3}/{frame_mid:.3}/{frame_tail:.3} ms",
        host.name(),
        session.example.name,
        grid.wide,
        grid.deep,
        grid.tall,
        opened.as_secs_f64(),
        session.bytes >> 20,
        session.build.screen.wide,
        session.build.screen.high,
        quality.name,
        session.build.suns,
        WARM + timed,
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

/// Times the scene's engine alone, with no picture, on the world `--size`
/// names: how long the world takes to make, the bytes of the engine's
/// bindings, and each frame on the device.
fn engined(host: &Host, asked: &Asked) -> Result<(), Failed> {
    use grit::rule3::code::Code;
    use grit::rule3::heat::Heat;
    use grit::rule3::nature::Nature;
    use grit::rule3::sand::Sand;

    match asked.which {
        Which::Valley | Which::Open => engined_by(host, asked, &Nature),
        Which::Sand => engined_by(host, asked, &Sand),
        Which::Fire => engined_by(host, asked, &Heat),
        Which::Code => engined_by(host, asked, &Code),
    }
}

fn engined_by<R: grit::rule3::Rule>(host: &Host, asked: &Asked, rule: &R) -> Result<(), Failed> {
    use grit::Groups;
    use grit::compact::Plan;
    use grit::layout::{Dimension, GRAND, PARAMS};
    use grit::{live, sweep};

    let side = asked.side.unwrap_or(QUALITIES[0].side);
    let [wide, deep, tall] = if asked.open {
        asked.shape.unwrap_or(sparse::OPEN)
    } else {
        asked.shape.unwrap_or([side; 3])
    };
    let grid = grit::sand::Grid::solid(wide, deep, tall)?;
    let example = asked.which.example();
    if asked.sparse {
        let clock = Instant::now();
        let world = if asked.open {
            sparse::World::Open(open::Land::new(grid, open::SEED))
        } else {
            sparse::World::Dense((example.scene)(grid))
        };
        let generated = clock.elapsed();
        let line = sparse::engined(
            host,
            rule,
            grid,
            world,
            &sparse::Asked {
                slots: asked.slots,
                skip: asked.skip,
                warm: WARM,
                timed: asked.timed.max(1),
                params: example.params.clone(),
                name: if asked.open { "open" } else { example.name },
            },
        )?;
        println!("{line} | generated in {:.1} s", generated.as_secs_f64());
        return Ok(());
    }
    let width = host.width();
    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )?;
    let windows = plan.windows();
    let sizes = windows.laid(&live::sizes3(rule, &plan));
    host.limits().admits_on(&sizes, None, host.heap())?;
    let (passes, stages) = live::wired3_by(rule, width, grid, &plan)?;
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules)?;

    let clock = Instant::now();
    let world = (example.scene)(grid);
    let made = clock.elapsed();
    let zeros = vec![0_u32; 1 << 24];
    for (binding, &words) in sizes.iter().enumerate() {
        let mut at = 0;
        while at < words {
            let run = zeros.len().min(words - at);
            held.write_at(binding, at, zeros.get(..run).unwrap_or(&[]))?;
            at += run;
        }
    }
    live::write_state(&mut held, windows, 0, &world)?;
    drop(world);
    if !example.params.is_empty() {
        held.write_at(PARAMS as usize, grit::rule3::FIRST_PARAM, &example.params)?;
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    live::seed(&mut held, grid, &plan)?;

    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    for _ in 0..asked.skip {
        held.staged(&groups)?;
    }
    let timed = asked.timed.max(1);
    let mut times = Vec::with_capacity(timed);
    let mut listed = Vec::with_capacity(timed);
    for at in 0..WARM + timed {
        held.staged(&groups)?;
        if at < WARM {
            continue;
        }
        times.push(held.on_device().ok_or("the device keeps no timestamps")?);
        listed.push(held.read(GRAND as usize, 1)?.first().copied().unwrap_or(0));
    }
    let (least, middle) = milliseconds(&mut times);
    let tail = times
        .get((times.len() * 99).div_ceil(100).saturating_sub(1))
        .copied()
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0;
    listed.sort_unstable();
    let bytes: usize = sizes.iter().sum::<usize>() * 4;
    println!(
        "{} | {} engine alone {}x{}x{} | made in {:.1} s | {} MiB on the device over {} bindings, {} windows of STATE | steps {}..{} | listed octs median {} | frame {least:.3}/{middle:.3}/{tail:.3} ms",
        host.name(),
        example.name,
        grid.wide,
        grid.deep,
        grid.tall,
        made.as_secs_f64(),
        bytes >> 20,
        sizes.len(),
        windows.count(),
        asked.skip + WARM as u32,
        asked.skip + (WARM + timed) as u32,
        listed.get(listed.len() / 2).copied().unwrap_or(0),
    );
    Ok(())
}
