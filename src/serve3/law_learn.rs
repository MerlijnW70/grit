use std::time::Duration;

use crate::law::chip::{Circuit, Read, Rest, Spot, Wiring};
use crate::machine::Machine;
use crate::netlist::{Gate, Netlist};
use crate::serve::{Effort, Halt};
use crate::stencil::{self, Bit, Border, Rows, Sample, Stencil, Tallied, Tally};
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

pub const TOTALISTIC: &str = "each write's output is T[n], n the number of the stencil's places, centre included, whose read bit is 1 (0 to 9 for 3x3, 0 to 27 for 3x3x3); T is read from the seen rows in one pass with no search, and a count the examples never showed is 0";

pub const OUTER: &str = "each write's output is T[c][n], c the centre's read bit and n the number of the other places whose read bit is 1 (0 to 8 for 3x3, 0 to 26 for 3x3x3); T is read from the seen rows in one pass with no search, and a count the examples never showed is 0";

pub const NO_SEARCH: &str = "no_search";

pub const STARTED: &str = "the start's law, its live gates under minimal and the or of their images under symmetric, was answered on the host on every seen row and, under symmetric, on every image of one";

pub const CHANGED_EVERY: &str = "every row";

pub const CHANGED_SEEN: &str = "the seen rows";

pub const ZERO_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; to read 0 past an example's edge, write it at the origin of a world at least one cell longer than it along each axis the stencil reads, every other cell of the read planes 0";

pub const WRAPPED_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; an inline example wraps on itself, so it runs alike on a world whose sides equal its own along each axis the stencil reads (3x3 reads x and y, so its tall is free; a world's tall is at least 2), and a world example's box reads its neighbours from that world";

pub const LEARN: &str = "grit_law_learn builds the stencil's rows from example worlds, refuses a row seen with two outputs (conflict) and, under rule symmetric, rows whose rotations or reflections disagree (asymmetric), then walks a gate ladder down from gates as grit_least does, with grit_machine's search over the seen rows: each rung starts its crowd at most at what one device binding holds and doubles it while unsolved up to 65536 or that limit, whichever is less; the answer names a lowered crowd as crowd_capped. The ladder ends after two failed rungs in a row. The law is the smallest solved circuit, wired as grit_law_from_chip would wire it, with the row order below. A call stops on its deadline or cancel only at a brake: before each build that measures the device, before each search's build and before each device submission of a look; no measured time is compared with the time left. Once the deadline has passed no further submission starts, so an answer can come late by the one submission in flight: a build, or one submission of a look, sized by the device's measured pace. A build cannot stop part way, and its kernel loops over the rows searched (under symmetric every image of every seen row), so it does not grow with them; before the ladder the call builds the kernel of its smallest rung, then larger ones while each build's first submission, where a driver that compiles at its first dispatch compiles, grown with the square of the kernel's words from the closest one measured, stays within half the device's patience; a top rung past that is refused learn_limit at gates with the most gates that fit, a limit measured on this device at this moment and so outside the same answer below; a stopped ladder answers the smallest circuit it solved so far, with stopped budget or cancelled, and refuses budget or cancelled when none. The seen rows, and under symmetric their images, are refused learn_limit before any search when they pass the rows one kernel holds; the expansion stops as soon as it passes them, so it never holds more. The expansion checks budget and cancel every 4096 rows and refuses budget or cancelled. The count of decided rows checks only cancel, every 4096 blocks of 64 rows, and none after a stopped ladder; otherwise it runs to its end, at most two to the inputs less 6 blocks times the inputs plus the law's gates, so a found law is never lost to the deadline. Under symmetric the ladder starts at most at the gates whose copies and ors fit the 600 gates a law always holds, and the answer names a lowered start as gates_capped. The same request gives the same answer on the same device when it answers stopped ladder; a stop on budget or cancel depends on when the deadline passed or the cancel came, and its ladder is a prefix of that answer's ladder, with the smallest circuit solved in it. Under rule totalistic or outer there is no search, ladder, brake or search kernel, and inline examples need no compute device: each write's table is read from the seen rows in one pass, a count never seen is 0, and the law counts with a gate circuit, an adder tree and a decoder of the count. Under rule minimal or symmetric, start is a circuit to begin from, the chip of an earlier answer; it is checked on the host on every seen row before any search: one that answers them all is a solved rung, so the answer is never larger than it and never refused budget or cancelled, and the ladder walks down from its live gates less one; one that misses rows is a member of the first rung's crowd, kept unless a candidate scores at least as well, and the ladder starts at its live gates when gates is fewer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    Calibration,
    Build,
    Submission,
    Submitted,
    Expansion,
    Count,
}

