use std::time::{Duration, Instant};

use grit::compact::Plan;
use grit::geometry::Solid;
use grit::layout::{BINDINGS, Dimension, GRAND, PARAMS, PIXELS, STATE};
use grit::rule3::code::{self, Code};
use grit::rule3::heat::{self, Heat};
use grit::rule3::sand::Sand;
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::{Grid, View};
use grit::view3::{
    self, Axis, Kinds, Look, PALETTE, SHOWN, cell_under, choosing, extent, palette_at, pictures,
};
use grit::{Groups, Host, Resident, live, sweep};

const SIDE: u32 = 128;

const VIEW: u32 = 512;

const REACH: u32 = 1;

const TIMED: usize = 31;

type Failed = Box<dyn std::error::Error>;

struct Example {
    rule: &'static str,
    params: Vec<u32>,
    palette: [u32; PALETTE],
    words: Vec<u32>,
    scene: fn(Grid) -> Vec<u32>,
}

#[derive(Debug, Clone)]
struct Asked {
    rule: String,
    side: u32,
    view: u32,
    axis: Axis,
    look: Look,
    layer: Option<u32>,
    frames: Option<u32>,
    shot: Option<String>,
    measure: bool,
}

fn asked() -> Result<Asked, Failed> {
    let mut held = Asked {
        rule: "sand".to_owned(),
        side: SIDE,
        view: VIEW,
        axis: Axis::Z,
        look: Look::Slice,
        layer: None,
        frames: None,
        shot: None,
        measure: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(one) = args.next() {
        let mut value = || -> Result<String, Failed> {
            args.next()
                .ok_or_else(|| format!("{one} needs a value").into())
        };
        match one.as_str() {
            "--rule" => held.rule = value()?,
            "--size" => held.side = value()?.parse()?,
            "--view" => held.view = value()?.parse()?,
            "--axis" => {
                held.axis = match value()?.as_str() {
                    "x" => Axis::X,
                    "y" => Axis::Y,
                    "z" => Axis::Z,
                    other => return Err(format!("--axis {other}: x, y or z").into()),
                }
            }
            "--depth" => held.look = Look::Depth,
            "--layer" => held.layer = Some(value()?.parse()?),
            "--frames" => held.frames = Some(value()?.parse()?),
            "--shot" => held.shot = Some(value()?),
            "--measure" => held.measure = true,
            other => {
                return Err(format!(
                    "unknown argument {other}; pour3 takes --rule sand|heat|code, --size, --view, --axis x|y|z, --depth, --layer, --frames, --shot FILE.png and --measure"
                )
                .into());
            }
        }
    }
    Ok(held)
}

fn main() -> Result<(), Failed> {
    let asked = asked()?;
    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    match asked.rule.as_str() {
        "sand" => run(&host, &Sand, &sanded(), &asked),
        "heat" => run(&host, &Heat, &heated(), &asked),
        "code" => run(&host, &Code, &coded(), &asked),
        other => Err(format!("--rule {other}: sand, heat or code").into()),
    }
}

fn index(grid: Grid, [x, y, z]: [u32; 3]) -> usize {
    Solid::cell(grid, [x, y, z]) as usize
}

fn put(cells: &mut [u32], grid: Grid, point: [u32; 3], word: u32) {
    if let Some(slot) = cells.get_mut(index(grid, point)) {
        *slot = word;
    }
}

fn boxed(grid: Grid, wall: u32, empty: u32) -> Vec<u32> {
    let mut cells = vec![empty; grid.cells()];
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let side = x == 0 || x + 1 == grid.wide || z == 0 || z + 1 == grid.tall;
                if y + 1 == grid.deep || side {
                    put(&mut cells, grid, [x, y, z], wall);
                }
            }
        }
    }
    cells
}

fn ball(cells: &mut [u32], grid: Grid, centre: [u32; 3], reach: u32, word: u32) {
    for z in 0..grid.tall {
        for y in 0..grid.deep {
            for x in 0..grid.wide {
                let apart = |a: u32, b: u32| a.abs_diff(b).pow(2);
                let far = apart(x, centre[0]) + apart(y, centre[1]) + apart(z, centre[2]);
                if far <= reach * reach {
                    put(cells, grid, [x, y, z], word);
                }
            }
        }
    }
}

