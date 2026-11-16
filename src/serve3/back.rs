use std::time::{Duration, Instant};

use crate::branch::format::{self, Image, State, UNIFORM};
use crate::wire::{Json, counted, map, text};

use super::budget::Stop;
use super::errors::{Code, Refusal};
use super::journal::Entry;
use super::schema::{property, whole, with};
use super::tools::{int, limited, refused, tool, world_number};
use super::{Failed, STEP_MOST, Server, Tool};

pub const TOOLS: &[Tool] = &[Tool {
    name: "grit_world_back",
    shown: back_shown,
    run: walked,
}];

pub const REVERSIBLE: &str = "a law declaring \"reversible\": true must be lifting steps over its words, 8 a plane: each step adds, subtracts or xors to one word, wrapping, a value made only of other words, params, frame, at, constants and looks at frozen planes, never of the word itself; anything else is refused not_lifting at the step's path. Its inverse, the steps undone in reverse order, is an ordinary law with its own hash. grit_world_back walks a world of such a law back, undoing frame f with the inverse at frame f's phase and params, to frame 0 at the earliest; it does not undo writes or params. home true checks the walk: a copy steps forward again, and a walk that does not come back bit for bit answers strayed with the first cell that differs and leaves the world as it was";

fn back_shown() -> Json {
    tool(
        "grit_world_back",
        &format!(
            "Walk a world of a reversible law back 1 to {STEP_MOST} frames, each undone by the law's inverse at its own phase. home true steps a copy forward again and answers strayed unless it comes home."
        ),
        vec![
            ("world", world_number()),
            (
                "frames",
                with(
                    whole("how many frames; 1 if left out", 1, i64::from(STEP_MOST)),
                    vec![("default", Json::Int(1))],
                ),
            ),
            (
                "home",
                with(
                    property(
                        "boolean",
                        "check the walk by stepping a copy forward again; false if left out",
                    ),
                    vec![("default", Json::Bool(false))],
                ),
            ),
        ],
        &["world"],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strayed {
    pub frames: u32,
    pub frame: u32,
    pub cell: [u32; 3],
    pub plane: u32,
    pub oct: [u32; 3],
    pub was: u32,
    pub now: u32,
}

impl Strayed {
    #[must_use]
    pub fn refusal(&self) -> Refusal {
        Refusal::new(
            Code::Strayed,
            "$.home",
            format!(
                "stepping the walked-back world forward {} frames again did not come home: cell {:?} of plane {}, in oct {:?} of frame {}, was {} and came back {}; the world is left as it was",
                self.frames, self.cell, self.plane, self.oct, self.frame, self.was, self.now
            ),
        )
    }

    #[must_use]
    pub fn told(&self) -> Vec<(&'static str, Json)> {
        let three = |held: [u32; 3]| Json::List(held.iter().map(|one| counted(*one)).collect());
        vec![(
            "strayed",
            map(vec![
                ("cell", three(self.cell)),
                ("plane", counted(self.plane)),
                ("oct", three(self.oct)),
                ("frame", counted(self.frame)),
                ("was", counted(self.was)),
                ("now", counted(self.now)),
            ]),
        )]
    }
}

impl std::fmt::Display for Strayed {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}", self.refusal().message)
    }
}

impl std::error::Error for Strayed {}

struct Reader<'image> {
    image: &'image Image,
    planes: usize,
    lattice: [u32; 3],
    bricks: usize,
    per: usize,
    held: usize,
}

impl<'image> Reader<'image> {
    fn new(image: &'image Image, sides: [u32; 3], planes: u32) -> Self {
        let lattice = format::lattice_of(image.size, sides);
        let per = format::brick_cells(sides).max(1);
        let planes = planes.max(1) as usize;
        let held = match &image.state {
            State::Dense(_) => 0,
            State::Sparse { pool, .. } => pool.len() / (per * planes),
        };
        Self {
            image,
            planes,
            lattice,
            bricks: lattice.iter().map(|&side| side as usize).product(),
            per,
            held,
        }
    }

    fn uniform(&self, brick: usize) -> Option<Vec<u32>> {
        let State::Sparse { table, .. } = &self.image.state else {
            return None;
        };
        if table.get(brick * 2).copied().unwrap_or(UNIFORM) != UNIFORM {
            return None;
        }
        Some(
            (0..self.planes)
                .map(|plane| {
                    let at = match plane {
                        0 => brick * 2 + 1,
                        _ => 2 * self.bricks + (plane - 1) * self.bricks + brick,
                    };
                    table.get(at).copied().unwrap_or_default()
                })
                .collect(),
        )
    }

