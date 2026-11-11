use super::font::{Canvas, HIGH, width};
use super::scenes::Tool;

/// The number key of the first tool; keys 0 to 3 switch scenes.
pub const FIRST_TOOL_KEY: u32 = 4;

/// A box on the screen: its top left, width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: i64,
    pub y: i64,
    pub wide: u32,
    pub high: u32,
}

impl Area {
    pub fn holds(&self, (x, y): (f32, f32)) -> bool {
        let (x, y) = (x as i64, y as i64);
        x >= self.x
            && y >= self.y
            && x < self.x + i64::from(self.wide)
            && y < self.y + i64::from(self.high)
    }
}

/// Glyph pixels a screen pixel for a window `high` pixels tall.
pub fn scale_of(high: u32) -> u32 {
    (high / 400).max(1)
}

/// The label of tool `at` on the bar: its key and its name.
pub fn label(at: usize, tool: &Tool) -> String {
    format!("{} {}", at as u32 + FIRST_TOOL_KEY, tool.name)
}

/// Where each tool's button lies on a window `wide` by `high`: a row along
/// the bottom, in the middle.
pub fn bar(tools: &[Tool], (wide, high): (u32, u32)) -> Vec<Area> {
    let scale = scale_of(high);
    let pad = 6 * scale;
    let gap = 4 * scale;
    let tall = HIGH * scale + 2 * pad;
    let sizes: Vec<u32> = tools
        .iter()
        .enumerate()
        .map(|(at, tool)| width(&label(at, tool), scale) + 2 * pad)
        .collect();
    let total: u32 = sizes.iter().sum::<u32>() + gap * sizes.len().saturating_sub(1) as u32;
    let mut x = (i64::from(wide) - i64::from(total)) / 2;
    let y = i64::from(high) - i64::from(tall) - i64::from(10 * scale);
    sizes
        .into_iter()
        .map(|size| {
            let area = Area {
                x,
                y,
                wide: size,
                high: tall,
            };
            x += i64::from(size + gap);
            area
        })
        .collect()
}

/// The tool whose button holds `point`, if any.
pub fn pressed(tools: &[Tool], window: (u32, u32), point: (f32, f32)) -> Option<usize> {
    bar(tools, window).iter().position(|area| area.holds(point))
}

const INK: u32 = 0x00F4_EEE6;
const SHADE: u32 = 0x0014_1210;
const CHOSEN: u32 = 0x00E8_A040;

/// Draws the tool bar with tool `chosen` lit.
pub fn draw_bar(canvas: &mut Canvas<'_>, tools: &[Tool], chosen: usize) {
    let window = (canvas.wide, canvas.high);
    let scale = scale_of(canvas.high);
    let pad = 6 * scale;
    for (at, (tool, area)) in tools.iter().zip(bar(tools, window)).enumerate() {
        let lit = at == chosen;
        canvas.fill(
            (area.x, area.y),
            (area.wide, area.high),
            SHADE,
            if lit { 0.72 } else { 0.5 },
        );
        canvas.frame(
            (area.x, area.y),
            (area.wide, area.high),
            if lit { CHOSEN } else { INK },
            if lit { 0.95 } else { 0.25 },
        );
        canvas.text(
            (area.x + i64::from(pad), area.y + i64::from(pad)),
            &label(at, tool),
            if lit { CHOSEN } else { INK },
            scale,
        );
    }
}

/// Draws one small line at the top left.
pub fn draw_corner(canvas: &mut Canvas<'_>, line: &str) {
    let scale = scale_of(canvas.high);
    let margin = i64::from(10 * scale);
    canvas.text((margin, margin), line, INK, scale);
}

/// What the user can press, click, drag or turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Digit(u32),
    W,
    A,
    S,
    D,
    Q,
    E,
    F,
    O,
    R,
    N,
    H,
    Space,
    Shift,
    Escape,
    Drag,
    ShiftDrag,
    MiddleDrag,
    RightDrag,
    Wheel,
    Click,
    ClickBar,
}

impl Input {
    /// The input the help panel names `name`.
    #[allow(dead_code)]
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "W" => Self::W,
            "A" => Self::A,
            "S" => Self::S,
            "D" => Self::D,
            "Q" => Self::Q,
            "E" => Self::E,
            "F" => Self::F,
            "O" => Self::O,
            "R" => Self::R,
            "N" => Self::N,
            "H" => Self::H,
            "SPACE" => Self::Space,
            "SHIFT" => Self::Shift,
            "ESC" => Self::Escape,
            "DRAG" => Self::Drag,
            "SHIFT + DRAG" => Self::ShiftDrag,
            "MIDDLE DRAG" => Self::MiddleDrag,
            "RIGHT DRAG" => Self::RightDrag,
            "WHEEL" => Self::Wheel,
            "CLICK" => Self::Click,
            "CLICK BAR" => Self::ClickBar,
            _ => {
                let digit = name.parse::<u32>().ok().filter(|&digit| digit <= 9)?;
                Self::Digit(digit)
            }
        })
    }
}