impl Point {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Calibration => "calibration",
            Self::Build => "build",
            Self::Submission => "submission",
            Self::Submitted => "submitted",
            Self::Expansion => "expansion",
            Self::Count => "count",
        }
    }

    #[cfg(feature = "faults")]
    const fn at(self) -> usize {
        match self {
            Self::Calibration => 0,
            Self::Build => 1,
            Self::Submission => 2,
            Self::Submitted => 3,
            Self::Expansion => 4,
            Self::Count => 5,
        }
    }
}

pub mod faults {
    #[cfg(feature = "faults")]
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    #[cfg(feature = "faults")]
    use crate::clock::Fake;
    use crate::host::Host;

    use super::Point;

    #[cfg(feature = "faults")]
    type Passed = Option<(Point, u64, Arc<Fake>, u64)>;

    #[cfg(feature = "faults")]
    thread_local! {
        static CHECKS: Cell<u64> = const { Cell::new(0) };
        static CANCEL: Cell<Option<u64>> = const { Cell::new(None) };
        static KERNEL: Cell<usize> = const { Cell::new(0) };
        static BUILT: Cell<u64> = const { Cell::new(0) };
        static SUBMITTED: Cell<u64> = const { Cell::new(0) };
        static REACHED: Cell<[u64; 6]> = const { Cell::new([0; 6]) };
        static PASSED: RefCell<Passed> = const { RefCell::new(None) };
        static CEILING: Cell<Option<u64>> = const { Cell::new(None) };
        static SLOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        static SLOWED: Cell<Option<(u64, Duration)>> = const { Cell::new(None) };
        static CALIBRATED: Cell<u64> = const { Cell::new(0) };
        static EACH: Cell<Option<u32>> = const { Cell::new(None) };
    }

    #[cfg(feature = "faults")]
    pub fn builds_slowed_by(each_word: Duration) {
        SLOW.set(each_word);
    }