fn sanded() -> Example {
    use grit::sand::{AIR, SAND, WALL, WATER};
    let mut palette = [0x0011_1622; PALETTE];
    for (kind, colour) in [
        (AIR, 0x0011_1622),
        (WATER, 0x0024_6FA8),
        (SAND, 0x00D9_B36A),
        (WALL, 0x0044_4A52),
    ] {
        if let Some(slot) = palette.get_mut(kind as usize) {
            *slot = colour;
        }
    }
    Example {
        rule: "sand",
        params: Vec::new(),
        palette,
        words: vec![AIR, SAND, WATER, WALL],
        scene: |grid| {
            let mut cells = boxed(grid, WALL, AIR);
            let (w, d, t) = (grid.wide, grid.deep, grid.tall);
            ball(&mut cells, grid, [w / 3, d / 4, t / 2], w / 8, SAND);
            ball(&mut cells, grid, [2 * w / 3, d / 4, t / 2], w / 8, WATER);
            for x in w / 4..3 * w / 4 {
                for z in t / 4..3 * t / 4 {
                    put(&mut cells, grid, [x, d / 2, z], WALL);
                }
            }
            cells
        },
    }
}

fn heated() -> Example {
    use heat::{AIR, ASH, FIRE, FUEL, ICE, STEAM, WALL, WATER, cell};
    let mut palette = [0x0010_1014; PALETTE];
    for (kind, colour) in [
        (AIR, 0x0010_1014),
        (WALL, 0x0050_5058),
        (ICE, 0x00B8_E0F0),
        (WATER, 0x0020_60C0),
        (STEAM, 0x00A0_A8B0),
        (FUEL, 0x0060_4020),
        (FIRE, 0x00FF_7010),
        (ASH, 0x0070_6860),
    ] {
        if let Some(slot) = palette.get_mut(kind as usize) {
            *slot = colour;
        }
    }
    Example {
        rule: "heat",
        params: heat::TUNED.to_vec(),
        palette,
        words: vec![
            cell(AIR, 293),
            cell(FIRE, 900),
            cell(FUEL, 293),
            cell(WATER, 300),
            cell(ICE, 250),
            cell(WALL, 293),
        ],
        scene: |grid| {
            let mut cells = boxed(grid, cell(WALL, 293), cell(AIR, 293));
            let (w, d, t) = (grid.wide, grid.deep, grid.tall);
            for x in w / 8..7 * w / 8 {
                for z in t / 8..7 * t / 8 {
                    for y in 3 * d / 4..d - 1 {
                        put(&mut cells, grid, [x, y, z], cell(WATER, 300));
                    }
                }
            }
            ball(
                &mut cells,
                grid,
                [w / 2, d / 3, t / 2],
                w / 6,
                cell(ICE, 250),
            );
            ball(
                &mut cells,
                grid,
                [w / 4, d / 2, t / 2],
                w / 10,
                cell(FUEL, 293),
            );
            ball(&mut cells, grid, [w / 4, d / 2, t / 2], 1, cell(FIRE, 900));
            cells
        },
    }
}