/// What an input does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Scene(usize),
    Tool(usize),
    Forward,
    Left,
    Back,
    Right,
    Down,
    Up,
    Faster,
    Fly,
    Overlay,
    Restart,
    Pause,
    Step,
    Help,
    Quit,
    Turn,
    Pan,
    Zoom,
    UseTool,
    KeepUsingTool,
    PickTool,
}

impl Action {
    /// Which way the action moves the camera while its key is held: right,
    /// up and forward, as [`super::camera::View::walked`] takes them.
    pub fn push(self) -> [f32; 3] {
        match self {
            Self::Forward => [0.0, 0.0, 1.0],
            Self::Back => [0.0, 0.0, -1.0],
            Self::Right => [1.0, 0.0, 0.0],
            Self::Left => [-1.0, 0.0, 0.0],
            Self::Up => [0.0, 1.0, 0.0],
            Self::Down => [0.0, -1.0, 0.0],
            _ => [0.0; 3],
        }
    }

    /// Whether the action lasts as long as its key is held, rather than
    /// happening once a press.
    pub fn held(self) -> bool {
        matches!(
            self,
            Self::Forward
                | Self::Left
                | Self::Back
                | Self::Right
                | Self::Down
                | Self::Up
                | Self::Faster
        )
    }
}

/// Every input and what it does: the demo reads its inputs from this
/// list, and the help panel names each of them.
pub const BINDINGS: [(Input, Action); 30] = [
    (Input::Digit(0), Action::Scene(0)),
    (Input::Digit(1), Action::Scene(1)),
    (Input::Digit(2), Action::Scene(2)),
    (Input::Digit(3), Action::Scene(3)),
    (Input::Digit(4), Action::Tool(0)),
    (Input::Digit(5), Action::Tool(1)),
    (Input::Digit(6), Action::Tool(2)),
    (Input::Digit(7), Action::Tool(3)),
    (Input::Digit(8), Action::Tool(4)),
    (Input::W, Action::Forward),
    (Input::A, Action::Left),
    (Input::S, Action::Back),
    (Input::D, Action::Right),
    (Input::Q, Action::Down),
    (Input::E, Action::Up),
    (Input::Shift, Action::Faster),
    (Input::F, Action::Fly),
    (Input::O, Action::Overlay),
    (Input::R, Action::Restart),
    (Input::Space, Action::Pause),
    (Input::N, Action::Step),
    (Input::H, Action::Help),
    (Input::Escape, Action::Quit),
    (Input::Drag, Action::Turn),
    (Input::ShiftDrag, Action::Pan),
    (Input::MiddleDrag, Action::Pan),
    (Input::Wheel, Action::Zoom),
    (Input::Click, Action::UseTool),
    (Input::RightDrag, Action::KeepUsingTool),
    (Input::ClickBar, Action::PickTool),
];

/// What `input` does, if anything.
pub fn meaning(input: Input) -> Option<Action> {
    BINDINGS
        .iter()
        .find(|(bound, _)| *bound == input)
        .map(|&(_, action)| action)
}

/// Every control, a line each: the inputs, then two spaces or more, then
/// what they do.
pub const HELP: [&str; 19] = [
    "CONTROLS",
    "",
    "DRAG                        TURN AROUND THE TARGET, OR LOOK WHEN FLYING",
    "SHIFT + DRAG / MIDDLE DRAG  MOVE THE TARGET ACROSS THE VIEW",
    "WHEEL                       ZOOM TO THE CURSOR",
    "W A S D                     MOVE OVER THE GROUND, OR FLY AHEAD",
    "Q / E                       DOWN / UP",
    "SHIFT                       FASTER",
    "F                           FLY, OR ORBIT AGAIN",
    "",
    "CLICK                       USE THE TOOL ONCE",
    "RIGHT DRAG                  KEEP USING THE TOOL",
    "4 - 8 / CLICK BAR           PICK A TOOL",
    "0 1 2 3                     VALLEY, SAND, FIRE, CODE",
    "R                           RESTART THE SCENE AND THE CAMERA",
    "SPACE / N                   PAUSE / ONE STEP",
    "O                           SHOW WHAT THE ENGINE STEPS",
    "H                           THIS HELP",
    "ESC                         QUIT",
];

/// The controls the corner line names.
pub const HINT: &str = "DRAG TURN, WHEEL ZOOM, WASD MOVE, F FLY, H HELP";