    #[cfg(feature = "faults")]
    pub fn calibration_slowed(at: u64, by: Duration) {
        SLOWED.set(Some((at, by)));
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn calibrations() -> u64 {
        CALIBRATED.get()
    }

    #[cfg(feature = "faults")]
    pub(super) fn calibrating(words: usize, measured: Duration) -> Duration {
        let at = CALIBRATED.get();
        CALIBRATED.set(at.saturating_add(1));
        let each = SLOW.get();
        let took = if each.is_zero() {
            measured
        } else {
            each.saturating_mul(u32::try_from(words).unwrap_or(u32::MAX))
        };
        match SLOWED.get() {
            Some((slowed, by)) if slowed == at => took.saturating_add(by),
            _ => took,
        }
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn calibrating(_: usize, measured: Duration) -> Duration {
        measured
    }

    #[cfg(feature = "faults")]
    pub fn rounds_each_submission(rounds: u32) {
        EACH.set(Some(rounds));
    }

    #[cfg(feature = "faults")]
    pub(super) fn submitted_most(asked: u32) -> u32 {
        EACH.get().unwrap_or(asked)
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn submitted_most(asked: u32) -> u32 {
        asked
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
    #[must_use]
    pub fn builds() -> u64 {
        BUILT.get()
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn submissions() -> u64 {
        SUBMITTED.get()
    }

    #[cfg(feature = "faults")]
    #[must_use]
    pub fn reached(point: Point) -> u64 {
        REACHED.get().get(point.at()).copied().unwrap_or(0)
    }

    #[cfg(feature = "faults")]
    pub(super) fn kernel(words: usize) {
        KERNEL.set(KERNEL.get().max(words));
        BUILT.set(BUILT.get().saturating_add(1));
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn kernel(_: usize) {}

    #[cfg(feature = "faults")]
    pub(super) fn submitted() {
        SUBMITTED.set(SUBMITTED.get().saturating_add(1));
        passing(Point::Submitted);
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn submitted() {}

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
    pub fn deadline_passed_at(point: Point, ordinal: u64, clock: Arc<Fake>, ms: u64) {
        PASSED.set(Some((point, ordinal, clock, ms)));
    }

    #[cfg(feature = "faults")]
    pub fn disarm() {
        CANCEL.set(None);
        PASSED.set(None);
        CEILING.set(None);
        SLOW.set(Duration::ZERO);
        SLOWED.set(None);
        EACH.set(None);
        CALIBRATED.set(0);
        KERNEL.set(0);
        BUILT.set(0);
        SUBMITTED.set(0);
        REACHED.set([0; 6]);
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
    fn passing(point: Point) {
        let mut reached = REACHED.get();
        let ordinal = reached.get(point.at()).copied().unwrap_or(0);
        if let Some(one) = reached.get_mut(point.at()) {
            *one = ordinal.saturating_add(1);
        }
        REACHED.set(reached);
        PASSED.with_borrow_mut(|held| {
            if let Some((at, when, clock, ms)) = held.as_ref()
                && *at == point
                && *when == ordinal
            {
                clock.advance(*ms);
                *held = None;
            }
        });
    }

    #[cfg(feature = "faults")]
    pub(super) fn checked(
        point: Point,
        host: Option<&Host>,
        cancel: Option<&Arc<AtomicBool>>,
        faults: &std::sync::Mutex<super::super::Faults>,
    ) {
        CHECKS.set(CHECKS.get().saturating_add(1));
        passing(point);
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
    }

    #[cfg(not(feature = "faults"))]
    pub(super) const fn checked(_: Point, _: Option<&Host>, _: Option<&Arc<AtomicBool>>) {}
}

struct Watch<'host> {
    guard: Guard,
    host: Option<&'host crate::host::Host>,
    stop: Option<Stop>,
    crowd_most: u32,
    #[cfg(feature = "faults")]
    faults: std::sync::Arc<std::sync::Mutex<super::Faults>>,
}

impl Watch<'_> {
    #[cfg(feature = "faults")]
    fn checked(&self, point: Point) {
        faults::checked(point, self.host, self.guard.cancel.as_ref(), &self.faults);
    }

    #[cfg(not(feature = "faults"))]
    fn checked(&self, point: Point) {
        faults::checked(point, self.host, self.guard.cancel.as_ref());
    }

    fn braked(&mut self, point: Point) -> bool {
        self.checked(point);
        if let Some(stop) = self.guard.stop() {
            self.stop = Some(stop);
            return true;
        }
        false
    }

    fn cancelled(&self, point: Point) -> bool {
        self.checked(point);
        self.guard.stop() == Some(Stop::Cancelled)
    }
}

impl Halt for Watch<'_> {
    fn halted(&mut self) -> bool {
        self.braked(Point::Submission)
    }

    fn submitted(&mut self) {
        faults::submitted();
    }

    fn building(&mut self) -> bool {
        self.braked(Point::Build)
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
                    "how rows the examples never showed are decided: minimal, the smallest circuit found; symmetric, alike under the stencil's rotations and reflections; totalistic, by the count of places whose read bit is 1; outer, by the centre's bit and the count of the other places. totalistic and outer take one read, read each write's table from the seen rows in one pass with no search, and decide a count never seen 0",
                    &["minimal", "symmetric", "totalistic", "outer"],
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
            ("start", start_shown()),
        ],
        &["stencil", "reads", "writes", "border", "rule", "examples"],
    )
}

fn start_shown() -> Json {
    let gate = object(
        vec![
            (
                "kind",
                with(
                    word(),
                    vec![(
                        "description",
                        text("which of the 16 two-input functions, as its truth table"),
                    )],
                ),
            ),
            (
                "left",
                with(
                    word(),
                    vec![(
                        "description",
                        text("the source it reads first: the inputs, then the gates before it"),
                    )],
                ),
            ),
            (
                "right",
                with(
                    word(),
                    vec![("description", text("the source it reads second"))],
                ),
            ),
        ],
        &["kind", "left", "right"],
    );
    let mut shown = object(
        vec![
            (
                "inputs",
                with(
                    word(),
                    vec![(
                        "description",
                        text("the stencil's places times the reads, in the row order"),
                    )],
                ),
            ),
            (
                "outputs",
                with(
                    word(),
                    vec![("description", text("one for each write, in order"))],
                ),
            ),
            (
                "state",
                with(
                    word(),
                    vec![
                        (
                            "description",
                            text("a learned law holds none; 0 if left out"),
                        ),
                        ("default", Json::Int(0)),
                    ],
                ),
            ),
            (
                "gates",
                list_shown(
                    gate,
                    "the circuit's gates as grit_law_learn and grit_law_from_chip list them; the last outputs gates are the outputs",
                    1,
                    i64::from(crate::netlist::SOURCES),
                ),
            ),
        ],
        &["inputs", "outputs", "gates"],
    );
    if let Json::Map(fields) = &mut shown {
        fields.push((
            "description".to_owned(),
            text(
                "rule minimal or symmetric: a circuit to begin from, the chip of an earlier answer with the same stencil, reads and writes; under symmetric its law is the or of its images, so pass the found chip of a symmetric answer. It is checked on the host on every seen row: one that answers them all is a solved rung and the ladder walks down from its live gates less one; one that misses rows is a member of the first rung's crowd, which starts at its live gates when gates is fewer. The answer's start names its live gates, the seen rows it answers wrongly and the rows the answer decides otherwise",
            ),
        ));
    }
    shown
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

type Trace = (Vec<u32>, Vec<u32>);

struct Laddered {
    rungs: Vec<Rung>,
    best: Option<(u32, Vec<Gate>)>,
    stop: Option<Stop>,
}

fn ladder(
    host: &crate::host::Host,
    watch: &mut Watch<'_>,
    shape: (u32, u32),
    (traces, sown): (&[Trace], &[Vec<u32>]),
    top: u32,
    effort: Effort,
) -> Result<Laddered, Failed> {
    let (inputs, outputs) = shape;
    let mut rungs = Vec::new();
    let mut best = None;
    let mut budget = top;
    let mut missed = 0;
    while budget >= outputs.max(1) && missed < PATIENCE && rungs.len() < crate::serve::RUNGS {
        let machine = Machine::over(0, inputs, outputs, budget, chip_kinds(), 1)?;
        let planted: &[Vec<u32>] = if budget == top { sown } else { &[] };
        let ran =
            crate::serve::pressed_machine_sown(host, machine, traces, effort, watch, planted)?;
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
    let resident = host.resident(&sizes, &modules)?;
    let took = faults::calibrating(module.len(), resident.warmed());
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
    (top, field): (u32, &str),
    traces: usize,
) -> Result<Option<Stop>, Failed> {
    let wide = u32::try_from(traces.div_ceil(32).max(1)).unwrap_or(u32::MAX);
    let least = shape.1.max(1);
    if let Some(stop) = calibration_halted(watch) {
        return Ok(Some(stop));
    }
    let (first, patience) = calibrated(host, shape, least, wide)?;
    let most = patience / MARGIN;
    let mut evidence = vec![first];
    let top_words = kernel_words(host, shape, top, wide)?;
    loop {
        let fits = |gates: u32, evidence: &[Built]| -> Result<bool, Failed> {
            Ok(predicted(evidence, kernel_words(host, shape, gates, wide)?) <= most)
        };
        if fits(top, &evidence)? {
            return Ok(None);
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
                field,
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
        if let Some(stop) = calibration_halted(watch) {
            return Ok(Some(stop));
        }
        evidence.push(calibrated(host, shape, low, wide)?.0);
    }
}

fn calibration_halted(watch: &mut Watch<'_>) -> Option<Stop> {
    watch
        .braked(Point::Calibration)
        .then(|| watch.stop.unwrap_or(Stop::Budget))
}

const CALIBRATING: &str = "while measuring how fast this device builds a search kernel";

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

fn channels_of(held: &Json, asked: &Asked) -> Result<u32, Failed> {
    let highest = asked
        .reads
        .iter()
        .chain(&asked.writes)
        .map(|one| one.plane)
        .max()
        .unwrap_or(0);
    match held.get("channels") {
        None => Ok(highest + 1),
        Some(_) => int(held, "channels"),
    }
}

fn split_shown(output: u32, (row, seen): (u32, stencil::Seen)) -> Json {
    map(vec![
        ("output", counted(output)),
        ("row", counted(row)),
        ("example", counted(seen.example)),
        ("at", super::tools::sized(seen.at)),
    ])
}

fn uncounted(tallied: &Tallied, tally: Tally, stencil: Stencil) -> Failed {
    let counts = tally.counts(stencil);
    let listed: Vec<Json> = tallied
        .split
        .iter()
        .take(LISTED)
        .map(|one| {
            let mut fields = vec![
                ("write", counted(one.write)),
                ("count", counted(one.class % counts)),
            ];
            if tally == Tally::Outer {
                fields.push(("centre", counted(one.class / counts)));
            }
            fields.push((
                "seen",
                Json::List(vec![split_shown(0, one.zero), split_shown(1, one.one)]),
            ));
            map(fields)
        })
        .collect();
    let count = u32::try_from(tallied.split.len()).unwrap_or(u32::MAX);
    Box::new(Told(
        Refusal::new(
            Code::NotCounting,
            "$.rule",
            format!(
                "{count} counts were seen with both outputs of one write, so no {} law answers every example; counts lists the first {}, each with a row for each output and where it was first seen",
                tally.name(),
                listed.len()
            ),
        ),
        vec![
            ("not_counting", counted(count)),
            ("counts", Json::List(listed)),
        ],
    ))
}

fn by_count(tally: Tally, stencil: Stencil, each: &dyn Fn(u32) -> Option<Json>) -> Json {
    let counts = tally.counts(stencil);
    let row = |from: u32| Json::List((from..from + counts).filter_map(each).collect());
    match tally {
        Tally::Totalistic => row(0),
        Tally::Outer => Json::List(vec![row(0), row(counts)]),
    }
}

fn counted_law(
    server: &mut Server<'_>,
    held: &Json,
    asked: &Asked,
    tally: Tally,
) -> Result<Json, Failed> {
    let stencil = asked.stencil;
    if held.get("start").is_some() {
        return Err(refused(
            Code::StartRule,
            "$.start",
            format!(
                "rule {} reads each write's table from the seen rows in one pass with no search, so it takes no start; start goes with rule minimal or symmetric",
                tally.name()
            ),
        ));
    }
    if asked.reads.len() != 1 {
        return Err(limited(
            Code::CountReads,
            "$.reads",
            format!(
                "rule {} counts the places of one read, and {} reads were given",
                tally.name(),
                asked.reads.len()
            ),
            counted(1),
        ));
    }
    let inputs = stencil.positions();
    let outputs = u32::try_from(asked.writes.len()).unwrap_or(u32::MAX);
    let channels = channels_of(held, asked)?;
    plane_checks(asked, channels)?;
    let owned = examples_of(server, asked, held)?;
    let samples: Vec<Sample<'_>> = owned.iter().map(Owned::sample).collect();
    let rows = stencil::rows_of(stencil, &asked.reads, &asked.writes, &samples, asked.border);
    if !rows.conflicts.is_empty() {
        return Err(conflicted(&rows));
    }
    let tallied = stencil::tallied(tally, stencil, outputs, &rows.seen);
    if !tallied.split.is_empty() {
        return Err(uncounted(&tallied, tally, stencil));
    }
    let table: Vec<u32> = tallied.table.iter().map(|one| one.unwrap_or(0)).collect();
    let gates = stencil::tally_gates(tally, stencil, outputs, &table);
    for (row, one) in &rows.seen {
        if stencil::answer(inputs, outputs, &gates, *row) != one.outputs {
            return Err(format!(
                "the counting circuit answers row {row} other than the examples; this is a fault of the server"
            )
            .into());
        }
    }
    let circuit = Circuit {
        state: 0,
        inputs,
        outputs,
        gates: gates.clone(),
    };
    let mut answer = super::law::chip_law(circuit, wiring(asked, held, channels))?;
    let total = 1_u64 << inputs;
    let seen = u64::try_from(rows.seen.len()).unwrap_or(u64::MAX);
    let unseen = total - seen;
    let counts = tally.counts(stencil);
    let mut tables = Vec::with_capacity(outputs as usize);
    let mut never = Vec::with_capacity(outputs as usize);
    let mut ones = Vec::with_capacity(outputs as usize);
    let mut zeros = Vec::with_capacity(outputs as usize);
    for output in 0..outputs {
        let bit = |class: u32| {
            table
                .get(class as usize)
                .map_or(0, |word| (word >> output) & 1)
        };
        tables.push(by_count(tally, stencil, &|class| Some(counted(bit(class)))));
        never.push(by_count(tally, stencil, &|class| {
            tallied
                .table
                .get(class as usize)
                .is_some_and(Option::is_none)
                .then(|| counted(class % counts))
        }));
        let all: u64 = (0..tally.classes(stencil))
            .filter(|class| bit(*class) == 1)
            .map(|class| tally.rows_in(stencil, class))
            .sum();
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
        ("rule", text(tally.name())),
        (
            "means",
            text(match tally {
                Tally::Totalistic => TOTALISTIC,
                Tally::Outer => OUTER,
            }),
        ),
        ("zeros", Json::List(zeros)),
        ("ones", Json::List(ones)),
    ];
    if inputs <= TABLE_INPUTS {
        let whole = stencil::table(inputs, outputs, &gates);
        decided.push((
            "table",
            Json::List(whole.iter().map(|one| counted(*one)).collect()),
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
        ("border", text(border_name(asked.border))),
        ("runs_as", text(runs_as(asked.border))),
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
            "chip",
            map(vec![
                ("inputs", counted(inputs)),
                ("outputs", counted(outputs)),
                ("gates", gates_shown(&gates)),
            ]),
        ),
        ("tables", Json::List(tables)),
        ("unseen_counts", Json::List(never)),
        ("unseen", map(decided)),
        ("ladder", Json::List(Vec::new())),
        ("stopped", text(NO_SEARCH)),
    ]);
    Ok(map(answer))
}

const fn border_name(border: Border) -> &'static str {
    match border {
        Border::Zero => "zero",
        Border::Wrapped => "wrapped",
    }
}

const fn runs_as(border: Border) -> &'static str {
    match border {
        Border::Zero => ZERO_RUNS,
        Border::Wrapped => WRAPPED_RUNS,
    }
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
        Some("totalistic") => return counted_law(server, held, &asked, Tally::Totalistic),
        Some("outer") => return counted_law(server, held, &asked, Tally::Outer),
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
    let channels = channels_of(held, &asked)?;
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
    let capped = asked_top.min(fits_law);
    if capped < outputs {
        return Err(limited(
            Code::LearnLimit,
            "$.gates",
            format!("gates is {capped}, and each of the {outputs} outputs is a gate"),
            counted(outputs),
        ));
    }
    let begun = start_of(held, inputs, outputs)?;
    if let Some(live) = &begun {
        let most = if symmetric {
            fits_law.min(GATES_MOST)
        } else {
            GATES_MOST
        };
        let count = u32::try_from(live.len()).unwrap_or(u32::MAX);
        if count > most {
            return Err(limited(
                Code::LearnLimit,
                "$.start",
                format!(
                    "the start has {count} live gates, more than the {most} a rung of the ladder takes{}; nothing was searched",
                    if symmetric {
                        ", since under symmetric the law is the copies of the circuit and their ors and must fit the gates a law always holds"
                    } else {
                        ""
                    }
                ),
                counted(most),
            ));
        }
    }
    let effort = Effort {
        crowd: int(held, "crowd")?,
        rounds: int(held, "rounds")?,
        seed: int(held, "seed")?,
        ..Effort::of(CROWD, ROUNDS)
    };
    let effort = Effort {
        submitted: faults::submitted_most(effort.submitted),
        ..effort
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
    let started = begun.map(|live| {
        let law = if symmetric {
            stencil::symmetrised(inputs, outputs, &live, &moves)
        } else {
            live.clone()
        };
        let misses = stencil::missed(inputs, outputs, &law, &rows.seen);
        Started { live, law, misses }
    });
    let solved = started.as_ref().is_some_and(|one| one.misses.is_empty());
    let begun_gates = started
        .as_ref()
        .map_or(0, |one| u32::try_from(one.live.len()).unwrap_or(u32::MAX));
    let (top, top_field) = match &started {
        Some(_) if solved => (begun_gates.saturating_sub(1), "$.start"),
        Some(_) if begun_gates > capped => (begun_gates, "$.start"),
        _ => (capped, "$.gates"),
    };
    let searching = top >= outputs.max(1);
    fits_kernel(inputs, outputs, top, rows.seen.len())?;
    let most = usize::try_from(rows_most(inputs, outputs, top)).unwrap_or(usize::MAX);
    let mut watch = Watch {
        guard: server.guard(),
        host: server.host,
        stop: None,
        crowd_most: u32::MAX,
        #[cfg(feature = "faults")]
        faults: server.faults(),
    };
    let mut stopped: Option<Stop> = None;
    let mut expanding: Option<Stop> = None;
    let wanted: Option<std::collections::BTreeMap<u32, u32>> = if symmetric {
        let mut halted = || {
            let braked = watch.braked(Point::Expansion);
            stopped = watch.stop;
            braked
        };
        match stencil::orbits(&rows.seen, &moves, most, &mut halted) {
            Ok(orbits) => {
                if !orbits.broken.is_empty() {
                    return Err(asymmetric(&orbits, stencil));
                }
                Some(orbits.expanded)
            }
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
                let stop = stopped.unwrap_or(Stop::Budget);
                if !solved {
                    return Err(halted_refusal(
                        stop,
                        "while expanding the seen rows under the symmetries",
                    ));
                }
                expanding = Some(stop);
                None
            }
        }
    } else {
        Some(
            rows.seen
                .iter()
                .map(|(row, one)| (*row, one.outputs))
                .collect(),
        )
    };
    let images_wrong = match (&started, &wanted) {
        (Some(one), Some(wanted)) if symmetric => Some(
            wanted
                .iter()
                .filter(|(row, out)| stencil::answer(inputs, outputs, &one.law, **row) != **out)
                .count() as u64,
        ),
        (Some(_), None) => Some(0),
        _ => None,
    };
    let asked_crowd = effort.crowd;
    let mut crowd_used = effort.crowd;
    let laddered = match &wanted {
        Some(wanted) if searching && expanding.is_none() => {
            let traces: Vec<(Vec<u32>, Vec<u32>)> = wanted
                .iter()
                .map(|(row, out)| (vec![*row], vec![*out]))
                .collect();
            fits_kernel(inputs, outputs, top, traces.len())?;
            let crowd_most = crowd_fitting(host, inputs, top, traces.len())?;
            match kernel_fitting(
                host,
                &mut watch,
                (inputs, outputs),
                (top, top_field),
                traces.len(),
            )? {
                Some(stop) => {
                    if !solved {
                        return Err(halted_refusal(stop, CALIBRATING));
                    }
                    Laddered {
                        rungs: Vec::new(),
                        best: None,
                        stop: Some(stop),
                    }
                }
                None => {
                    watch.crowd_most = crowd_most;
                    crowd_used = effort.crowd.min(crowd_most);
                    let effort = Effort {
                        crowd: crowd_used,
                        ..effort
                    };
                    let sown: Vec<Vec<u32>> = started
                        .as_ref()
                        .filter(|_| !solved)
                        .map(|one| {
                            stencil::padded(inputs, &one.live, top)
                                .iter()
                                .map(|gate| gate.pack())
                                .collect()
                        })
                        .into_iter()
                        .collect();
                    ladder(
                        host,
                        &mut watch,
                        (inputs, outputs),
                        (&traces, &sown),
                        top,
                        effort,
                    )?
                }
            }
        }
        _ => Laddered {
            rungs: Vec::new(),
            best: None,
            stop: expanding,
        },
    };
    let begun_best = started
        .as_ref()
        .filter(|_| solved)
        .map(|one| (begun_gates, one.live.clone()));
    let Some((found, gates)) = laddered.best.clone().or(begun_best) else {
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
        let searched = wanted.as_ref().map_or(0, std::collections::BTreeMap::len);
        return Err(Box::new(Told(
            Refusal::new(
                Code::Unsolved,
                "$.gates",
                format!(
                    "no circuit of at most {top} gates answering every one of the {searched} rows was found within these rounds and crowd; the best scored {marks} of {of} marks"
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
    let checked_rows: Vec<(u32, u32)> = match &wanted {
        Some(wanted) => wanted.iter().map(|(row, out)| (*row, *out)).collect(),
        None => rows
            .seen
            .iter()
            .map(|(row, one)| (*row, one.outputs))
            .collect(),
    };
    for (row, out) in &checked_rows {
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
    let already = laddered.stop.is_some();
    let mut halted = || !already && watch.cancelled(Point::Count);
    let Some(count) = stencil::ones(inputs, outputs, &law_gates, &mut halted) else {
        return Err(halted_refusal(
            Stop::Cancelled,
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
    if symmetric && let Some(wanted) = &wanted {
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
    let mut rungs_shown = ladder_shown(&laddered.rungs);
    if solved && let Json::List(rungs_held) = &mut rungs_shown {
        rungs_held.insert(
            0,
            map(vec![
                ("gates", counted(begun_gates)),
                ("solved", Json::Bool(true)),
                ("start", Json::Bool(true)),
            ]),
        );
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
        ("ladder", rungs_shown),
        ("stopped", text(laddered.stop.map_or("ladder", Stop::name))),
    ]);
    if let Some(one) = &started {
        let every = inputs <= TABLE_INPUTS;
        let changes = stencil::changed(inputs, outputs, (&one.law, &law_gates), &rows.seen, every);
        answer.push((
            "start",
            start_told(one, (solved, images_wrong), &changes, every),
        ));
    }
    if crowd_used < asked_crowd {
        answer.push((
            "crowd_capped",
            map(vec![
                ("asked", counted(asked_crowd)),
                ("used", counted(crowd_used)),
                (
                    "why",
                    text(
                        "a candidate holds the inputs plus the gates times the rows over 32 words in one binding, and this device's binding holds no larger crowd; it is not doubled past this",
                    ),
                ),
            ]),
        ));
    }
    if capped < asked_top {
        answer.push((
            "gates_capped",
            map(vec![
                ("asked", counted(asked_top)),
                ("used", counted(capped)),
                (
                    "why",
                    text(&format!(
                        "a symmetric law is {copies} copies of the circuit and {} or gates, and a law always holds {} gates, so the ladder starts at {capped}",
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

struct Started {
    live: Vec<Gate>,
    law: Vec<Gate>,
    misses: Vec<stencil::Miss>,
}

fn start_word(one: &Json, key: &str, at: &str) -> Result<u32, Failed> {
    one.get(key)
        .and_then(Json::int)
        .and_then(|held| u32::try_from(held).ok())
        .ok_or_else(|| {
            refused(
                Code::Malformed,
                &format!("{at}.{key}"),
                format!("{key} did not reach the tool as its schema names it"),
            )
        })
}

fn start_of(held: &Json, inputs: u32, outputs: u32) -> Result<Option<Vec<Gate>>, Failed> {
    let Some(given) = held.get("start") else {
        return Ok(None);
    };
    let given_inputs = start_word(given, "inputs", "$.start")?;
    let given_outputs = start_word(given, "outputs", "$.start")?;
    let state = match given.get("state") {
        None => 0,
        Some(_) => start_word(given, "state", "$.start")?,
    };
    if state != 0 || given_inputs != inputs || given_outputs != outputs {
        return Err(Box::new(Told(
            Refusal::new(
                Code::StartShape,
                "$.start",
                format!(
                    "the start has {given_inputs} inputs, {given_outputs} outputs and {state} state bits, and this request's law takes {inputs} inputs (the stencil's places times the reads), {outputs} outputs (one for each write) and no state"
                ),
            ),
            vec![("inputs", counted(inputs)), ("outputs", counted(outputs))],
        )));
    }
    let listed = given
        .get("gates")
        .and_then(Json::list)
        .ok_or_else(|| unread("start.gates"))?;
    let gates = listed
        .iter()
        .enumerate()
        .map(|(at, one)| {
            let path = format!("$.start.gates[{at}]");
            Ok(Gate {
                kind: start_word(one, "kind", &path)?,
                left: start_word(one, "left", &path)?,
                right: start_word(one, "right", &path)?,
            })
        })
        .collect::<Result<Vec<Gate>, Failed>>()?;
    let count = u32::try_from(gates.len()).unwrap_or(u32::MAX);
    let shape = Netlist::over(inputs, outputs, count, chip_kinds()).map_err(|why| {
        refused(
            Code::ChipRefused,
            "$.start.gates",
            format!("the start does not parse: {why}"),
        )
    })?;
    for (at, gate) in gates.iter().enumerate() {
        if let Err(fault) = shape.checked(u32::try_from(at).unwrap_or(u32::MAX), *gate) {
            return Err(refused(
                Code::ChipRefused,
                &format!("$.start.gates[{at}]"),
                format!("the start does not parse: gate {at} is not legal: {fault}"),
            ));
        }
    }
    Ok(Some(stencil::live(inputs, outputs, &gates)))
}

fn start_told(
    started: &Started,
    (solved, images_wrong): (bool, Option<u64>),
    changes: &[stencil::Change],
    every: bool,
) -> Json {
    let wrong: Vec<Json> = started
        .misses
        .iter()
        .take(LISTED)
        .map(|one| {
            map(vec![
                ("row", counted(one.row)),
                ("outputs", counted(one.seen.outputs)),
                ("answered", counted(one.answered)),
                ("example", counted(one.seen.example)),
                ("at", super::tools::sized(one.seen.at)),
            ])
        })
        .collect();
    let shown: Vec<Json> = changes
        .iter()
        .take(LISTED)
        .map(|one| {
            map(vec![
                ("row", counted(one.row)),
                ("seen", Json::Bool(one.seen)),
                ("start", counted(one.was)),
                ("law", counted(one.now)),
            ])
        })
        .collect();
    let on_seen = changes.iter().filter(|one| one.seen).count() as u64;
    let mut fields = vec![
        (
            "gates",
            counted(u32::try_from(started.live.len()).unwrap_or(u32::MAX)),
        ),
        ("solved", Json::Bool(solved)),
        ("checked", text(STARTED)),
        ("wrong", exact(started.misses.len() as u64)),
        ("wrong_rows", Json::List(wrong)),
    ];
    if let Some(images) = images_wrong {
        fields.push(("images_wrong", exact(images)));
    }
    fields.extend([
        ("changed", exact(changes.len() as u64)),
        ("changed_seen", exact(on_seen)),
        (
            "changed_unseen",
            exact((changes.len() as u64).saturating_sub(on_seen)),
        ),
        (
            "changed_over",
            text(if every { CHANGED_EVERY } else { CHANGED_SEEN }),
        ),
        ("changed_rows", Json::List(shown)),
    ]);
    map(fields)
}