fn coded() -> Example {
    use code::{ADD, COPY, DATA, HALT, MOVE, NOP, PLUS_X, PLUS_Z, TURN, word};
    let mut palette = [0x000C_0C10; PALETTE];
    for (kind, colour) in [
        (NOP, 0x000C_0C10),
        (DATA, 0x0030_C060),
        (MOVE, 0x00F0_D040),
        (COPY, 0x0040_A0F0),
        (ADD, 0x00E0_50E0),
        (TURN, 0x00F0_8030),
        (HALT, 0x00C0_3030),
    ] {
        if let Some(slot) = palette.get_mut(kind as usize) {
            *slot = colour;
        }
    }
    Example {
        rule: "code",
        params: Vec::new(),
        palette,
        words: vec![
            word(NOP, 0, 0),
            word(MOVE, PLUS_X, 1),
            word(DATA, 0, 1),
            word(COPY, PLUS_Z, 0),
            word(ADD, PLUS_X, 0),
            word(HALT, 0, 0),
        ],
        scene: |grid| {
            let mut cells = vec![word(NOP, 0, 0); grid.cells()];
            let (w, d, t) = (grid.wide, grid.deep, grid.tall);
            for row in 0..d / 4 {
                let y = d / 8 + row * 2;
                put(
                    &mut cells,
                    grid,
                    [w / 16, y, t / 2],
                    word(MOVE, PLUS_X, row),
                );
                put(&mut cells, grid, [w / 2, y, t / 4], word(MOVE, PLUS_Z, row));
            }
            for x in w / 4..3 * w / 4 {
                put(&mut cells, grid, [x, 3 * d / 4, t / 2], word(DATA, 0, x));
            }
            put(
                &mut cells,
                grid,
                [w / 4, 3 * d / 4, t / 2],
                word(ADD, PLUS_X, 0),
            );
            for z in 0..t {
                put(&mut cells, grid, [3 * w / 4, d / 4, z], word(HALT, 0, 0));
            }
            cells
        },
    }
}

fn run<R: Rule>(host: &Host, rule: &R, example: &Example, asked: &Asked) -> Result<(), Failed> {
    let width = host.width();
    let grid = Grid::solid(asked.side, asked.side, asked.side)?;
    let view = View::of(asked.view, asked.view);
    let kinds = Kinds::named(example.rule).ok_or("no kinds for the rule")?;
    let cells = (example.scene)(grid);
    if asked.measure {
        return measured(host, grid, kinds, &cells);
    }

    let plan = Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )?;
    let (mut passes, engine) = live::wired3_by(rule, width, grid, &plan)?;
    passes.extend(pictures(width, grid, view, kinds)?);
    let sizes = view3::showing(live::sizes3(rule, &plan), width, view);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words])?;
    }
    held.write(STATE as usize, &cells)?;
    if !example.params.is_empty() {
        held.write_at(PARAMS as usize, FIRST_PARAM, &example.params)?;
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])?;
    live::seed(&mut held, grid, &plan)?;
    held.write_at(PIXELS as usize, palette_at(view), &example.palette)?;
    println!(
        "{} {width} {} {}x{}x{} view {}x{} {} blocks",
        host.name(),
        example.rule,
        grid.wide,
        grid.deep,
        grid.tall,
        view.wide,
        view.deep,
        plan.blocks
    );

    let mut window = minifb::Window::new(
        "pour3",
        view.wide as usize,
        view.deep as usize,
        minifb::WindowOptions::default(),
    )?;
    window.set_target_fps(60);

    let mut seen = Seen {
        axis: asked.axis,
        look: asked.look,
        layers: [grid.wide / 2, grid.deep / 2, grid.tall / 2],
        paused: false,
        chosen: 1,
    };
    if let Some(layer) = asked.layer
        && let Some(slot) = seen.layers.get_mut(seen.axis.index())
    {
        *slot = layer;
    }
    let still: Vec<Vec<Groups>> = engine
        .iter()
        .map(|stage| stage.iter().map(|_| Groups::Fixed(0)).collect())
        .collect();
    let mut frame = 0_u32;
    let mut shown = 0_u32;
    let mut picture = Vec::new();
    let mut cost = Duration::ZERO;
    let mut clock = Instant::now();
    let mut counted = 0_u32;
    while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
        let step = seen.keys(&window, grid, example.words.len());
        drawn(&window, &mut held, grid, view, &plan, &seen, &example.words)?;

        let moving = !seen.paused || step;
        let mut groups: Vec<&[Groups]> = if moving { &engine } else { &still }
            .iter()
            .map(Vec::as_slice)
            .collect();
        let chosen = choosing(width, view, seen.axis, seen.look);
        groups.push(&chosen);
        for axis in Axis::ALL {
            let layer = seen.layers.get(axis.index()).copied().unwrap_or(0);
            held.write_at(PARAMS as usize, axis.layer_word(), &[layer])?;
        }
        let (spent, got) = held.staged_reading(&groups, PIXELS as usize, view.pixels())?;
        picture = got;
        cost += spent;
        counted += 1;
        if moving {
            frame = frame.wrapping_add(1);
        }
        window.update_with_buffer(&picture, view.wide as usize, view.deep as usize)?;
        shown += 1;

        if clock.elapsed().as_secs_f64() >= 0.5 {
            let listed = held.read(GRAND as usize, 1)?.first().copied().unwrap_or(0);
            let layer = seen.layers.get(seen.axis.index()).copied().unwrap_or(0);
            let word = example.words.get(seen.chosen).copied().unwrap_or(0);
            window.set_title(&format!(
                "pour3 {} axis {:?} layer {layer} {:?} frame {frame} listed {listed} word {word:#010x} {:.2} ms{}",
                example.rule,
                seen.axis,
                seen.look,
                cost.as_secs_f64() * 1000.0 / f64::from(counted.max(1)),
                if seen.paused { " paused" } else { "" }
            ));
            clock = Instant::now();
            cost = Duration::ZERO;
            counted = 0;
        }
        if asked.frames.is_some_and(|last| shown >= last) {
            break;
        }
    }
    if let Some(path) = &asked.shot {
        std::fs::write(path, png(&picture, view.wide, view.deep))?;
        println!("{path}: frame {frame}, {}x{}", view.wide, view.deep);
    }
    Ok(())
}