/// Draws the help panel in the middle of the window.
pub fn draw_help(canvas: &mut Canvas<'_>) {
    let scale = scale_of(canvas.high);
    let line = (HIGH + 5) * scale;
    let pad = 14 * scale;
    let wide = HELP
        .iter()
        .map(|text| width(text, scale))
        .max()
        .unwrap_or(0)
        + 2 * pad;
    let high = line * HELP.len() as u32 + 2 * pad;
    let x = (i64::from(canvas.wide) - i64::from(wide)) / 2;
    let y = (i64::from(canvas.high) - i64::from(high)) / 2;
    canvas.fill((x, y), (wide, high), SHADE, 0.78);
    canvas.frame((x, y), (wide, high), INK, 0.3);
    for (at, text) in HELP.iter().enumerate() {
        canvas.text(
            (
                x + i64::from(pad),
                y + i64::from(pad) + i64::from(line) * at as i64,
            ),
            text,
            if at == 0 { CHOSEN } else { INK },
            scale,
        );
    }
}

/// Draws everything the window lays over the picture: the corner line, the
/// tool bar with tool `chosen` lit, and the help panel when `help`.
pub fn draw_all(canvas: &mut Canvas<'_>, line: &str, tools: &[Tool], chosen: usize, help: bool) {
    draw_corner(canvas, line);
    draw_bar(canvas, tools, chosen);
    if help {
        draw_help(canvas);
    }
}

/// The rows of a window `wide` by `high` that [`draw_all`] can draw on,
/// each a first row and the row past the last.
pub fn bands(tools: &[Tool], (wide, high): (u32, u32), help: bool) -> Vec<(u32, u32)> {
    let scale = scale_of(high);
    let mut out = vec![(0, (20 + HIGH + 2) * scale)];
    if let Some(area) = bar(tools, (wide, high)).first() {
        let from = area.y.max(0) as u32;
        out.push((from, (from + area.high + 2).min(high)));
    }
    if help {
        let line = (HIGH + 5) * scale;
        let tall = line * HELP.len() as u32 + 28 * scale;
        let from = high.saturating_sub(tall) / 2;
        out.push((from, (from + tall + 2).min(high)));
    }
    out.into_iter()
        .map(|(from, to)| (from.min(high), to.min(high)))
        .collect()
}

/// What the window lays over the picture on the device, kept as the words
/// [`grit::render3::present::covered`] reads: drawn twice, over black and
/// over white, so how much each pixel covers is what the two differ by.
pub struct Overlay {
    wide: u32,
    high: u32,
    black: Vec<u32>,
    white: Vec<u32>,
    sent: Vec<u32>,
    bands: Vec<(u32, u32)>,
}

impl Overlay {
    pub fn new((wide, high): (u32, u32)) -> Self {
        let pixels = wide as usize * high as usize;
        Self {
            wide,
            high,
            black: vec![0; pixels],
            white: vec![0x00FF_FFFF; pixels],
            sent: vec![0; pixels],
            bands: Vec::new(),
        }
    }

    /// Draws with `draw` over the rows `bands` gives, and gives the rows
    /// whose words changed since the last time, each its row and words.
    pub fn redrawn(
        &mut self,
        bands: Vec<(u32, u32)>,
        draw: impl Fn(&mut Canvas<'_>),
    ) -> Vec<(u32, Vec<u32>)> {
        let wide = self.wide as usize;
        let mut rows: Vec<u32> = self
            .bands
            .iter()
            .chain(&bands)
            .flat_map(|&(from, to)| from..to)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        for &row in &rows {
            let span = row as usize * wide..(row as usize + 1) * wide;
            if let Some(black) = self.black.get_mut(span.clone()) {
                black.fill(0);
            }
            if let Some(white) = self.white.get_mut(span) {
                white.fill(0x00FF_FFFF);
            }
        }
        for (pixels, _) in [(&mut self.black, 0), (&mut self.white, 1)] {
            let mut canvas = Canvas {
                pixels,
                wide: self.wide,
                high: self.high,
            };
            draw(&mut canvas);
        }
        self.bands = bands;
        let mut out = Vec::new();
        for row in rows {
            let span = row as usize * wide..(row as usize + 1) * wide;
            let (Some(black), Some(white)) =
                (self.black.get(span.clone()), self.white.get(span.clone()))
            else {
                continue;
            };
            let words: Vec<u32> = black
                .iter()
                .zip(white)
                .map(|(&dark, &light)| {
                    let keep = ((light >> 8) & 0xFF).saturating_sub((dark >> 8) & 0xFF);
                    ((255 - keep) << 24) | (dark & 0x00FF_FFFF)
                })
                .collect();
            if let Some(sent) = self.sent.get_mut(span)
                && *sent != words
            {
                sent.copy_from_slice(&words);
                out.push((row, words));
            }
        }
        out
    }
}
