use std::time::Duration;

use crate::law::chip::{Circuit, Read, Rest, Spot, Wiring};
use crate::machine::Machine;
use crate::netlist::Gate;
use crate::serve::{Effort, Halt};
use crate::stencil::{self, Bit, Border, Rows, Sample, Stencil};
use crate::wire::{Json, counted, map, text};

use super::budget::{Guard, Stop};
use super::errors::{Code, Refusal};
use super::gradient::Told;
use super::schema::{choice, object, property, whole, with, word};
use super::tools::{int, ints, limited, refused, tool, unread};
use super::{Failed, Server, Tool};

pub const TOOLS: &[Tool] = &[Tool {
    name: "grit_law_learn",
    shown: learn_shown,
    run: learned,
}];

pub const EXAMPLES_MOST: u32 = 256;

pub const CELLS_MOST: u64 = 1 << 20;

pub const READS_MOST: u32 = 3;

pub const WRITES_MOST: u32 = 8;

pub const INPUTS_MOST: u32 = 27;

pub const GATES: u32 = 16;

pub const GATES_MOST: u32 = 256;

pub const CROWD: u32 = 4096;

pub const ROUNDS: u32 = 2000;

pub const LISTED: usize = 64;

pub const TABLE_INPUTS: u32 = 9;

pub const PATIENCE: u32 = 2;

pub const MARGIN: u32 = 2;

pub const MINIMAL: &str = "each row the examples never showed takes the answer of the smallest circuit the gate ladder found that answers every seen row";

pub const SYMMETRIC: &str = "the ladder finds the smallest circuit f that answers every seen row and every rotation and reflection of one (8 for 3x3, 48 for 3x3x3); the law answers output j 1 exactly when f answers it 1 on some rotation or reflection of the neighbourhood, so every row the examples never showed is decided alike with its images";

pub const ZERO_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; to read 0 past an example's edge, write it at the origin of a world at least one cell longer than it along each axis the stencil reads, every other cell of the read planes 0";