    fn word(&self, brick: usize, offset: usize, point: [u32; 3], plane: usize) -> u32 {
        match &self.image.state {
            State::Dense(cells) => {
                let size = self.image.size;
                let index = (point[2] as usize * size[1] as usize + point[1] as usize)
                    * size[0] as usize
                    + point[0] as usize;
                let stride = size.iter().map(|&side| side as usize).product::<usize>();
                cells
                    .get(plane * stride + index)
                    .copied()
                    .unwrap_or_default()
            }
            State::Sparse { table, pool, .. } => match self.uniform(brick) {
                Some(values) => values.get(plane).copied().unwrap_or_default(),
                None => {
                    let slot = table.get(brick * 2).copied().unwrap_or_default() as usize;
                    pool.get(plane * self.held * self.per + slot * self.per + offset)
                        .copied()
                        .unwrap_or_default()
                }
            },
        }
    }
}

#[must_use]
pub fn first_differing(was: &Image, now: &Image, planes: u32) -> Option<([u32; 3], u32, u32, u32)> {
    if was.state == now.state {
        return None;
    }
    let sides = match &was.state {
        State::Dense(_) => was.size,
        State::Sparse { sides, .. } => *sides,
    };
    let before = Reader::new(was, sides, planes);
    let after = Reader::new(now, sides, planes);
    let across = before.lattice[0] as usize;
    let down = before.lattice[1] as usize;
    let [wide, deep, _] = sides.map(|side| side as usize);
    for brick in 0..before.bricks {
        let held = before.uniform(brick);
        if held.is_some() && held == after.uniform(brick) {
            continue;
        }
        let corner = [
            brick % across,
            (brick / across) % down,
            brick / across / down,
        ];
        for offset in 0..before.per {
            let inside = [offset % wide, (offset / wide) % deep, offset / wide / deep];
            let placed = |axis: usize, side: u32| {
                let at = corner.get(axis).copied().unwrap_or(0) * side as usize
                    + inside.get(axis).copied().unwrap_or(0);
                u32::try_from(at).unwrap_or(0)
            };
            let point = [
                placed(0, sides[0]),
                placed(1, sides[1]),
                placed(2, sides[2]),
            ];
            for plane in 0..before.planes {
                let one = before.word(brick, offset, point, plane);
                let other = after.word(brick, offset, point, plane);
                if one != other {
                    return Some((point, u32::try_from(plane).unwrap_or(0), one, other));
                }
            }
        }
    }
    None
}

fn oct_of(cell: [u32; 3], size: [u32; 3], frame: u32) -> [u32; 3] {
    let phase = frame & 1;
    let along = |at: u32, side: u32| {
        let side = side.max(2);
        (at + side - phase) % side / 2
    };
    [
        along(cell[0], size[0]),
        along(cell[1], size[1]),
        along(cell[2], size[2]),
    ]
}

fn not_reversible(rule: &str) -> Failed {
    refused(
        Code::NotReversible,
        "$.world",
        format!(
            "the world runs {rule}, which is not a law declaring \"reversible\": true, so it has no inverse to walk back with; nothing ran"
        ),
    )
}