struct Seen {
    axis: Axis,
    look: Look,
    layers: [u32; 3],
    paused: bool,
    chosen: usize,
}

impl Seen {
    fn keys(&mut self, window: &minifb::Window, grid: Grid, words: usize) -> bool {
        use minifb::{Key, KeyRepeat};
        let pressed = |key| window.is_key_pressed(key, KeyRepeat::No);
        let held = |key| window.is_key_pressed(key, KeyRepeat::Yes);
        if pressed(Key::Tab) {
            self.axis = self.axis.next();
        }
        for (key, axis) in [(Key::X, Axis::X), (Key::Y, Axis::Y), (Key::Z, Axis::Z)] {
            if pressed(key) {
                self.axis = axis;
            }
        }
        if pressed(Key::D) {
            self.look = match self.look {
                Look::Slice => Look::Depth,
                Look::Depth => Look::Slice,
            };
        }
        if pressed(Key::Space) {
            self.paused = !self.paused;
        }
        let quick = window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift);
        let by = if quick { 8 } else { 1 };
        let length = extent(grid, self.axis);
        if let Some(layer) = self.layers.get_mut(self.axis.index()) {
            if held(Key::Up) || held(Key::W) {
                *layer = (*layer + by).min(length - 1);
            }
            if held(Key::Down) || held(Key::S) {
                *layer = layer.saturating_sub(by);
            }
        }
        let digits = [
            Key::Key1,
            Key::Key2,
            Key::Key3,
            Key::Key4,
            Key::Key5,
            Key::Key6,
            Key::Key7,
            Key::Key8,
            Key::Key9,
        ];
        for (at, key) in digits.into_iter().enumerate().take(words) {
            if pressed(key) {
                self.chosen = at;
            }
        }
        self.paused && (pressed(Key::N) || held(Key::Right))
    }
}

fn drawn(
    window: &minifb::Window,
    held: &mut Resident<'_>,
    grid: Grid,
    view: View,
    plan: &Plan,
    seen: &Seen,
    words: &[u32],
) -> Result<(), Failed> {
    let writing = window.get_mouse_down(minifb::MouseButton::Left);
    let erasing = window.get_mouse_down(minifb::MouseButton::Right);
    if !writing && !erasing {
        return Ok(());
    }
    let Some((x, y)) = window.get_mouse_pos(minifb::MouseMode::Discard) else {
        return Ok(());
    };
    let word = if erasing {
        words.first().copied().unwrap_or(0)
    } else {
        words.get(seen.chosen).copied().unwrap_or(0)
    };
    let layer = seen.layers.get(seen.axis.index()).copied().unwrap_or(0);
    let centre = (x as u32, y as u32);
    let mut cells = Vec::new();
    for down in centre.1.saturating_sub(REACH)..=centre.1 + REACH {
        for across in centre.0.saturating_sub(REACH)..=centre.0 + REACH {
            if across >= view.wide || down >= view.deep {
                continue;
            }
            let point = cell_under(grid, view, seen.axis, layer, (across, down));
            cells.push((Solid::cell(grid, point), word));
        }
    }
    cells.sort_unstable();
    cells.dedup();
    live::put3(held, grid, plan, &cells)?;
    Ok(())
}

