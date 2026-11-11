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

/// Every control, a line each.
pub const HELP: [&str; 13] = [
    "CONTROLS",
    "",
    "DRAG               ORBIT THE CAMERA",
    "WHEEL              ZOOM",
    "CLICK              USE THE TOOL ONCE",
    "RIGHT DRAG         KEEP USING THE TOOL",
    "4 - 8 / CLICK BAR  PICK A TOOL",
    "0  1  2  3         VALLEY, SAND, FIRE, CODE",
    "R                  RESTART THE SCENE",
    "SPACE / N          PAUSE / ONE STEP",
    "A                  SHOW WHAT THE ENGINE STEPS",
    "H                  THIS HELP",
    "ESC                QUIT",
];

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