fn walked(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let number = int(held, "world")?;
    let frames = int(held, "frames")?;
    let home = matches!(held.get("home"), Some(Json::Bool(true)));
    let live = server.world(held)?;
    let rule = live.world.rule().to_owned();
    let inverse = match live.world.law().map(crate::law::Law::inverse) {
        Some(Ok(Some(inverse))) if live.world.reversible() => inverse.hex(),
        _ => return Err(not_reversible(&rule)),
    };
    let frame = live.world.frame();
    if frames > frame {
        return Err(limited(
            Code::PastStart,
            "$.frames",
            format!(
                "the world is at frame {frame}, so it walks back at most {frame} frames; nothing ran"
            ),
            counted(frame),
        ));
    }
    if home {
        let bytes = server.book.held.get(&number).map_or(0, |held| held.bytes);
        server
            .book
            .memory
            .admits(bytes, "the copy that checks the walk")?;
    }
    let owed = server.owed(number)?;
    let start = if home {
        Some(server.world(held)?.world.image()?)
    } else {
        None
    };
    let mut pace = server
        .book
        .held
        .get(&number)
        .map(|held| held.pace)
        .ok_or("the world has no journal")?;
    let mut done = 0_u32;
    let mut spent = Duration::ZERO;
    let mut stopped = None;
    let mut failure: Option<Failed> = None;
    let mut batch = 0_u32;
    while done < frames {
        if let Some(stop) = server.stop() {
            stopped = Some(stop);
            break;
        }
        let step = if batch == 0 {
            pace.opening(frames - done, server.clock.left())
        } else {
            pace.batch(frames - done, server.clock.left())
        };
        if step == 0 {
            stopped = Some(Stop::Budget);
            break;
        }
        if let Err(why) = super::tools::faults::batch() {
            failure = Some(why);
            break;
        }
        let began = Instant::now();
        super::tools::faults::stall(step);
        let ran = server.world(held)?.world.back(step);
        let took = began.elapsed();
        if let Err(why) = ran {
            failure = Some(why);
            break;
        }
        if super::tools::wedged_after(server, batch) {
            failure = Some(Box::new(crate::host::Error::Wedged { after: took }));
            break;
        }
        pace.ran(step, took);
        let stats = &mut server.book.stats;
        stats
            .batches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if took > stats.longest {
            stats.longest = took;
            stats.longest_frames = step;
        }
        spent = spent.saturating_add(took);
        done += step;
        batch += 1;
    }
    if let Some(held) = server.book.held.get_mut(&number) {
        held.pace = pace;
    }
    if let Some(why) = failure {
        if !home && done > 0 {
            server.journaled(number, Entry::Back(done), spent);
        }
        let frame = server
            .book
            .held
            .get(&number)
            .map_or(0, |held| held.ledger.frame);
        let (mut refusal, _) = super::refused_with(why.as_ref());
        refusal.message = format!(
            "{}; {} of the {frames} frames were walked back before it, and the world is at frame {frame}",
            refusal.message,
            if home { 0 } else { done }
        );
        if super::lost_device(why.as_ref()) {
            refusal.code = Code::DeviceLost;
            return Err(Box::new(super::tools::Lost(refusal)));
        }
        return Err(Box::new(refusal));
    }
    if let Some(start) = start.as_ref()
        && done > 0
    {
        checked(server, number, start, done)?;
    }
    let journal = if done > 0 {
        server.journaled(number, Entry::Back(done), spent).or(owed)
    } else {
        owed
    };
    let snapshot = server.settle(number);
    let live = server.world(held)?;
    let listed = live.world.listed()?;
    let mut answer = vec![
        ("world", counted(number)),
        ("frames", counted(frames)),
        ("frame", counted(live.world.frame())),
        ("listed", counted(listed)),
        ("inverse", text(&inverse)),
    ];
    if home {
        answer.push(("home", Json::Bool(true)));
    }
    if let Some(used) = live.world.pool_use()? {
        answer.push(("sparse", Json::Bool(true)));
        answer.push(("pool_use", super::pooled(used)));
    }
    if let Some(stop) = stopped {
        answer.push(("stopped", text(stop.name())));
        answer.push(("ran", counted(done)));
        answer.push(("hint", text(stop.code().hint())));
    }
    if let Some(note) = journal.or(snapshot) {
        answer.push(("journal", note));
    }
    Ok(map(answer))
}

fn probed(
    server: &mut Server<'_>,
    number: u32,
    start: &Image,
    done: u32,
) -> Result<Option<Strayed>, Failed> {
    let live = server.world_at(number, "$.world")?;
    let planes = live.world.planes();
    let walked = live.world.frame();
    let mut probe = live.world.fork()?;
    probe.step(done)?;
    strayed(server, &mut probe)?;
    let again = probe.image()?;
    drop(probe);
    Ok(
        first_differing(start, &again, planes).map(|(cell, plane, was, now)| Strayed {
            frames: done,
            frame: walked,
            cell,
            plane,
            oct: oct_of(cell, start.size, walked),
            was,
            now,
        }),
    )
}

fn checked(server: &mut Server<'_>, number: u32, start: &Image, done: u32) -> Result<(), Failed> {
    let host = server.host.ok_or(crate::host::Error::NoComputeDevice)?;
    let why: Failed = match probed(server, number, start, done) {
        Ok(None) => return Ok(()),
        Ok(Some(strayed)) => Box::new(strayed),
        Err(why) if super::lost_device(why.as_ref()) => return Err(why),
        Err(why) => why,
    };
    let rebuilt = super::law::opened(host, start)?;
    server.world_at(number, "$.world")?.world = rebuilt;
    Err(why)
}

#[cfg(feature = "faults")]
fn strayed(server: &Server<'_>, probe: &mut crate::branch::World<'_>) -> Result<(), Failed> {
    let armed = server
        .book
        .faults
        .lock()
        .map(|mut faults| std::mem::take(&mut faults.stray_walk))
        .unwrap_or(false);
    if !armed {
        return Ok(());
    }
    let held = probe.cells_at(&[[0, 0, 0]])?;
    let word = held.first().copied().unwrap_or_default();
    probe.write(&[([0, 0, 0], word ^ 1)])
}

#[cfg(not(feature = "faults"))]
#[allow(clippy::unnecessary_wraps)]
const fn strayed(_: &Server<'_>, _: &mut crate::branch::World<'_>) -> Result<(), Failed> {
    Ok(())
}