fn measured(host: &Host, grid: Grid, kinds: Kinds, cells: &[u32]) -> Result<(), Failed> {
    let width = host.width();
    let view = View::of(1024, 512);
    let passes = pictures(width, grid, view, kinds)?;
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut sizes = [1_usize; BINDINGS as usize];
    if let Some(params) = sizes.get_mut(PARAMS as usize) {
        *params = Dimension::Three.params_words();
    }
    if let Some(state) = sizes.get_mut(STATE as usize) {
        *state = grid.cells();
    }
    let sizes = view3::showing(sizes, width, view);
    let mut held = host.resident(&sizes, &modules)?;
    for (binding, words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0_u32; *words])?;
    }
    held.write(STATE as usize, cells)?;
    println!(
        "{} width {width} world {}x{}x{} view {}x{}, {TIMED} timed runs each, device time",
        host.name(),
        grid.wide,
        grid.deep,
        grid.tall,
        view.wide,
        view.deep
    );
    for (axis, look) in SHOWN {
        held.write_at(PARAMS as usize, axis.layer_word(), &[0])?;
        let chosen = choosing(width, view, axis, look);
        let stages: Vec<&[Groups]> = vec![&chosen];
        for _ in 0..3 {
            held.staged(&stages)?;
        }
        let mut times = Vec::with_capacity(TIMED);
        for _ in 0..TIMED {
            held.staged(&stages)?;
            times.push(held.on_device().ok_or("the device keeps no timestamps")?);
        }
        times.sort_unstable();
        let least = times.first().copied().unwrap_or_default();
        let middle = times.get(TIMED / 2).copied().unwrap_or_default();
        println!(
            "{axis:?} {look:?} depth {}: min {:.3} ms, median {:.3} ms",
            extent(grid, axis),
            least.as_secs_f64() * 1000.0,
            middle.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

fn crc_table() -> [u32; 256] {
    let mut table = [0_u32; 256];
    for (n, slot) in (0_u32..).zip(table.iter_mut()) {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    table
}

fn crc(table: &[u32; 256], bytes: &[u8]) -> u32 {
    let mut c = u32::MAX;
    for &byte in bytes {
        let at = ((c ^ u32::from(byte)) & 0xFF) as usize;
        c = table.get(at).copied().unwrap_or(0) ^ (c >> 8);
    }
    c ^ u32::MAX
}

fn chunk(out: &mut Vec<u8>, table: &[u32; 256], kind: &[u8; 4], data: &[u8]) {
    let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&length.to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc(table, &body).to_be_bytes());
}

fn png(picture: &[u32], wide: u32, deep: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity(picture.len() * 3 + deep as usize);
    for row in picture.chunks(wide as usize) {
        raw.push(0);
        for &pixel in row {
            raw.extend_from_slice(pixel.to_be_bytes().get(1..4).unwrap_or(&[0, 0, 0]));
        }
    }
    let mut packed = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xFFFF).peekable();
    while let Some(block) = blocks.next() {
        packed.push(u8::from(blocks.peek().is_none()));
        let length = u16::try_from(block.len()).unwrap_or(u16::MAX);
        packed.extend_from_slice(&length.to_le_bytes());
        packed.extend_from_slice(&(!length).to_le_bytes());
        packed.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    packed.extend_from_slice(&((b << 16) | a).to_be_bytes());

    let table = crc_table();
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&wide.to_be_bytes());
    header.extend_from_slice(&deep.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, &table, b"IHDR", &header);
    chunk(&mut out, &table, b"IDAT", &packed);
    chunk(&mut out, &table, b"IEND", &[]);
    out
}
