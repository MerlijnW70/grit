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

pub const MINIMAL: &str = "each row the examples never showed takes the answer of the smallest circuit the gate ladder found that answers every seen row";

pub const SYMMETRIC: &str = "the ladder finds the smallest circuit f that answers every seen row and every rotation and reflection of one (8 for 3x3, 48 for 3x3x3); the law answers output j 1 exactly when f answers it 1 on some rotation or reflection of the neighbourhood, so every row the examples never showed is decided alike with its images";

pub const ZERO_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; to read 0 past an example's edge, write it at the origin of a world at least one cell longer than it along each axis the stencil reads, every other cell of the read planes 0";

pub const WRAPPED_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; an inline example wraps on itself, so it runs alike on a world of exactly its size, and a world example's box read its neighbours from that world";

pub const LEARN: &str = "grit_law_learn builds the stencil's rows from example worlds, refuses a row seen with two outputs (conflict) and, under rule symmetric, rows whose rotations or reflections disagree (asymmetric), then walks a gate ladder down from gates as grit_least does, with grit_machine's search over the seen rows: each rung doubles its crowd up to 65536 while unsolved, and the ladder ends after two failed rungs in a row. The law is the smallest solved circuit, wired as grit_law_from_chip would wire it, with the row order below. A search checks its budget and cancel before each 50 rounds of a rung and stops when the last 50 rounds it measured, or the device's first measured round times 50, do not fit the time left, and before each search's build when the last build it measured in this call does not fit; the first build cannot stop part way, and its kernel is unrolled over the rows searched (under symmetric every image of every seen row), so it grows with them and an answer can come late by up to one build; a stopped ladder answers the smallest circuit it solved so far, with stopped budget or cancelled, and refuses budget or cancelled when none. Counting the decided rows cannot stop part way, so an answer can come late by up to two to the inputs rows times the law's gates of host work. The same request gives the same answer on the same device";

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
        false
    }

    fn halted(&mut self, next: Duration) -> bool {
        if let Some(began) = self.began.take() {
            self.built = Some(began.elapsed());
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
            "one word a cell, x fastest, then y, then z",
            1,
            most(CELLS_MOST),
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
                    "the crowd each rung starts with, doubled up to 65536 while unsolved",
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
            Code::TooMany,
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
                Code::TooMany,
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

fn fits_kernel(inputs: u32, outputs: u32, top: u32, traces: usize) -> Result<(), Failed> {
    let each = u64::from(top) + u64::from(inputs) + u64::from(outputs);
    let wide = u64::try_from(traces)
        .unwrap_or(u64::MAX)
        .div_ceil(32)
        .max(1);
    if each.saturating_mul(wide) <= crate::serve::WORK {
        return Ok(());
    }
    let fits = crate::serve::WORK / each * 32;
    Err(limited(
        Code::TooMany,
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
            Code::TooMany,
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
    let top = int(held, "gates")?;
    if top < outputs {
        return Err(limited(
            Code::TooMany,
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
    let wanted: std::collections::BTreeMap<u32, u32> = if symmetric {
        let orbits = stencil::orbits(&rows.seen, &moves);
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
    let mut watch = Watch {
        guard: server.guard(),
        host: server.host,
        stop: None,
        began: None,
        built: None,
        #[cfg(feature = "faults")]
        faults: server.faults(),
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
    let count = stencil::ones(inputs, outputs, &law_gates);
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
    if let Some(stop) = laddered.stop {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}