pub const WRAPPED_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; an inline example wraps on itself, so it runs alike on a world whose sides equal its own along each axis the stencil reads (3x3 reads x and y, so its tall is free; a world's tall is at least 2), and a world example's box reads its neighbours from that world";

pub const LEARN: &str = "grit_law_learn builds the stencil's rows from example worlds, refuses a row seen with two outputs (conflict) and, under rule symmetric, rows whose rotations or reflections disagree (asymmetric), then walks a gate ladder down from gates as grit_least does, with grit_machine's search over the seen rows: each rung starts its crowd at most at what one device binding holds and doubles it while unsolved up to 65536 or that limit, whichever is less; the answer names a lowered crowd as crowd_capped. The ladder ends after two failed rungs in a row. The law is the smallest solved circuit, wired as grit_law_from_chip would wire it, with the row order below. A search checks its budget and cancel before each 50 rounds of a rung and stops when the last 50 rounds it measured, or the device's first measured round times 50, do not fit the time left, and before each search's build when the last build it measured in this call, less its wait for other calls' builds, does not fit; a build cannot stop part way, and its kernel loops over the rows searched (under symmetric every image of every seen row), so it does not grow with them; before the ladder the call builds the kernel of its smallest rung, then larger ones while each build, grown with the square of its words from the closest build measured, stays within half the device's patience; a top rung past that is refused learn_limit at gates with the most gates that fit, so an answer can come late by up to one build the device's evidence puts within half its patience; a stopped ladder answers the smallest circuit it solved so far, with stopped budget or cancelled, and refuses budget or cancelled when none. The seen rows, and under symmetric their images, are refused learn_limit before any search when they pass the rows one kernel holds; the expansion stops as soon as it passes them, so it never holds more. The expansion and the count of decided rows check budget and cancel every 4096 rows or 64-row blocks and refuse budget or cancelled; after a stopped ladder the count runs to its end, at most two to the inputs less 6 blocks times the inputs plus the law's gates, so that the smallest circuit solved so far is answered with its counts. Under symmetric the ladder starts at most at the gates whose copies and ors fit the 600 gates a law always holds, and the answer names a lowered start as gates_capped. The same request gives the same answer on the same device when it answers stopped ladder; a stop on budget or cancel depends on the time measured, and its ladder is a prefix of that answer's ladder, with the smallest circuit solved in it";

pub mod faults {
    #[cfg(feature = "faults")]
    use std::cell::Cell;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    use crate::host::Host;

    #[cfg(feature = "faults")]
    thread_local! {
        static CHECKS: Cell<u64> = const { Cell::new(0) };
        static CANCEL: Cell<Option<u64>> = const { Cell::new(None) };
        static CHARGE: Cell<Option<u64>> = const { Cell::new(None) };
        static KERNEL: Cell<usize> = const { Cell::new(0) };
        static BUILDS: Cell<u64> = const { Cell::new(0) };
        static HELD_UP: Cell<Option<(u64, Duration)>> = const { Cell::new(None) };
        static CEILING: Cell<Option<u64>> = const { Cell::new(None) };
        static SLOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static CALIBRATED: Cell<u64> = const { Cell::new(0) };
    }

    #[cfg(feature = "faults")]
    pub fn builds_slowed_by(each_word: Duration) {
        SLOW.set(each_word);
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn calibrations() -> u64 {
        CALIBRATED.get()
    }

    #[cfg(feature = "faults")]
    pub(super) fn calibrating(words: usize) -> Duration {
        CALIBRATED.set(CALIBRATED.get().saturating_add(1));
        SLOW.get()
            .saturating_mul(u32::try_from(words).unwrap_or(u32::MAX))
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn calibrating(_: usize) -> Duration {
        Duration::ZERO
    }

    #[cfg(feature = "faults")]
    pub fn ceiling_at(bytes: u64) {
        CEILING.set(Some(bytes));
    }

    #[cfg(feature = "faults")]
    pub(super) fn ceiling(host: &Host) -> u64 {
        CEILING.get().unwrap_or_else(|| host.ceiling())
    }

    #[cfg(not(feature = "faults"))]
    pub(super) fn ceiling(host: &Host) -> u64 {
        host.ceiling()
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn kernel_most() -> usize {
        KERNEL.get()
    }

    #[cfg(feature = "faults")]
    pub fn held_up_at(build: u64, wait: Duration) {
        BUILDS.set(0);
        HELD_UP.set(Some((build, wait)));
    }

    #[cfg(feature = "faults")]
    pub(super) fn kernel(words: usize) {
        KERNEL.set(KERNEL.get().max(words));
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn kernel(_: usize) {}

    #[cfg(feature = "faults")]
    pub(super) fn held_up() -> Duration {
        let build = BUILDS.get();
        BUILDS.set(build.saturating_add(1));
        match HELD_UP.get() {
            Some((at, extra)) if at == build => {
                HELD_UP.set(None);
                extra
            }
            _ => Duration::ZERO,
        }
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn held_up() -> Duration {
        Duration::ZERO
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn checks() -> u64 {
        CHECKS.get()
    }

    #[cfg(feature = "faults")]
    pub fn cancel_after(checks: u64) {
        CANCEL.set(Some(checks));
    }

    #[cfg(feature = "faults")]
    pub fn charge_after(checks: u64) {
        CHARGE.set(Some(checks));
    }

    #[cfg(feature = "faults")]
    pub fn disarm() {
        CANCEL.set(None);
        CHARGE.set(None);
        HELD_UP.set(None);
        CEILING.set(None);
        SLOW.set(Duration::ZERO);
        CALIBRATED.set(0);
        KERNEL.set(0);
    }

    #[cfg(feature = "faults")]
    fn due(held: &'static std::thread::LocalKey<Cell<Option<u64>>>) -> bool {
        match held.get() {
            Some(0) => {
                held.set(None);
                true
            }
            Some(left) => {
                held.set(Some(left - 1));
                false
            }
            None => false,
        }
    }

    #[cfg(feature = "faults")]
    pub(super) fn checked(
        host: Option<&Host>,
        cancel: Option<&Arc<AtomicBool>>,
        faults: &std::sync::Mutex<super::super::Faults>,
    ) -> Duration {
        CHECKS.set(CHECKS.get().saturating_add(1));
        if due(&CANCEL)
            && let Some(flag) = cancel
        {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let wedge = faults
            .lock()
            .map(|mut held| match held.wedge_after_checks {
                Some(0) => {
                    held.wedge_after_checks = None;
                    true
                }
                Some(left) => {
                    held.wedge_after_checks = Some(left - 1);
                    false
                }
                None => false,
            })
            .unwrap_or(false);
        if wedge && let Some(host) = host {
            host.wedge();
        }
        if due(&CHARGE) {
            Duration::MAX
        } else {
            Duration::ZERO
        }
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn checked(_: Option<&Host>, _: Option<&Arc<AtomicBool>>) -> Duration {
        Duration::ZERO
    }
}

struct Watch<'host> {
    guard: Guard,
    host: Option<&'host crate::host::Host>,
    stop: Option<Stop>,
    began: Option<std::time::Instant>,
    built: Option<Duration>,
    waited: Duration,
    held_up: Duration,
    crowd_most: u32,
    #[cfg(feature = "faults")]
    faults: std::sync::Arc<std::sync::Mutex<super::Faults>>,
}

impl Watch<'_> {
    #[cfg(feature = "faults")]
    fn charged(&self) -> Duration {
        faults::checked(self.host, self.guard.cancel.as_ref(), &self.faults)
    }

    #[cfg(not(feature = "faults"))]
    fn charged(&self) -> Duration {
        faults::checked(self.host, self.guard.cancel.as_ref())
    }
}

impl Halt for Watch<'_> {
    fn building(&mut self) -> bool {
        let next = self.built.unwrap_or(Duration::ZERO);
        if self.over(next) {
            return true;
        }
        self.began = Some(std::time::Instant::now());
        self.waited = Duration::ZERO;
        self.held_up = Duration::ZERO;
        false
    }

    fn crowd_most(&self) -> u32 {
        self.crowd_most
    }

    fn rolled(&self) -> bool {
        true
    }

    fn kernel(&mut self, words: usize) {
        faults::kernel(words);
    }

    fn waited(&mut self, wait: Duration) {
        let extra = faults::held_up();
        self.held_up = self.held_up.saturating_add(extra);
        self.waited = self.waited.saturating_add(wait).saturating_add(extra);
    }

    fn halted(&mut self, next: Duration) -> bool {
        if let Some(began) = self.began.take() {
            self.built = Some(
                began
                    .elapsed()
                    .saturating_add(self.held_up)
                    .saturating_sub(self.waited),
            );
        }
        self.over(next)
    }
}

impl Watch<'_> {
    fn over(&mut self, next: Duration) -> bool {
        let charged = self.charged();
        if let Some(stop) = self.guard.stop() {
            self.stop = Some(stop);
            return true;
        }
        if next.saturating_add(charged) > self.guard.left() {
            self.stop = Some(Stop::Budget);
            return true;
        }
        false
    }
}

fn spot_shown(what: &str) -> Json {
    object(
        vec![
            (
                "plane",
                whole(
                    &format!("the plane {what}"),
                    0,
                    i64::from(crate::law::limits::CHANNELS - 1),
                ),
            ),
            ("bit", whole("the bit of that plane's word", 0, 31)),
        ],
        &["plane", "bit"],
    )
}

fn list_shown(items: Json, what: &str, least: i64, most: i64) -> Json {
    map(vec![
        ("type", text("array")),
        ("items", items),
        ("description", text(what)),
        ("minItems", Json::Int(least)),
        ("maxItems", Json::Int(most)),
    ])
}

fn words_shown(what: &str) -> Json {
    list_shown(
        list_shown(
            word(),
            "one word a cell, x fastest, then y, then z; a larger example is read from a world",
            1,
            most(super::READ_MOST),
        ),
        what,
        1,
        i64::from(READS_MOST.max(WRITES_MOST)),
    )
}

fn most(held: u64) -> i64 {
    i64::try_from(held).unwrap_or(i64::MAX)
}

fn corner_shown(what: &str) -> Json {
    list_shown(word(), what, 3, 3)
}

fn effort_shown(what: &str, least: i64, most: i64, or: i64) -> Json {
    with(
        whole(&format!("{what}; {or} if left out"), least, most),
        vec![("default", Json::Int(or))],
    )
}

fn learn_shown() -> Json {
    let example = object(
        vec![
            (
                "size",
                list_shown(
                    whole("a side", 1, i64::from(u32::MAX)),
                    "inline: the example's [wide, deep, tall]",
                    3,
                    3,
                ),
            ),
            (
                "input",
                words_shown(
                    "inline: one list of words for each read, in order; the read takes its bit",
                ),
            ),
            (
                "output",
                words_shown(
                    "inline: one list of words for each write, in order; the write's bit of a word is the wanted output",
                ),
            ),
            (
                "world",
                whole(
                    "a world whose read planes hold the input; give this or size, input and output",
                    1,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "output_world",
                whole(
                    "the world whose write planes hold the wanted output; world if left out",
                    1,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "from",
                corner_shown("with world: the box's least corner; [0, 0, 0] if left out"),
            ),
            (
                "to",
                corner_shown(
                    "with world: the box's greatest corner; the world's last cell if left out",
                ),
            ),
        ],
        &[],
    );
    tool(
        "grit_law_learn",
        "Learn a law for grit_world_create from example worlds: the server builds the stencil's rows, refuses conflicts and states how it decided rows never seen. Row bit j is input j.",
        vec![
            (
                "stencil",
                choice(
                    &format!("the neighbourhood: 3x3 (dz 0) or 3x3x3. {}", stencil::ORDER),
                    &["3x3", "3x3x3"],
                ),
            ),
            (
                "reads",
                list_shown(
                    spot_shown("read at every place of the stencil"),
                    "the input bits, each read at every place; places times reads at most 27",
                    1,
                    i64::from(READS_MOST),
                ),
            ),
            (
                "writes",
                list_shown(
                    spot_shown("written at the cell"),
                    "the output bits, written at the cell itself",
                    1,
                    i64::from(WRITES_MOST),
                ),
            ),
            (
                "border",
                choice(
                    "what a neighbour past an example's edge reads: zero, or wrapped on the example (an inline example on itself, a world's box from that world)",
                    &["zero", "wrapped"],
                ),
            ),
            (
                "rule",
                choice(
                    "how rows the examples never showed are decided: minimal, the smallest circuit found; symmetric, alike under the stencil's rotations and reflections",
                    &["minimal", "symmetric"],
                ),
            ),
            (
                "examples",
                list_shown(
                    example,
                    "pairs of input and wanted output, inline or from worlds",
                    1,
                    i64::from(EXAMPLES_MOST),
                ),
            ),
            (
                "channels",
                whole(
                    "the planes of the world the law runs on; one past the highest plane read or written if left out",
                    1,
                    i64::from(crate::law::limits::CHANNELS),
                ),
            ),
            (
                "rest",
                with(
                    choice(
                        "what the other bits of a written word become; kept if left out",
                        &["kept", "cleared"],
                    ),
                    vec![("default", text("kept"))],
                ),
            ),
            (
                "name",
                with(
                    property("string", "the law's name; learned if left out"),
                    vec![("default", text("learned"))],
                ),
            ),
            (
                "gates",
                effort_shown(
                    "the gate budget the ladder starts from",
                    1,
                    i64::from(GATES_MOST),
                    i64::from(GATES),
                ),
            ),
            (
                "crowd",
                effort_shown(
                    "the crowd each rung starts with, doubled while unsolved up to 65536 or what one device binding holds",
                    64,
                    i64::from(crate::serve::CROWD),
                    i64::from(CROWD),
                ),
            ),
            (
                "rounds",
                effort_shown(
                    "generations a rung's search may run",
                    1,
                    1 << 20,
                    i64::from(ROUNDS),
                ),
            ),
            (
                "seed",
                effort_shown(
                    "where the search starts",
                    0,
                    i64::from(u32::MAX),
                    i64::from(crate::serve::SEEDING),
                ),
            ),
        ],
        &["stencil", "reads", "writes", "border", "rule", "examples"],
    )
}

fn bits_of(held: &Json, key: &str) -> Result<Vec<Bit>, Failed> {
    held.get(key)
        .and_then(Json::list)
        .ok_or_else(|| unread(key))?
        .iter()
        .map(|one| {
            Ok(Bit {
                plane: int(one, "plane")?,
                bit: int(one, "bit")?,
            })
        })
        .collect()
}

fn corner_of(held: &Json, key: &str) -> Result<Option<[u32; 3]>, Failed> {
    held.get(key)
        .map(|one| {
            let got = ints(one, key)?;
            <[u32; 3]>::try_from(got.as_slice()).map_err(|_| unread(key))
        })
        .transpose()
}

struct Owned {
    size: [u32; 3],
    inputs: Vec<Vec<u32>>,
    from: [u32; 3],
    inner: [u32; 3],
    outputs: Vec<Vec<u32>>,
}

impl Owned {
    fn sample(&self) -> Sample<'_> {
        Sample {
            size: self.size,
            inputs: self.inputs.iter().map(Vec::as_slice).collect(),
            from: self.from,
            inner: self.inner,
            outputs: self.outputs.iter().map(Vec::as_slice).collect(),
        }
    }
}

struct Asked {
    stencil: Stencil,
    reads: Vec<Bit>,
    writes: Vec<Bit>,
    border: Border,
}

fn cells_of(size: [u32; 3]) -> u64 {
    size.iter().map(|side| u64::from(*side)).product()
}

fn example_refused(at: usize, key: &str, message: String) -> Failed {
    let field = if key.is_empty() {
        format!("$.examples[{at}]")
    } else {
        format!("$.examples[{at}].{key}")
    };
    refused(Code::Example, &field, message)
}

fn inline(asked: &Asked, held: &Json, at: usize) -> Result<Owned, Failed> {
    let size = ints(held.get("size").ok_or_else(|| unread("size"))?, "size")?;
    let size = <[u32; 3]>::try_from(size.as_slice()).map_err(|_| unread("size"))?;
    let cells = cells_of(size);
    if cells > CELLS_MOST {
        return Err(limited(
            Code::LearnLimit,
            &format!("$.examples[{at}].size"),
            format!("the example holds {cells} cells, more than the {CELLS_MOST} a call takes"),
            Json::Int(most(CELLS_MOST)),
        ));
    }
    if asked.border == Border::Wrapped {
        for (axis, side) in size.iter().enumerate().take(asked.stencil.axes()) {
            if *side < 2 || !side.is_power_of_two() {
                return Err(example_refused(
                    at,
                    "size",
                    format!(
                        "side {axis} is {side}; a wrapped inline example runs alike only on a world of its own size, so each side the {} stencil reads across must be a power of two from 2",
                        asked.stencil.name()
                    ),
                ));
            }
        }
    }
    let lists = |key: &str, wanted: usize| -> Result<Vec<Vec<u32>>, Failed> {
        let listed = held
            .get(key)
            .and_then(Json::list)
            .ok_or_else(|| example_refused(at, key, format!("an inline example needs {key}")))?;
        if listed.len() != wanted {
            return Err(example_refused(
                at,
                key,
                format!(
                    "{key} holds {} lists of words, and {wanted} were wanted, one for each {}",
                    listed.len(),
                    if key == "input" { "read" } else { "write" }
                ),
            ));
        }
        listed
            .iter()
            .enumerate()
            .map(|(which, one)| {
                let words = ints(one, key)?;
                if words.len() as u64 != cells {
                    return Err(example_refused(
                        at,
                        &format!("{key}[{which}]"),
                        format!(
                            "the list holds {} words, and the example's {} by {} by {} cells need {cells}",
                            words.len(),
                            size[0],
                            size[1],
                            size[2]
                        ),
                    ));
                }
                Ok(words)
            })
            .collect()
    };
    Ok(Owned {
        size,
        inputs: lists("input", asked.reads.len())?,
        from: [0; 3],
        inner: size,
        outputs: lists("output", asked.writes.len())?,
    })
}

fn boxed(from: [u32; 3], size: [u32; 3], wrap: [u32; 3]) -> Vec<[u32; 3]> {
    let mut points = Vec::with_capacity(usize::try_from(cells_of(size)).unwrap_or(0));
    for z in 0..size[2] {
        for y in 0..size[1] {
            for x in 0..size[0] {
                points.push([
                    (from[0] + x) % wrap[0].max(1),
                    (from[1] + y) % wrap[1].max(1),
                    (from[2] + z) % wrap[2].max(1),
                ]);
            }
        }
    }
    points
}

fn from_world(
    server: &mut Server<'_>,
    asked: &Asked,
    held: &Json,
    at: usize,
) -> Result<Owned, Failed> {
    for key in ["size", "input", "output"] {
        if held.get(key).is_some() {
            return Err(example_refused(
                at,
                key,
                format!("{key} goes with an inline example; a world example reads its world"),
            ));
        }
    }
    let number = int(held, "world")?;
    let output = match held.get("output_world") {
        None => number,
        Some(_) => int(held, "output_world")?,
    };
    let field = format!("$.examples[{at}].world");
    let size = server.world_at(number, &field)?.world.size();
    let planes = server.world_at(number, &field)?.world.planes();
    let out_field = format!("$.examples[{at}].output_world");
    let out_size = server.world_at(output, &out_field)?.world.size();
    let out_planes = server.world_at(output, &out_field)?.world.planes();
    if out_size != size {
        return Err(example_refused(
            at,
            "output_world",
            format!(
                "world {output} is {out_size:?} and world {number} is {size:?}; an example's two worlds must be one size"
            ),
        ));
    }
    let from = corner_of(held, "from")?.unwrap_or([0; 3]);
    let to = corner_of(held, "to")?.unwrap_or(size.map(|side| side.saturating_sub(1)));
    for (key, corner) in [("from", from), ("to", to)] {
        if corner.iter().zip(size).any(|(one, side)| *one >= side) {
            return Err(limited(
                Code::Outside,
                &format!("$.examples[{at}].{key}"),
                format!("{key} {corner:?} is outside world {number} of {size:?} cells"),
                super::tools::sized(size.map(|side| side.saturating_sub(1))),
            ));
        }
    }
    if from.iter().zip(to).any(|(least, most)| *least > most) {
        return Err(refused(
            Code::Reversed,
            &format!("$.examples[{at}].from"),
            format!("from {from:?} is past to {to:?} on some axis"),
        ));
    }
    let inner = [
        to[0] - from[0] + 1,
        to[1] - from[1] + 1,
        to[2] - from[2] + 1,
    ];
    for (which, read) in asked.reads.iter().enumerate() {
        if read.plane >= planes {
            return Err(limited(
                Code::PlanePast,
                &format!("$.reads[{which}].plane"),
                format!(
                    "read {which} is on plane {}, past the {planes} planes of world {number}",
                    read.plane
                ),
                counted(planes.saturating_sub(1)),
            ));
        }
    }
    for (which, write) in asked.writes.iter().enumerate() {
        if write.plane >= out_planes {
            return Err(limited(
                Code::PlanePast,
                &format!("$.writes[{which}].plane"),
                format!(
                    "write {which} is on plane {}, past the {out_planes} planes of world {output}",
                    write.plane
                ),
                counted(out_planes.saturating_sub(1)),
            ));
        }
    }
    let (data_from, data_size, inner_from) = if asked.border == Border::Wrapped {
        let mut data_from = from;
        let mut data_size = inner;
        let mut inner_from = [0; 3];
        for axis in 0..asked.stencil.axes() {
            if let (Some(start), Some(side), Some(offset), Some(whole)) = (
                data_from.get_mut(axis),
                data_size.get_mut(axis),
                inner_from.get_mut(axis),
                size.get(axis),
            ) {
                *start = (*start + whole - 1) % whole;
                *side += 2;
                *offset = 1;
            }
        }
        (data_from, data_size, inner_from)
    } else {
        (from, inner, [0; 3])
    };
    let points = boxed(data_from, data_size, size);
    let inner_points = boxed(from, inner, size);
    let mut inputs = Vec::with_capacity(asked.reads.len());
    for read in &asked.reads {
        inputs.push(
            server
                .world_at(number, &field)?
                .world
                .cells_at_plane(&points, read.plane)?,
        );
    }
    let mut outputs = Vec::with_capacity(asked.writes.len());
    for write in &asked.writes {
        outputs.push(
            server
                .world_at(output, &out_field)?
                .world
                .cells_at_plane(&inner_points, write.plane)?,
        );
    }
    Ok(Owned {
        size: data_size,
        inputs,
        from: inner_from,
        inner,
        outputs,
    })
}

fn examples_of(server: &mut Server<'_>, asked: &Asked, held: &Json) -> Result<Vec<Owned>, Failed> {
    let listed = held
        .get("examples")
        .and_then(Json::list)
        .ok_or_else(|| unread("examples"))?;
    let mut owned = Vec::with_capacity(listed.len());
    let mut cells = 0_u64;
    for (at, one) in listed.iter().enumerate() {
        let example = match (one.get("world"), one.get("size")) {
            (Some(_), _) => from_world(server, asked, one, at)?,
            (None, Some(_)) => {
                for key in ["output_world", "from", "to"] {
                    if one.get(key).is_some() {
                        return Err(example_refused(
                            at,
                            key,
                            format!("{key} goes with a world example"),
                        ));
                    }
                }
                inline(asked, one, at)?
            }
            (None, None) => {
                return Err(example_refused(
                    at,
                    "",
                    "an example needs world, or size with input and output".to_owned(),
                ));
            }
        };
        cells = cells.saturating_add(cells_of(example.inner));
        if cells > CELLS_MOST {
            return Err(limited(
                Code::LearnLimit,
                &format!("$.examples[{at}]"),
                format!(
                    "the examples up to this one hold {cells} cells, more than the {CELLS_MOST} a call takes"
                ),
                Json::Int(most(CELLS_MOST)),
            ));
        }
        owned.push(example);
    }
    Ok(owned)
}

fn plane_checks(asked: &Asked, channels: u32) -> Result<(), Failed> {
    for (which, read) in asked.reads.iter().enumerate() {
        if read.plane >= channels {
            return Err(limited(
                Code::PlanePast,
                &format!("$.reads[{which}].plane"),
                format!(
                    "read {which} is on plane {}, past the law's {channels} channels",
                    read.plane
                ),
                counted(channels - 1),
            ));
        }
        if let Some(write) = asked.writes.iter().position(|one| one.plane == read.plane) {
            return Err(refused(
                Code::HaloWritten,
                &format!("$.reads[{which}].plane"),
                format!(
                    "read {which} looks at plane {} around the cell, and write {write} writes it; a law may look only at a plane it does not write",
                    read.plane
                ),
            ));
        }
    }
    for (which, write) in asked.writes.iter().enumerate() {
        if write.plane >= channels {
            return Err(limited(
                Code::PlanePast,
                &format!("$.writes[{which}].plane"),
                format!(
                    "write {which} is on plane {}, past the law's {channels} channels",
                    write.plane
                ),
                counted(channels - 1),
            ));
        }
        if asked
            .writes
            .iter()
            .take(which)
            .any(|one| one.plane == write.plane && one.bit == write.bit)
        {
            return Err(refused(
                Code::BitShared,
                &format!("$.writes[{which}]"),
                format!(
                    "write {which} is on bit {} of plane {}, which an earlier write takes",
                    write.bit, write.plane
                ),
            ));
        }
    }
    Ok(())
}

fn seen_shown(one: &stencil::Seen) -> Json {
    map(vec![
        ("outputs", counted(one.outputs)),
        ("example", counted(one.example)),
        ("at", super::tools::sized(one.at)),
    ])
}

fn conflicted(rows: &Rows) -> Failed {
    let listed: Vec<Json> = rows
        .conflicts
        .iter()
        .take(LISTED)
        .map(|(row, seen)| {
            map(vec![
                ("row", counted(*row)),
                ("seen", Json::List(seen.iter().map(seen_shown).collect())),
            ])
        })
        .collect();
    let count = u32::try_from(rows.conflicts.len()).unwrap_or(u32::MAX);
    Box::new(Told(
        Refusal::new(
            Code::Conflict,
            "$.examples",
            format!(
                "{count} neighbourhoods were seen with more than one output, so no law answers every example; rows lists the first {} with each output and where it was first seen",
                listed.len()
            ),
        ),
        vec![("conflicts", counted(count)), ("rows", Json::List(listed))],
    ))
}

fn asymmetric(orbits: &stencil::Orbits, stencil: Stencil) -> Failed {
    let listed: Vec<Json> = orbits
        .broken
        .iter()
        .take(LISTED)
        .map(|(least, rows)| {
            map(vec![
                ("orbit", counted(*least)),
                (
                    "rows",
                    Json::List(
                        rows.iter()
                            .map(|(row, outputs)| {
                                map(vec![("row", counted(*row)), ("outputs", counted(*outputs))])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    let count = u32::try_from(orbits.broken.len()).unwrap_or(u32::MAX);
    Box::new(Told(
        Refusal::new(
            Code::Asymmetric,
            "$.rule",
            format!(
                "{count} sets of seen rows are rotations or reflections of each other under the {} stencil's {} symmetries and were seen with different outputs, so no symmetric law answers every example; orbits lists the first {}, each by its least row, with every seen row in it",
                stencil.name(),
                stencil.symmetries().len(),
                listed.len()
            ),
        ),
        vec![
            ("asymmetric", counted(count)),
            ("orbits", Json::List(listed)),
        ],
    ))
}

fn gates_shown(gates: &[Gate]) -> Json {
    Json::List(
        gates
            .iter()
            .map(|one| {
                map(vec![
                    ("kind", counted(one.kind)),
                    ("left", counted(one.left)),
                    ("right", counted(one.right)),
                ])
            })
            .collect(),
    )
}

struct Rung {
    gates: u32,
    solved: bool,
    marks: u32,
    of: u32,
    rounds: u32,
}

struct Laddered {
    rungs: Vec<Rung>,
    best: Option<(u32, Vec<Gate>)>,
    stop: Option<Stop>,
}

fn ladder(
    host: &crate::host::Host,
    watch: &mut Watch<'_>,
    shape: (u32, u32),
    traces: &[(Vec<u32>, Vec<u32>)],
    top: u32,
    effort: Effort,
) -> Result<Laddered, Failed> {
    let (inputs, outputs) = shape;
    let mut rungs = Vec::new();
    let mut best = None;
    let mut budget = top;
    let mut missed = 0;
    while budget >= outputs.max(1) && missed < PATIENCE && rungs.len() < crate::serve::RUNGS {
        if watch.halted(Duration::ZERO) {
            break;
        }
        let machine = Machine::over(0, inputs, outputs, budget, chip_kinds(), 1)?;
        let ran = crate::serve::pressed_machine_halted(host, machine, traces, effort, watch)?;
        if ran.stopped {
            break;
        }
        let solved = ran.marks >= ran.full;
        rungs.push(Rung {
            gates: budget,
            solved,
            marks: ran.marks,
            of: ran.full,
            rounds: ran.rounds,
        });
        if solved {
            best = Some((budget, ran.gates));
            missed = 0;
        } else {
            missed += 1;
        }
        budget -= 1;
    }
    Ok(Laddered {
        rungs,
        best,
        stop: watch.stop,
    })
}

const fn chip_kinds() -> u32 {
    crate::law::chip::KINDS
}

fn ladder_shown(rungs: &[Rung]) -> Json {
    Json::List(
        rungs
            .iter()
            .map(|rung| {
                map(vec![
                    ("gates", counted(rung.gates)),
                    ("solved", Json::Bool(rung.solved)),
                    ("marks", counted(rung.marks)),
                    ("of", counted(rung.of)),
                    ("rounds", counted(rung.rounds)),
                ])
            })
            .collect(),
    )
}

fn rows_most(inputs: u32, outputs: u32, top: u32) -> u64 {
    let each = u64::from(top) + u64::from(inputs) + u64::from(outputs);
    crate::serve::WORK / each.max(1) * 32
}

fn crowd_fitting(
    host: &crate::host::Host,
    inputs: u32,
    top: u32,
    traces: usize,
) -> Result<u32, Failed> {
    let wide = u64::try_from(traces)
        .unwrap_or(u64::MAX)
        .div_ceil(32)
        .max(1);
    let words = faults::ceiling(host) / 4;
    let each = u64::from(inputs + top).saturating_mul(wide).max(1);
    let width = u64::from(host.width().max(1));
    let fits = (words / each / width).saturating_mul(width);
    let least = width.max(64);
    if fits < least {
        return Err(limited(
            Code::LearnLimit,
            "$.examples",
            format!(
                "{traces} rows searched with {top} gates and {inputs} inputs need {each} words a candidate in one binding, and this device's binding holds {words} words, so not even a crowd of {least} fits; give fewer rows or gates"
            ),
            Json::Int(i64::try_from(words / each).unwrap_or(i64::MAX)),
        ));
    }
    Ok(u32::try_from(fits.min(u64::from(crate::serve::CROWD))).unwrap_or(u32::MAX))
}

fn kernel_words(
    host: &crate::host::Host,
    shape: (u32, u32),
    gates: u32,
    wide: u32,
) -> Result<usize, Failed> {
    let machine = Machine::over(0, shape.0, shape.1, gates, chip_kinds(), 1)?;
    Ok(crate::marking::stepped_rolled(host.width(), machine, wide)?.len())
}

struct Built {
    gates: u32,
    words: usize,
    took: Duration,
}

fn predicted(evidence: &[Built], words: usize) -> Duration {
    evidence
        .iter()
        .map(|one| {
            let ratio = (words as f64 / one.words.max(1) as f64).max(1.0);
            one.took.mul_f64(ratio * ratio)
        })
        .min()
        .unwrap_or(Duration::MAX)
}

fn calibrated(
    host: &crate::host::Host,
    shape: (u32, u32),
    gates: u32,
    wide: u32,
) -> Result<(Built, Duration), Failed> {
    let machine = Machine::over(0, shape.0, shape.1, gates, chip_kinds(), 1)?;
    let module = crate::marking::stepped_rolled(host.width(), machine, wide)?;
    let modules: Vec<&[u32]> = vec![&module];
    let sizes = vec![64_usize; crate::marking::BINDINGS as usize];
    let began = std::time::Instant::now();
    let resident = host.resident(&sizes, &modules)?;
    let took = began
        .elapsed()
        .saturating_sub(resident.waited())
        .saturating_add(faults::calibrating(module.len()));
    Ok((
        Built {
            gates,
            words: module.len(),
            took,
        },
        resident.patience(),
    ))
}

fn kernel_fitting(
    host: &crate::host::Host,
    watch: &mut Watch<'_>,
    shape: (u32, u32),
    top: u32,
    traces: usize,
) -> Result<(), Failed> {
    let wide = u32::try_from(traces.div_ceil(32).max(1)).unwrap_or(u32::MAX);
    let least = shape.1.max(1);
    let (first, patience) = calibrated(host, shape, least, wide)?;
    let most = patience / MARGIN;
    let mut evidence = vec![first];
    let top_words = kernel_words(host, shape, top, wide)?;
    loop {
        let fits = |gates: u32, evidence: &[Built]| -> Result<bool, Failed> {
            Ok(predicted(evidence, kernel_words(host, shape, gates, wide)?) <= most)
        };
        if fits(top, &evidence)? {
            return Ok(());
        }
        let (mut low, mut high) = (least, top);
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            if fits(mid, &evidence)? {
                low = mid;
            } else {
                high = mid;
            }
        }
        let tried = evidence.iter().map(|one| one.gates).max().unwrap_or(least);
        let admitted = if fits(low, &evidence)? { low } else { 0 };
        if admitted <= tried {
            let best = evidence
                .iter()
                .min_by_key(|one| predicted(std::slice::from_ref(*one), top_words))
                .map_or((0, Duration::ZERO), |one| (one.words, one.took));
            return Err(limited(
                Code::LearnLimit,
                "$.gates",
                format!(
                    "a search kernel of {top} gates over {} inputs and {} outputs is {top_words} words; this device built {} words in {} ms, and with a build growing at most with the square of its words that is about {} ms, past half the {} ms a submission may take on this device; at most {} gates fit, and nothing was searched",
                    shape.0,
                    shape.1,
                    best.0,
                    best.1.as_millis(),
                    predicted(&evidence, top_words).as_millis(),
                    patience.as_millis(),
                    admitted
                ),
                counted(admitted),
            ));
        }
        if watch.over(predicted(&evidence, kernel_words(host, shape, low, wide)?)) {
            return Err(halted_refusal(
                watch.stop.unwrap_or(Stop::Budget),
                "while measuring how fast this device builds a search kernel",
            ));
        }
        evidence.push(calibrated(host, shape, low, wide)?.0);
    }
}

fn fits_kernel(inputs: u32, outputs: u32, top: u32, traces: usize) -> Result<(), Failed> {
    let fits = rows_most(inputs, outputs, top);
    if u64::try_from(traces).unwrap_or(u64::MAX) <= fits {
        return Ok(());
    }
    Err(limited(
        Code::LearnLimit,
        "$.examples",
        format!(
            "{traces} rows to search over with {top} gates, {inputs} inputs and {outputs} outputs pass the {} operations one kernel is allowed; at most {fits} rows fit",
            crate::serve::WORK
        ),
        Json::Int(i64::try_from(fits).unwrap_or(i64::MAX)),
    ))
}

fn wiring(asked: &Asked, held: &Json, channels: u32) -> Wiring {
    let positions = asked.stencil.positions();
    let reads = asked
        .reads
        .iter()
        .flat_map(|read| {
            (0..positions).map(move |place| Read {
                plane: read.plane,
                at: asked.stencil.offset(place),
                bit: read.bit,
            })
        })
        .collect();
    Wiring {
        name: held
            .get("name")
            .and_then(Json::text)
            .unwrap_or("learned")
            .to_owned(),
        channels,
        reads,
        writes: asked
            .writes
            .iter()
            .map(|one| Spot {
                plane: one.plane,
                bit: one.bit,
            })
            .collect(),
        keeps: Vec::new(),
        rest: match held.get("rest").and_then(Json::text) {
            Some("cleared") => Rest::Cleared,
            _ => Rest::Kept,
        },
    }
}

fn halted_refusal(stop: Stop, when: &str) -> Failed {
    refused(
        stop.code(),
        if stop == Stop::Budget {
            "$.budget_ms"
        } else {
            "$"
        },
        format!(
            "the call stopped on {} {when}; nothing was learned",
            stop.name()
        ),
    )
}

fn exact(held: u64) -> Json {
    Json::Int(i64::try_from(held).unwrap_or(i64::MAX))
}

fn learned(server: &mut Server<'_>, held: &Json) -> Result<Json, Failed> {
    let stencil = match held.get("stencil").and_then(Json::text) {
        Some("3x3x3") => Stencil::Cube,
        Some("3x3") => Stencil::Square,
        _ => return Err(unread("stencil")),
    };
    let asked = Asked {
        stencil,
        reads: bits_of(held, "reads")?,
        writes: bits_of(held, "writes")?,
        border: match held.get("border").and_then(Json::text) {
            Some("wrapped") => Border::Wrapped,
            Some("zero") => Border::Zero,
            _ => return Err(unread("border")),
        },
    };
    let symmetric = match held.get("rule").and_then(Json::text) {
        Some("symmetric") => true,
        Some("minimal") => false,
        _ => return Err(unread("rule")),
    };
    let reads = u32::try_from(asked.reads.len()).unwrap_or(u32::MAX);
    let outputs = u32::try_from(asked.writes.len()).unwrap_or(u32::MAX);
    let inputs = stencil.positions().saturating_mul(reads);
    if inputs > INPUTS_MOST {
        return Err(limited(
            Code::LearnLimit,
            "$.reads",
            format!(
                "{reads} reads at the {} places of the {} stencil make {inputs} inputs, more than the {INPUTS_MOST} a learned law takes",
                stencil.positions(),
                stencil.name()
            ),
            counted(INPUTS_MOST / stencil.positions()),
        ));
    }
    let highest = asked
        .reads
        .iter()
        .chain(&asked.writes)
        .map(|one| one.plane)
        .max()
        .unwrap_or(0);
    let channels = match held.get("channels") {
        None => highest + 1,
        Some(_) => int(held, "channels")?,
    };
    plane_checks(&asked, channels)?;
    let asked_top = int(held, "gates")?;
    let copies = if symmetric {
        u32::try_from(stencil.symmetries().len()).unwrap_or(u32::MAX)
    } else {
        1
    };
    let fits_law = if symmetric {
        crate::law::chip::GATES_SURE.saturating_sub(outputs.saturating_mul(copies - 1)) / copies
    } else {
        asked_top
    };
    if fits_law < outputs {
        return Err(limited(
            Code::LearnLimit,
            "$.writes",
            format!(
                "a symmetric law is {copies} copies of the circuit joined by ors, and with {outputs} outputs not even one gate an output fits the {} gates a law always holds",
                crate::law::chip::GATES_SURE
            ),
            counted(0),
        ));
    }
    let top = asked_top.min(fits_law);
    if top < outputs {
        return Err(limited(
            Code::LearnLimit,
            "$.gates",
            format!("gates is {top}, and each of the {outputs} outputs is a gate"),
            counted(outputs),
        ));
    }
    let effort = Effort {
        crowd: int(held, "crowd")?,
        rounds: int(held, "rounds")?,
        seed: int(held, "seed")?,
        ..Effort::of(CROWD, ROUNDS)
    };
    let host = server.host.ok_or_else(|| {
        refused(
            Code::NoComputeDevice,
            "$",
            "the gate search runs on a compute device, and this server has none".to_owned(),
        )
    })?;
    let owned = examples_of(server, &asked, held)?;
    let samples: Vec<Sample<'_>> = owned.iter().map(Owned::sample).collect();
    let rows = stencil::rows_of(stencil, &asked.reads, &asked.writes, &samples, asked.border);
    if !rows.conflicts.is_empty() {
        return Err(conflicted(&rows));
    }
    let moves: Vec<Vec<u32>> = stencil
        .symmetries()
        .iter()
        .map(|places| stencil::inputs_moved(stencil, reads, places))
        .collect();
    fits_kernel(inputs, outputs, top, rows.seen.len())?;
    let most = usize::try_from(rows_most(inputs, outputs, top)).unwrap_or(usize::MAX);
    let mut watch = Watch {
        guard: server.guard(),
        host: server.host,
        stop: None,
        began: None,
        built: None,
        waited: Duration::ZERO,
        held_up: Duration::ZERO,
        crowd_most: u32::MAX,
        #[cfg(feature = "faults")]
        faults: server.faults(),
    };
    let mut stopped: Option<Stop> = None;
    let wanted: std::collections::BTreeMap<u32, u32> = if symmetric {
        let mut halted = || {
            let _ = watch.charged();
            stopped = watch.guard.stop();
            stopped.is_some()
        };
        let orbits = match stencil::orbits(&rows.seen, &moves, most, &mut halted) {
            Ok(orbits) => orbits,
            Err(stencil::Cut::Rows(count)) => {
                return Err(limited(
                    Code::LearnLimit,
                    "$.examples",
                    format!(
                        "the images of the {} seen rows under the {} stencil's {copies} symmetries pass {count} rows, more than the {most} one search kernel holds with {top} gates, {inputs} inputs and {outputs} outputs; nothing was searched",
                        rows.seen.len(),
                        stencil.name()
                    ),
                    Json::Int(i64::try_from(most).unwrap_or(i64::MAX)),
                ));
            }
            Err(stencil::Cut::Halted) => {
                return Err(halted_refusal(
                    stopped.unwrap_or(Stop::Budget),
                    "while expanding the seen rows under the symmetries",
                ));
            }
        };
        if !orbits.broken.is_empty() {
            return Err(asymmetric(&orbits, stencil));
        }
        orbits.expanded
    } else {
        rows.seen
            .iter()
            .map(|(row, one)| (*row, one.outputs))
            .collect()
    };
    let traces: Vec<(Vec<u32>, Vec<u32>)> = wanted
        .iter()
        .map(|(row, out)| (vec![*row], vec![*out]))
        .collect();
    fits_kernel(inputs, outputs, top, traces.len())?;
    let crowd_most = crowd_fitting(host, inputs, top, traces.len())?;
    kernel_fitting(host, &mut watch, (inputs, outputs), top, traces.len())?;
    watch.crowd_most = crowd_most;
    let asked_crowd = effort.crowd;
    let effort = Effort {
        crowd: effort.crowd.min(crowd_most),
        ..effort
    };
    let laddered = ladder(host, &mut watch, (inputs, outputs), &traces, top, effort)?;
    let Some((found, gates)) = laddered.best else {
        if let Some(stop) = laddered.stop {
            return Err(refused(
                stop.code(),
                if stop == Stop::Budget {
                    "$.budget_ms"
                } else {
                    "$"
                },
                format!(
                    "the gate ladder stopped on {} after {} rungs, before any circuit answered every seen row; nothing was learned",
                    stop.name(),
                    laddered.rungs.len()
                ),
            ));
        }
        let marks = laddered
            .rungs
            .iter()
            .map(|rung| rung.marks)
            .max()
            .unwrap_or(0);
        let of = laddered.rungs.first().map_or(0, |rung| rung.of);
        return Err(Box::new(Told(
            Refusal::new(
                Code::Unsolved,
                "$.gates",
                format!(
                    "no circuit of at most {top} gates answering every one of the {} rows was found within these rounds and crowd; the best scored {marks} of {of} marks",
                    traces.len()
                ),
            ),
            vec![("ladder", ladder_shown(&laddered.rungs))],
        )));
    };
    let kept = stencil::live(inputs, outputs, &gates);
    let law_gates = if symmetric {
        stencil::symmetrised(inputs, outputs, &kept, &moves)
    } else {
        kept.clone()
    };
    for (row, out) in &wanted {
        if stencil::answer(inputs, outputs, &law_gates, *row) != *out {
            return Err(format!(
                "the learned circuit answers row {row} other than the examples; this is a fault of the server"
            )
            .into());
        }
    }
    let circuit = Circuit {
        state: 0,
        inputs,
        outputs,
        gates: law_gates.clone(),
    };
    let mut answer = super::law::chip_law(circuit, wiring(&asked, held, channels))?;
    let total = 1_u64 << inputs;
    let seen = u64::try_from(rows.seen.len()).unwrap_or(u64::MAX);
    let unseen = total - seen;
    let mut counting_stop: Option<Stop> = None;
    let guard = server.guard();
    let already = laddered.stop.is_some();
    let mut halted = || {
        if already {
            return false;
        }
        counting_stop = guard.stop();
        counting_stop.is_some()
    };
    let Some(count) = stencil::ones(inputs, outputs, &law_gates, &mut halted) else {
        return Err(halted_refusal(
            counting_stop.unwrap_or(Stop::Budget),
            "while counting the decided rows, after the law was found",
        ));
    };
    let mut ones = Vec::with_capacity(outputs as usize);
    let mut zeros = Vec::with_capacity(outputs as usize);
    for output in 0..outputs {
        let all = count.get(output as usize).copied().unwrap_or(0);
        let on_seen = rows
            .seen
            .values()
            .filter(|one| (one.outputs >> output) & 1 == 1)
            .count() as u64;
        let decided = all.saturating_sub(on_seen);
        ones.push(exact(decided));
        zeros.push(exact(unseen.saturating_sub(decided)));
    }
    let mut decided = vec![
        ("rows", exact(unseen)),
        (
            "rule",
            text(if symmetric { "symmetric" } else { "minimal" }),
        ),
        ("means", text(if symmetric { SYMMETRIC } else { MINIMAL })),
        ("zeros", Json::List(zeros)),
        ("ones", Json::List(ones)),
    ];
    if symmetric {
        decided.push((
            "by_symmetry",
            exact(
                u64::try_from(wanted.len())
                    .unwrap_or(u64::MAX)
                    .saturating_sub(seen),
            ),
        ));
    }
    if inputs <= TABLE_INPUTS {
        let table = stencil::table(inputs, outputs, &law_gates);
        decided.push((
            "table",
            Json::List(table.iter().map(|one| counted(*one)).collect()),
        ));
        decided.push((
            "unseen_rows",
            Json::List(
                (0..1_u32 << inputs)
                    .filter(|row| !rows.seen.contains_key(row))
                    .map(counted)
                    .collect(),
            ),
        ));
    }
    answer.extend([
        ("stencil", text(stencil.name())),
        ("row_order", text(stencil::ORDER)),
        (
            "border",
            text(if asked.border == Border::Zero {
                "zero"
            } else {
                "wrapped"
            }),
        ),
        (
            "runs_as",
            text(if asked.border == Border::Zero {
                ZERO_RUNS
            } else {
                WRAPPED_RUNS
            }),
        ),
        (
            "examples",
            map(vec![
                ("cells", exact(rows.cells)),
                ("rows", exact(total)),
                ("seen", exact(seen)),
                ("conflicts", Json::Int(0)),
            ]),
        ),
        (
            "found",
            map(vec![
                ("gates", counted(found)),
                (
                    "live",
                    counted(u32::try_from(kept.len()).unwrap_or(u32::MAX)),
                ),
                ("chip", gates_shown(&kept)),
            ]),
        ),
        (
            "chip",
            map(vec![
                ("inputs", counted(inputs)),
                ("outputs", counted(outputs)),
                ("gates", gates_shown(&law_gates)),
            ]),
        ),
        ("unseen", map(decided)),
        ("ladder", ladder_shown(&laddered.rungs)),
        ("stopped", text(laddered.stop.map_or("ladder", Stop::name))),
    ]);
    if effort.crowd < asked_crowd {
        answer.push((
            "crowd_capped",
            map(vec![
                ("asked", counted(asked_crowd)),
                ("used", counted(effort.crowd)),
                (
                    "why",
                    text(
                        "a candidate holds the inputs plus the gates times the rows over 32 words in one binding, and this device's binding holds no larger crowd; it is not doubled past this",
                    ),
                ),
            ]),
        ));
    }
    if top < asked_top {
        answer.push((
            "gates_capped",
            map(vec![
                ("asked", counted(asked_top)),
                ("used", counted(top)),
                (
                    "why",
                    text(&format!(
                        "a symmetric law is {copies} copies of the circuit and {} or gates, and a law always holds {} gates, so the ladder starts at {top}",
                        outputs.saturating_mul(copies - 1),
                        crate::law::chip::GATES_SURE
                    )),
                ),
            ]),
        ));
    }
    if let Some(stop) = laddered.stop {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}
