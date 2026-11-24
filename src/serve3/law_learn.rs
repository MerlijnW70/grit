use std::time::Duration;

use crate::law::chip::{Circuit, Copies, Read, Rest, Spot, Wiring};
use crate::machine::Machine;
use crate::netlist::{Gate, Netlist};
use crate::serve::{Effort, Halt};
use crate::stencil::{
    self, Bit, Border, Counting, Deviation, Rows, Sample, Stencil, Tallied, Tally, Voted, Voting,
};
use crate::wire::{Json, counted, map, text};

use super::budget::{Guard, Stop};
use super::errors::{Code, Refusal};
use super::gradient::Told;
use super::law_real;
use super::schema::{choice, object, property, whole, with, word};
use super::tools::{int, ints, limited, refused, tool, unread};
use super::{Failed, Server, Tool};
use crate::steps::least::Search;

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

pub const TOTALISTIC_PLACES: &str = "each write's output is T[n], n the number of the places named in places, centre included, whose read bit is 1; the law reads no other place; T is read from the seen rows in one pass with no search, and a count the examples never showed is 0";

pub const OUTER_PLACES: &str = "each write's output is T[c][n], c the centre's read bit and n the number of the other places named in places whose read bit is 1; the law reads no other place; T is read from the seen rows in one pass with no search, and a count the examples never showed is 0";

pub const SUPPORT_MOST: u32 = 64;

pub const NO_SEARCH: &str = "no_search";

pub const STARTED: &str = "the start's law, its live gates under minimal and the or of their images under symmetric, was answered on the host on every seen row and, under symmetric, on every image of one";

pub const CHANGED_EVERY: &str = "every row";

pub const CHANGED_SEEN: &str = "the seen rows";

pub const ZERO_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; to read 0 past an example's edge, write it at the origin of a world at least one cell longer than it along each axis the stencil reads, every other cell of the read planes 0";

pub const WRAPPED_RUNS: &str = "the law reads its neighbours wrapped on the world's torus; an inline example wraps on itself, so it runs alike on a world whose sides equal its own along each axis the stencil reads (3x3 reads x and y, so its tall is free; a world's tall is at least 2), and a world example's box reads its neighbours from that world";

pub const LEARN: &str = "grit_law_learn builds the stencil's rows from example worlds, refuses a row seen with two outputs (conflict) and, under rule symmetric, rows whose rotations or reflections disagree (asymmetric), then walks a gate ladder down from gates as grit_least does, with grit_machine's search over the seen rows: each rung starts its crowd at most at what one device binding holds and doubles it while unsolved up to 65536 or that limit, whichever is less; the answer names a lowered crowd as crowd_capped. The ladder ends after two failed rungs in a row. The law is the smallest solved circuit, wired as grit_law_from_chip would wire it, with the row order below. A call stops on its deadline or cancel only at a brake: before each build that measures the device, before each search's build and before each device submission of a look; no measured time is compared with the time left. Once the deadline has passed no further submission starts, so an answer can come late by the one submission in flight: a build, or one submission of a look, sized by the device's measured pace. A build cannot stop part way, and its kernel loops over the rows searched (under symmetric every image of every seen row), so it does not grow with them; before the ladder the call builds the kernel of its smallest rung, then larger ones while each build's first submission, where a driver that compiles at its first dispatch compiles, grown with the square of the kernel's words from the closest one measured, stays within half the device's patience; a top rung past that is refused learn_limit at gates with the most gates that fit, a limit measured on this device at this moment and so outside the same answer below; a stopped ladder answers the smallest circuit it solved so far, with stopped budget or cancelled, and refuses budget or cancelled when none. The seen rows, and under symmetric their images, are refused learn_limit before any search when they pass the rows one kernel holds; the expansion stops as soon as it passes them, so it never holds more. The expansion checks budget and cancel every 4096 rows and refuses budget or cancelled. The count of decided rows checks only cancel, every 4096 blocks of 64 rows, and none after a stopped ladder; otherwise it runs to its end, at most two to the inputs less 6 blocks times the inputs plus the law's gates, so a found law is never lost to the deadline. Under symmetric the ladder starts at most at the gates whose copies and ors fit the 600 gates a law always holds, and the answer names a lowered start as gates_capped. The same request gives the same answer on the same device when it answers stopped ladder; a stop on budget or cancel depends on when the deadline passed or the cancel came, and its ladder is a prefix of that answer's ladder, with the smallest circuit solved in it. Under rule totalistic or outer there is no search, ladder, brake or search kernel, and inline examples need no compute device: each write's table is read from the seen rows in one pass, a count never seen is 0, and the law counts with a gate circuit, an adder tree and a decoder of the count. Under rule minimal or symmetric, start is a circuit to begin from, the chip of an earlier answer; it is checked on the host on every seen row before any search: one that answers them all is a solved rung, so the answer is never larger than it and never refused budget or cancelled, and the ladder walks down from its live gates less one; one that misses rows is a member of the first rung's crowd, kept unless a candidate scores at least as well, and the ladder starts at its live gates when gates is fewer. Under rule totalistic or outer, noise vote reads the examples cell by cell: each write's entry for a count, and under outer the centre, is the majority of the example cells that show it, a tie is 0 and listed in ties, and a count never seen is 0; no row is refused conflict or not_counting, examples.conflicts stays 0, and every other refusal stands. The answer then adds per write tallies, [zeros, ones] for each count in the shape of tables, ties, deviating, the example cells whose output disagrees with the law, and deviating_cells, at most 64 of them in example order, then z, y, x, each with example, at, count, centre under outer, seen and law; and deviating_share, the deviating cells over the voted cells (the example cells times the writes) as [num, den]. On examples that noise refuse answers, vote answers the same bytes with only these fields added. noise refuse, the default, answers as a request without noise, and noise vote under minimal or symmetric is refused noise_rule at $.noise. Under rule totalistic or outer, places names the counted places by a preset: moore, every place, as without places; von_neumann, the centre and its 4 (3x3) or 6 (3x3x3) face neighbours; hex, 3x3 only, places 1, 2, 3, 4, 5, 6, 7, the centre and the axial hex neighbours; or place_list names them as distinct place indices that hold the centre. Under totalistic the count runs over those places, the centre included, and under outer over those other than the centre, which stays the separate bit; the law reads no other place, and the counts, tables, unseen_counts, not_counting and the vote shrink to them. The answer then adds places, the sorted list and the preset if one was given. A preset the stencil lacks, an index outside it, one named twice, a list without the centre, or both places and place_list is refused places; places or place_list under minimal or symmetric is refused places_rule. min_support, 1 if left out and at most the stencil's places times 64, goes with noise vote only and is refused support_noise without it: the answer adds thin, per write in the shape of ties, each count shown by at least one and fewer than min_support example cells, with its count and tally; such a count is still decided by the vote and the law does not change";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    Calibration,
    Build,
    Submission,
    Submitted,
    Expansion,
    Count,
    Search,
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
            Self::Search => "search",
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
            Self::Search => 6,
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
        static REACHED: Cell<[u64; 7]> = const { Cell::new([0; 7]) };
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
        REACHED.set([0; 7]);
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
                    "how rows the examples never showed are decided: minimal, the smallest circuit found; symmetric, alike under the stencil's rotations and reflections; totalistic, by the count of places whose read bit is 1; outer, by the centre's bit and the count of the other places. totalistic and outer take one read, read each write's table from the seen rows in one pass with no search, and decide a count never seen 0. linear reads whole words as signed fixed-point numbers and fits each write as a sum of coefficients times terms plus a bias, by least squares",
                    &["minimal", "symmetric", "totalistic", "outer", "linear"],
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
                "parallel",
                with(
                    property(
                        "boolean",
                        "rules minimal, symmetric, totalistic and outer: answer the law in bit-parallel form: each read and write is the whole word of its plane at bit 0, and bit j of every word is an independent copy j, so one world runs 32 copies; false if left out",
                    ),
                    vec![("default", Json::Bool(false))],
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
            (
                "noise",
                with(
                    choice(
                        "rule totalistic or outer: how examples that disagree are taken; refuse, as conflict and not_counting; vote, each count decided by the majority of the example cells that show it, a tie 0, and the cells that disagree with the law listed; least_wrong, with any steps, the table with the fewest wrong example cells after k steps, with margins, the entries the data does not pin and a noise estimate; likely, the same, then the table that makes the examples most likely when cells of both frames flip at the estimated rate",
                        &["refuse", "vote", "least_wrong", "likely"],
                    ),
                    vec![("default", text("refuse"))],
                ),
            ),
            (
                "places",
                choice(
                    "rule totalistic or outer: the places counted, the centre always among them; moore, every place, if left out; von_neumann, the centre and its 4 (3x3) or 6 (3x3x3) face neighbours; hex, 3x3 only, the centre and places 1, 2, 3, 5, 6, 7, the axial hex neighbours (+1,0) (-1,0) (0,+1) (0,-1) (+1,-1) (-1,+1) as (dx, dy). Give this or place_list",
                    &stencil::PRESETS,
                ),
            ),
            (
                "place_list",
                list_shown(
                    word(),
                    "rule totalistic or outer: the places counted as stencil place indices in the row order, distinct, in the stencil and with the centre (4 for 3x3, 13 for 3x3x3); give this or places",
                    1,
                    27,
                ),
            ),
            (
                "steps",
                whole(
                    "rule totalistic or outer: k, how many applications of the law lie between each example's input and output; 1 if left out, at most 8; above 1 the tables of the rule are searched",
                    1,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "search",
                choice(
                    "rule totalistic or outer: how the tables are searched; exhaustive, every table, at most 18 classes, the default up to 18; local, the default past 18, the best interval tables then single and pair flips kept only when they lower the wrong cells",
                    &["exhaustive", "local"],
                ),
            ),
            (
                "poly",
                whole(
                    "rule linear: 1 (if left out) for the counted places only; 2 adds the products of two reads' centres and each centre times the sum of a read's other counted places; 3 adds the products of three centres",
                    1,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "frac",
                whole(
                    "rule linear: the fractional bits of the signed fixed-point number in each read and written word; 16 if left out, at most 30",
                    0,
                    i64::from(u32::MAX),
                ),
            ),
            (
                "clamp",
                list_shown(
                    whole("a signed word", i64::from(i32::MIN), i64::from(i32::MAX)),
                    "rule linear: [least, greatest] signed words each written word is clamped to",
                    2,
                    2,
                ),
            ),
            (
                "range",
                list_shown(
                    whole("a signed word", i64::from(i32::MIN), i64::from(i32::MAX)),
                    "rule linear: [least, greatest] signed words of every read the law is built never to wrap on; it must hold every example input word; the examples' own least and greatest if left out",
                    2,
                    2,
                ),
            ),
            (
                "min_support",
                whole(
                    "noise vote: a count shown by fewer example cells than this is listed in thin with its tally; 1 if left out, at most the stencil's places times 64",
                    1,
                    27 * i64::from(SUPPORT_MOST),
                ),
            ),
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
    margin: u32,
    copies: Copies,
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
                *start = (*start + whole - asked.margin % whole) % whole;
                *side += 2 * asked.margin;
                *offset = asked.margin;
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

fn copies_of(held: &Json, asked: &Asked) -> Result<Copies, Failed> {
    match held.get("parallel") {
        None | Some(Json::Bool(false)) => return Ok(Copies::One),
        Some(Json::Bool(true)) => {}
        Some(_) => return Err(unread("parallel")),
    }
    let bits = asked
        .reads
        .iter()
        .enumerate()
        .map(|(at, one)| ("reads", at, one.bit))
        .chain(
            asked
                .writes
                .iter()
                .enumerate()
                .map(|(at, one)| ("writes", at, one.bit)),
        );
    for (list, at, bit) in bits {
        if bit != 0 {
            return Err(refused(
                Code::ParallelBit,
                &format!("$.{list}[{at}].bit"),
                format!(
                    "{list}[{at}] names bit {bit}; with parallel true a read or write is the whole word of its plane, bit j of it copy j, so each names bit 0"
                ),
            ));
        }
    }
    Ok(Copies::Word)
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

fn uncounted(tallied: &Tallied, counting: Counting) -> Failed {
    let tally = counting.tally;
    let counts = counting.counts();
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

fn by_count(counting: Counting, each: &dyn Fn(u32) -> Option<Json>) -> Json {
    let counts = counting.counts();
    let row = |from: u32| Json::List((from..from + counts).filter_map(each).collect());
    match counting.tally {
        Tally::Totalistic => row(0),
        Tally::Outer => Json::List(vec![row(0), row(counts)]),
    }
}

struct Masked {
    mask: u32,
    preset: Option<&'static str>,
}

fn places_refused(field: &str, message: String) -> Failed {
    refused(Code::Places, field, message)
}

fn masked(held: &Json, stencil: Stencil, what: &str) -> Result<Option<Masked>, Failed> {
    let positions = stencil.positions();
    let centre = stencil.place([0, 0, 0]);
    match (held.get("places"), held.get("place_list")) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(places_refused(
            "$.place_list",
            "places and place_list were both given, and the counted places are named by one of them".to_owned(),
        )),
        (Some(one), None) => {
            let asked = one.text().ok_or_else(|| unread("places"))?;
            let preset = stencil::PRESETS
                .iter()
                .find(|name| **name == asked)
                .ok_or_else(|| unread("places"))?;
            let mask = stencil::preset(stencil, preset).ok_or_else(|| {
                places_refused(
                    "$.places",
                    format!(
                        "places {preset} names a neighbourhood the {} stencil does not have; hex is a 3x3 neighbourhood",
                        stencil.name()
                    ),
                )
            })?;
            Ok(Some(Masked {
                mask,
                preset: Some(preset),
            }))
        }
        (None, Some(one)) => {
            let listed = ints(one, "place_list")?;
            let mut mask = 0_u32;
            for (at, place) in listed.iter().enumerate() {
                if *place >= positions {
                    return Err(places_refused(
                        &format!("$.place_list[{at}]"),
                        format!(
                            "place {place} is not a place of the {} stencil, whose places are 0 to {}",
                            stencil.name(),
                            positions - 1
                        ),
                    ));
                }
                if (mask >> place) & 1 == 1 {
                    return Err(places_refused(
                        &format!("$.place_list[{at}]"),
                        format!("place {place} is named more than once"),
                    ));
                }
                mask |= 1 << place;
            }
            if (mask >> centre) & 1 == 0 {
                return Err(places_refused(
                    "$.place_list",
                    format!(
                        "the centre, place {centre} of the {} stencil, is not named, and {what} always reads it",
                        stencil.name()
                    ),
                ));
            }
            Ok(Some(Masked { mask, preset: None }))
        }
    }
}

fn support_of(held: &Json, stencil: Stencil) -> Result<Option<u32>, Failed> {
    if held.get("min_support").is_none() {
        return Ok(None);
    }
    let asked = int(held, "min_support")?;
    let most = stencil.positions() * SUPPORT_MOST;
    if asked > most {
        return Err(limited(
            Code::SupportLimit,
            "$.min_support",
            format!(
                "min_support is {asked}, and the {} stencil takes 1 to {most}, its places times {SUPPORT_MOST}",
                stencil.name()
            ),
            counted(most),
        ));
    }
    Ok(Some(asked))
}

fn places_shown(counting: Counting, preset: Option<&str>) -> Json {
    let mut fields = vec![(
        "list",
        Json::List(counting.places().into_iter().map(counted).collect()),
    )];
    if let Some(name) = preset {
        fields.push(("preset", text(name)));
    }
    map(fields)
}

fn counted_law(
    server: &mut Server<'_>,
    held: &Json,
    asked: &Asked,
    tally: Tally,
    vote: bool,
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
    let given = masked(held, stencil, "a counting law")?;
    let support = support_of(held, stencil)?;
    let counting = Counting {
        tally,
        stencil,
        mask: given
            .as_ref()
            .map_or(Counting::moore(tally, stencil).mask, |one| one.mask),
    };
    let inputs = stencil.positions();
    let outputs = u32::try_from(asked.writes.len()).unwrap_or(u32::MAX);
    let channels = channels_of(held, asked)?;
    plane_checks(asked, channels)?;
    let owned = examples_of(server, asked, held)?;
    let samples: Vec<Sample<'_>> = owned.iter().map(Owned::sample).collect();
    let rows = stencil::rows_of(stencil, &asked.reads, &asked.writes, &samples, asked.border);
    let voted = if vote {
        let voting = Voting {
            counting,
            reads: &asked.reads,
            writes: &asked.writes,
            border: asked.border,
        };
        Some(stencil::voted(voting, &samples, LISTED))
    } else {
        None
    };
    let entries = if let Some(one) = &voted {
        one.table.clone()
    } else {
        if !rows.conflicts.is_empty() {
            return Err(conflicted(&rows));
        }
        let tallied = stencil::tallied(counting, outputs, &rows.seen);
        if !tallied.split.is_empty() {
            return Err(uncounted(&tallied, counting));
        }
        tallied.table
    };
    let table: Vec<u32> = entries.iter().map(|one| one.unwrap_or(0)).collect();
    let law_at = |row: u32| {
        table
            .get(counting.class(row) as usize)
            .copied()
            .unwrap_or(0)
    };
    let gates = stencil::counted_gates(counting, outputs, &table);
    for (row, one) in &rows.seen {
        let wanted = if vote { law_at(*row) } else { one.outputs };
        if stencil::answer(inputs, outputs, &gates, *row) != wanted {
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
    let mut answer = super::law::chip_law(circuit, wiring(asked, held, channels), asked.copies)?;
    let total = 1_u64 << inputs;
    let seen = u64::try_from(rows.seen.len()).unwrap_or(u64::MAX);
    let unseen = total - seen;
    let counts = counting.counts();
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
        tables.push(by_count(counting, &|class| Some(counted(bit(class)))));
        never.push(by_count(counting, &|class| {
            entries
                .get(class as usize)
                .is_some_and(Option::is_none)
                .then(|| counted(class % counts))
        }));
        let all: u64 = (0..counting.classes())
            .filter(|class| bit(*class) == 1)
            .map(|class| counting.rows_in(class))
            .sum();
        let on_seen = rows
            .seen
            .keys()
            .filter(|row| (law_at(**row) >> output) & 1 == 1)
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
            text(match (tally, given.is_some()) {
                (Tally::Totalistic, false) => TOTALISTIC,
                (Tally::Outer, false) => OUTER,
                (Tally::Totalistic, true) => TOTALISTIC_PLACES,
                (Tally::Outer, true) => OUTER_PLACES,
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
    if let Some(one) = &given {
        answer.push(("places", places_shown(counting, one.preset)));
    }
    if let Some(one) = &voted {
        answer.extend(vote_shown(one, counting));
        if let Some(least) = support {
            answer.push(("thin", thin_shown(one, counting, least)));
        }
    }
    Ok(map(answer))
}

const LINEAR_KEYS: [&str; 4] = ["poly", "frac", "clamp", "range"];

pub const LINEAR_ITERATE: &str = "after each frame write each written plane's words back to the read plane at the same position in reads, then step again";

fn signed_pair(held: &Json, key: &str) -> Result<Option<[i32; 2]>, Failed> {
    let Some(one) = held.get(key) else {
        return Ok(None);
    };
    let words = one
        .list()
        .ok_or_else(|| unread(key))?
        .iter()
        .map(|item| {
            item.int()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| unread(key))
        })
        .collect::<Result<Vec<i32>, Failed>>()?;
    let pair = <[i32; 2]>::try_from(words.as_slice()).map_err(|_| unread(key))?;
    if pair[0] > pair[1] {
        return Err(refused(
            Code::Reversed,
            &format!("$.{key}"),
            format!(
                "{key} [{}, {}] has its least past its greatest",
                pair[0], pair[1]
            ),
        ));
    }
    Ok(Some(pair))
}

fn whole_words(spots: &[Bit], key: &str) -> Result<Vec<u32>, Failed> {
    for (which, spot) in spots.iter().enumerate() {
        if spot.bit != 0 {
            return Err(refused(
                Code::PlaneFormat,
                &format!("$.{key}[{which}].bit"),
                format!(
                    "{key} {which} names bit {}, and under rule linear plane {} holds one signed fixed-point number in its whole word, from bit 0",
                    spot.bit, spot.plane
                ),
            ));
        }
    }
    Ok(spots.iter().map(|one| one.plane).collect())
}

fn linear_law(
    server: &mut Server<'_>,
    held: &Json,
    asked: &Asked,
    vote: bool,
) -> Result<Json, Failed> {
    let stencil = asked.stencil;
    if held.get("start").is_some() {
        return Err(refused(
            Code::StartRule,
            "$.start",
            "rule linear fits its coefficients by least squares with no search, so it takes no start; start goes with rule minimal or symmetric".to_owned(),
        ));
    }
    if vote {
        return Err(refused(
            Code::NoiseRule,
            "$.noise",
            "rule linear fits by least squares and reports the residual, so it takes no vote; noise vote goes with rule totalistic or outer".to_owned(),
        ));
    }
    let reads = whole_words(&asked.reads, "reads")?;
    let writes = whole_words(&asked.writes, "writes")?;
    let poly = match held.get("poly") {
        None => 1,
        Some(_) => int(held, "poly")?,
    };
    if poly > law_real::POLY_MOST {
        return Err(limited(
            Code::PolyLimit,
            "$.poly",
            format!(
                "poly {poly} asks for terms of degree {poly}, and rule linear builds terms up to degree {}",
                law_real::POLY_MOST
            ),
            counted(law_real::POLY_MOST),
        ));
    }
    let frac = match held.get("frac") {
        None => law_real::FRAC,
        Some(_) => int(held, "frac")?,
    };
    if frac > law_real::FRAC_MOST {
        return Err(limited(
            Code::PlaneFormat,
            "$.frac",
            format!(
                "frac {frac} leaves no bit for the sign and the whole part of a 32-bit word; frac goes up to {}",
                law_real::FRAC_MOST
            ),
            counted(law_real::FRAC_MOST),
        ));
    }
    let clamp = signed_pair(held, "clamp")?;
    let range = signed_pair(held, "range")?;
    let given = masked(held, stencil, "a linear law")?;
    let mask = given
        .as_ref()
        .map_or((1 << stencil.positions()) - 1, |one| one.mask);
    let read_count = u32::try_from(reads.len()).unwrap_or(u32::MAX);
    let places = mask.count_ones();
    let inputs = places.saturating_mul(read_count);
    if inputs > INPUTS_MOST {
        return Err(limited(
            Code::LearnLimit,
            "$.reads",
            format!(
                "{read_count} reads at {places} counted places of the {} stencil make {inputs} place terms, more than the {INPUTS_MOST} a learned law takes",
                stencil.name()
            ),
            counted(INPUTS_MOST / places.max(1)),
        ));
    }
    let channels = channels_of(held, asked)?;
    plane_checks(asked, channels)?;
    let owned = examples_of(server, asked, held)?;
    if let Some([least, most]) = range {
        for (at, example) in owned.iter().enumerate() {
            for (read, words) in example.inputs.iter().enumerate() {
                if let Some(word) = words
                    .iter()
                    .map(|one| one.cast_signed())
                    .find(|one| *one < least || *one > most)
                {
                    return Err(refused(
                        Code::PlaneFormat,
                        "$.range",
                        format!(
                            "example {at} holds word {word} on read {read}, outside range [{least}, {most}]; the law is built never to wrap inside range, so range must hold every example input word"
                        ),
                    ));
                }
            }
        }
    }
    let samples: Vec<Sample<'_>> = owned.iter().map(Owned::sample).collect();
    let name = held.get("name").and_then(Json::text).unwrap_or("learned");
    let plan = law_real::Plan {
        stencil,
        reads: &reads,
        writes: &writes,
        mask,
        poly,
        frac,
        clamp,
        range,
        border: asked.border,
        channels,
        name,
    };
    let mut answer = law_real::learned(&plan, &samples)?;
    let cells: u64 = owned.iter().map(|one| cells_of(one.inner)).sum();
    let iterate = match (reads.as_slice(), writes.as_slice()) {
        ([read], [write]) => map(vec![(
            "alternate",
            Json::List(vec![counted(*read), counted(*write)]),
        )]),
        _ if reads.len() == writes.len() => map(vec![
            (
                "copy",
                Json::List(
                    writes
                        .iter()
                        .zip(&reads)
                        .map(|(write, read)| Json::List(vec![counted(*write), counted(*read)]))
                        .collect(),
                ),
            ),
            ("means", text(LINEAR_ITERATE)),
        ]),
        _ => map(Vec::new()),
    };
    answer.extend([
        ("stencil", text(stencil.name())),
        ("row_order", text(stencil::ORDER)),
        ("border", text(border_name(asked.border))),
        ("runs_as", text(runs_as(asked.border))),
        ("examples", map(vec![("cells", exact(cells))])),
        ("iterate", iterate),
        ("ladder", Json::List(Vec::new())),
        ("stopped", text(NO_SEARCH)),
    ]);
    if let Some(one) = &given {
        let counting = Counting {
            tally: Tally::Totalistic,
            stencil,
            mask,
        };
        answer.push(("places", places_shown(counting, one.preset)));
    }
    Ok(map(answer))
}

fn deviation_shown(one: &Deviation, tally: Tally, counts: u32) -> Json {
    let mut fields = vec![
        ("example", counted(one.example)),
        ("at", super::tools::sized(one.at)),
        ("count", counted(one.class % counts)),
    ];
    if tally == Tally::Outer {
        fields.push(("centre", counted(one.class / counts)));
    }
    fields.push(("seen", counted(one.seen)));
    fields.push(("law", counted(one.law)));
    map(fields)
}

fn thin_shown(voted: &Voted, counting: Counting, least: u32) -> Json {
    let counts = counting.counts();
    Json::List(
        voted
            .tallies
            .iter()
            .map(|held| {
                by_count(counting, &|class| {
                    let [zeros, ones] = held.get(class as usize).copied().unwrap_or([0, 0]);
                    let cells = zeros.saturating_add(ones);
                    (cells > 0 && cells < u64::from(least)).then(|| {
                        map(vec![
                            ("count", counted(class % counts)),
                            ("tally", Json::List(vec![exact(zeros), exact(ones)])),
                        ])
                    })
                })
            })
            .collect(),
    )
}

fn vote_shown(voted: &Voted, counting: Counting) -> Vec<(&'static str, Json)> {
    let tally = counting.tally;
    let counts = counting.counts();
    let tallies = voted
        .tallies
        .iter()
        .map(|held| {
            by_count(counting, &|class| {
                held.get(class as usize)
                    .map(|pair| Json::List(pair.iter().map(|one| exact(*one)).collect()))
            })
        })
        .collect();
    let ties = voted
        .ties
        .iter()
        .map(|tied| {
            by_count(counting, &|class| {
                tied.contains(&class).then(|| counted(class % counts))
            })
        })
        .collect();
    let cells = voted
        .deviations
        .iter()
        .map(|held| {
            Json::List(
                held.iter()
                    .map(|one| deviation_shown(one, tally, counts))
                    .collect(),
            )
        })
        .collect();
    let deviating: u64 = voted.deviating.iter().sum();
    let voted_cells = voted
        .cells
        .saturating_mul(u64::try_from(voted.deviating.len()).unwrap_or(u64::MAX));
    vec![
        ("tallies", Json::List(tallies)),
        ("ties", Json::List(ties)),
        (
            "deviating",
            Json::List(voted.deviating.iter().map(|one| exact(*one)).collect()),
        ),
        ("deviating_cells", Json::List(cells)),
        (
            "deviating_share",
            Json::List(vec![exact(deviating), exact(voted_cells)]),
        ),
    ]
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
        margin: 1,
        copies: Copies::One,
    };
    let asked = Asked {
        copies: copies_of(held, &asked)?,
        ..asked
    };
    let (vote, least, likely) = match held.get("noise") {
        None => (false, false, false),
        Some(one) => match one.text() {
            Some("refuse") => (false, false, false),
            Some("vote") => (true, false, false),
            Some("least_wrong") => (false, true, false),
            Some("likely") => (false, true, true),
            _ => return Err(unread("noise")),
        },
    };
    if !vote && held.get("min_support").is_some() {
        return Err(refused(
            Code::SupportNoise,
            "$.min_support",
            "min_support marks the counts a vote decides on few example cells, and noise is not vote; min_support goes with noise vote".to_owned(),
        ));
    }
    let linear = held.get("rule").and_then(Json::text) == Some("linear");
    if linear && asked.copies == Copies::Word {
        return Err(refused(
            Code::ParallelRule,
            "$.parallel",
            "rule linear reads each whole word as one signed fixed-point number, so a word holds no 32 bits to run as copies; parallel goes with rule minimal, symmetric, totalistic or outer".to_owned(),
        ));
    }
    if !linear {
        for key in LINEAR_KEYS {
            if held.get(key).is_some() {
                return Err(refused(
                    Code::LinearRule,
                    &format!("$.{key}"),
                    format!(
                        "{key} shapes a fixed-point law, and rule {} learns a law over bits; {key} goes with rule linear",
                        held.get("rule").and_then(Json::text).unwrap_or("")
                    ),
                ));
            }
        }
    }
    let steps = steps_of(held)?;
    let search = search_of(held)?;
    if let Some(rule @ ("minimal" | "symmetric" | "linear")) = held.get("rule").and_then(Json::text)
    {
        if search.is_some() {
            return Err(refused(
                Code::SearchRule,
                "$.search",
                format!(
                    "search picks how the tables of a counting rule are searched, and rule {rule} has no table; search goes with rule totalistic or outer"
                ),
            ));
        }
        if least {
            return Err(refused(
                Code::NoiseRule,
                "$.noise",
                format!(
                    "noise {} answers a counting table, and rule {rule} has no table; noise least_wrong and likely go with rule totalistic or outer",
                    if likely { "likely" } else { "least_wrong" }
                ),
            ));
        }
    }
    if steps > 1 || least || search.is_some() {
        let tally = match held.get("rule").and_then(Json::text) {
            Some("totalistic") => Tally::Totalistic,
            Some("outer") => Tally::Outer,
            Some(rule @ ("minimal" | "symmetric")) => {
                return Err(refused(
                    Code::StepsRule,
                    "$.steps",
                    format!(
                        "steps {steps} searches the tables of a counting rule, and rule {rule} searches for a circuit; steps above 1 go with rule totalistic or outer"
                    ),
                ));
            }
            Some("linear") => {
                return Err(refused(
                    Code::StepsRule,
                    "$.steps",
                    format!(
                        "steps {steps} searches the tables of a counting rule, and rule linear fits one step by least squares; steps above 1 go with rule totalistic or outer"
                    ),
                ));
            }
            _ => return Err(unread("rule")),
        };
        return stepped_law(
            server,
            held,
            &asked,
            tally,
            (steps, vote, least, search),
            likely,
        );
    }
    let symmetric = match held.get("rule").and_then(Json::text) {
        Some("symmetric") => true,
        Some("minimal") => false,
        Some("totalistic") => return counted_law(server, held, &asked, Tally::Totalistic, vote),
        Some("outer") => return counted_law(server, held, &asked, Tally::Outer, vote),
        Some("linear") => return linear_law(server, held, &asked, vote),
        _ => return Err(unread("rule")),
    };
    if vote {
        return Err(refused(
            Code::NoiseRule,
            "$.noise",
            format!(
                "rule {} searches for a circuit that answers every seen row, so it takes no vote; noise vote goes with rule totalistic or outer",
                if symmetric { "symmetric" } else { "minimal" }
            ),
        ));
    }
    for key in ["places", "place_list"] {
        if held.get(key).is_some() {
            return Err(refused(
                Code::PlacesRule,
                &format!("$.{key}"),
                format!(
                    "rule {} searches for a circuit over every place of the stencil, so it takes no {key}; {key} goes with rule totalistic or outer",
                    if symmetric { "symmetric" } else { "minimal" }
                ),
            ));
        }
    }
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
    let mut answer = super::law::chip_law(circuit, wiring(&asked, held, channels), asked.copies)?;
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

pub const STEPS: &str = "steps k from 2 to 8, under rule totalistic or outer, reads each example's output as the read bit after k applications of one counting table, with the example's border at every step: zero reads 0 past the example's box at every step; wrapped wraps an inline example on itself at every step, and reads a world example's box with k cells around it from that world, so it equals k steps of the law on that world. It takes one read and one write on the read's bit. Every table of the rule and its counted places is searched, 2 to the classes of them, at most 18 classes (262144 tables) unless search is local, on the host with no device: 64 tables at a time, each first run on the cells a few probe cells depend on and then on every example cell, a table dropped only once it is wrong on more cells than a table already seen. A table is numbered by the sum of T[class] times 2 to the class, class n under totalistic and the centre times the counts plus n under outer, and tables are taken in ascending number: the law is the first table that gives every example cell, consistent counts them all, consistent_tables lists the first 16 in the shape of tables, checked the tables tried or excluded and space the tables of the rule. The search reads no clock; it passes a brake before each 64 tables, so an answer can come late by the check of 64 tables. A stop on budget or cancel answers what it found, a prefix of the whole search's order, with stopped budget or cancelled and complete false, and refuses budget or cancelled when it found none. A complete search answers stopped complete and complete true, the same bytes for the same request; with no table consistent it is refused no_rule_k at examples, with closest, the first table with the fewest wrong example cells, and its wrong cells. The found law is checked on the host by running it k steps on every example before it is answered. iterate names how to run it: alternate, the read's and the write's planes, and frames k. steps 1 answers as a request without steps. Refused before any example is read: steps past 8 steps_limit, steps above 1 under minimal or symmetric steps_rule, with noise vote steps_noise, without exactly one write on the read's bit steps_writes (the write is on another plane than the read, at the read's bit, since a write on the read's plane is refused halo_written), and search exhaustive past 18 classes steps_space with space and classes. Past 18 classes, with search local, or with noise least_wrong the search of law_least runs instead";

pub const STEPS_ZERO_RUNS: &str = "the law is applied k times and each step read 0 past the example's edge; a world reproduces that where the cells around the example stay 0 for the k steps: create it with alternate, write the input to the read's plane, step k frames and read the current plane";

pub const STEPS_WRAPPED_RUNS: &str = "the law is applied k times on the example's torus; create a world of the example's size (a world example's own world) with alternate, write the input to the read's plane, step k frames and read the current plane";

pub const STEPS_TABLES_MOST: u64 = 1 << crate::steps::CLASSES_MOST;

fn steps_of(held: &Json) -> Result<u32, Failed> {
    if held.get("steps").is_none() {
        return Ok(1);
    }
    let asked = int(held, "steps")?;
    let most = crate::steps::STEPS_MOST;
    if asked > most {
        return Err(limited(
            Code::StepsLimit,
            "$.steps",
            format!(
                "steps is {asked}, and a law is learned from examples at most {most} steps apart"
            ),
            counted(most),
        ));
    }
    Ok(asked)
}

const fn steps_runs(border: Border) -> &'static str {
    match border {
        Border::Zero => STEPS_ZERO_RUNS,
        Border::Wrapped => STEPS_WRAPPED_RUNS,
    }
}

fn by_code(counting: Counting, code: u64) -> Json {
    Json::List(vec![by_count(counting, &|class| {
        Some(counted(crate::steps::entry(code, class)))
    })])
}

fn replayed(stepping: &crate::steps::Stepping, samples: &[Sample<'_>], table: &[u32]) -> bool {
    let counting = stepping.counting;
    let stencil = counting.stencil;
    let read = Bit { plane: 0, bit: 0 };
    samples.iter().all(|sample| {
        let words: &[u32] = match sample.inputs.as_slice() {
            [one, ..] => one,
            [] => &[],
        };
        let mut grid: Vec<u32> = words
            .iter()
            .map(|word| (word >> stepping.read) & 1)
            .collect();
        for _ in 0..stepping.steps {
            let whole = Sample {
                size: sample.size,
                inputs: vec![grid.as_slice()],
                from: [0; 3],
                inner: sample.size,
                outputs: Vec::new(),
            };
            let mut next = Vec::with_capacity(grid.len());
            for z in 0..sample.size[2] {
                for y in 0..sample.size[1] {
                    for x in 0..sample.size[0] {
                        let row = stencil::row_at(
                            stencil,
                            std::slice::from_ref(&read),
                            &whole,
                            [x, y, z],
                            stepping.border,
                        );
                        let class = counting.class(row) as usize;
                        next.push(table.get(class).copied().unwrap_or(0));
                    }
                }
            }
            grid = next;
        }
        let wanted: &[u32] = match sample.outputs.as_slice() {
            [one, ..] => one,
            [] => &[],
        };
        let [wide, deep, tall] = sample.inner;
        (0..tall).all(|z| {
            (0..deep).all(|y| {
                (0..wide).all(|x| {
                    let inside = [sample.from[0] + x, sample.from[1] + y, sample.from[2] + z];
                    let got = grid
                        .get(stencil::index(sample.size, inside))
                        .copied()
                        .unwrap_or(0);
                    let want = wanted
                        .get(stencil::index(sample.inner, [x, y, z]))
                        .map_or(0, |word| (word >> stepping.write) & 1);
                    got == want
                })
            })
        })
    })
}

fn stepped_law(
    server: &mut Server<'_>,
    held: &Json,
    asked: &Asked,
    tally: Tally,
    (steps, vote, least, search): (u32, bool, bool, Option<Search>),
    likely: bool,
) -> Result<Json, Failed> {
    let stencil = asked.stencil;
    if vote {
        return Err(refused(
            Code::StepsNoise,
            "$.noise",
            if steps > 1 {
                format!(
                    "noise vote decides each count by the example cells at one step, and steps is {steps}; a vote over examples k steps apart is not offered, so give noise refuse, noise least_wrong or steps 1"
                )
            } else {
                "noise vote decides each count by the example cells at one step with no search, and search was given; give noise least_wrong or drop search".to_owned()
            },
        ));
    }
    if held.get("start").is_some() {
        return Err(refused(
            Code::StartRule,
            "$.start",
            format!(
                "rule {} searches its tables with no circuit to start from; start goes with rule minimal or symmetric",
                tally.name()
            ),
        ));
    }
    let [read] = asked.reads.as_slice() else {
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
    };
    let given = masked(held, stencil, "a counting law")?;
    let counting = Counting {
        tally,
        stencil,
        mask: given
            .as_ref()
            .map_or(Counting::moore(tally, stencil).mask, |one| one.mask),
    };
    if asked.writes.len() != 1 || asked.writes.iter().any(|one| one.bit != read.bit) {
        return Err(refused(
            Code::StepsWrites,
            "$.writes",
            format!(
                "a law applied {steps} times reads back what it wrote, so it takes one write on the read's bit {}, and {} writes were given",
                read.bit,
                asked.writes.len()
            ),
        ));
    }
    let stepping = crate::steps::Stepping {
        counting,
        steps,
        border: asked.border,
        read: read.bit,
        write: read.bit,
    };
    let classes = stepping.classes();
    let space = stepping.space();
    let mode = match search {
        Some(asked) => asked,
        None if stepping.fits() => Search::Exhaustive,
        None => Search::Local,
    };
    if mode == Search::Exhaustive && !stepping.fits() {
        return Err(Box::new(Told(
            Refusal::new(
                Code::StepsSpace,
                "$.steps",
                format!(
                    "rule {} over the {} counted places of the {} stencil has {classes} classes, so {space} tables, past the {} tables ({} classes) a search over every table takes; nothing was searched",
                    tally.name(),
                    counting.places().len(),
                    stencil.name(),
                    STEPS_TABLES_MOST,
                    crate::steps::CLASSES_MOST
                ),
            )
            .limited(exact(STEPS_TABLES_MOST)),
            vec![("space", exact(space)), ("classes", counted(classes))],
        )));
    }
    let channels = channels_of(held, asked)?;
    plane_checks(asked, channels)?;
    let padded = Asked {
        stencil,
        reads: asked.reads.clone(),
        writes: asked.writes.clone(),
        border: asked.border,
        margin: steps,
        copies: asked.copies,
    };
    let owned = examples_of(server, &padded, held)?;
    let samples: Vec<Sample<'_>> = owned.iter().map(Owned::sample).collect();
    let mut watch = Watch {
        guard: server.guard(),
        host: server.host,
        stop: None,
        crowd_most: u32::MAX,
        #[cfg(feature = "faults")]
        faults: server.faults(),
    };
    if least || mode == Search::Local {
        let shape = Shape {
            asked,
            held,
            channels,
            given: given.as_ref(),
        };
        return least_law(
            &stepping,
            &samples,
            &mut watch,
            (mode, least, likely),
            &shape,
        );
    }
    let found = {
        let mut halted = || watch.braked(Point::Search);
        crate::steps::searched(&stepping, &samples, &mut halted)
    };
    let searched = vec![
        ("checked", exact(found.checked)),
        ("space", exact(space)),
        ("steps", counted(steps)),
    ];
    let [code, ..] = found.codes.as_slice() else {
        if !found.complete {
            let stop = watch.stop.unwrap_or(Stop::Budget);
            return Err(Box::new(Told(
                Refusal::new(
                    stop.code(),
                    if stop == Stop::Budget {
                        "$.budget_ms"
                    } else {
                        "$"
                    },
                    format!(
                        "the search stopped on {} after {} of {space} tables, before any table applied {steps} times gave every example; nothing was learned",
                        stop.name(),
                        found.checked
                    ),
                ),
                searched,
            )));
        }
        let (wrong, near) = found.closest.unwrap_or((found.cells, 0));
        let mut told = vec![
            (
                "closest",
                map(vec![
                    ("tables", by_code(counting, near)),
                    ("wrong", exact(wrong)),
                ]),
            ),
            ("cells", exact(found.cells)),
        ];
        told.extend(searched);
        return Err(Box::new(Told(
            Refusal::new(
                Code::NoRuleK,
                "$.examples",
                format!(
                    "none of the {space} tables of rule {} applied {steps} times gives every example; the closest is wrong on {wrong} of {} example cells",
                    tally.name(),
                    found.cells
                ),
            ),
            told,
        )));
    };
    let code = *code;
    let table = crate::steps::table_of(code, classes);
    if !replayed(&stepping, &samples, &table) {
        return Err(format!(
            "the found table {code} applied {steps} times does not give the examples when run again on the host; this is a fault of the server"
        )
        .into());
    }
    let inputs = stencil.positions();
    let gates = stencil::counted_gates(counting, 1, &table);
    if inputs <= TABLE_INPUTS {
        for row in 0..1_u32 << inputs {
            let wanted = table
                .get(counting.class(row) as usize)
                .copied()
                .unwrap_or(0);
            if stencil::answer(inputs, 1, &gates, row) != wanted {
                return Err(format!(
                    "the counting circuit answers row {row} other than the table; this is a fault of the server"
                )
                .into());
            }
        }
    }
    let circuit = Circuit {
        state: 0,
        inputs,
        outputs: 1,
        gates: gates.clone(),
    };
    let mut answer = super::law::chip_law(circuit, wiring(asked, held, channels), asked.copies)?;
    let write = match asked.writes.as_slice() {
        [one, ..] => *one,
        [] => *read,
    };
    let stopped = watch.stop.filter(|_| !found.complete);
    answer.extend([
        ("stencil", text(stencil.name())),
        ("row_order", text(stencil::ORDER)),
        ("border", text(border_name(asked.border))),
        ("runs_as", text(steps_runs(asked.border))),
        ("examples", map(vec![("cells", exact(found.cells))])),
        (
            "chip",
            map(vec![
                ("inputs", counted(inputs)),
                ("outputs", counted(1)),
                ("gates", gates_shown(&gates)),
            ]),
        ),
        ("tables", by_code(counting, code)),
        ("steps", counted(steps)),
        ("consistent", exact(found.consistent)),
        (
            "consistent_tables",
            Json::List(
                found
                    .codes
                    .iter()
                    .map(|one| by_code(counting, *one))
                    .collect(),
            ),
        ),
        ("checked", exact(found.checked)),
        ("space", exact(space)),
        ("stopped", text(stopped.map_or("complete", Stop::name))),
        ("complete", Json::Bool(found.complete)),
        (
            "iterate",
            map(vec![
                (
                    "alternate",
                    Json::List(vec![counted(read.plane), counted(write.plane)]),
                ),
                ("frames", counted(steps)),
            ]),
        ),
    ]);
    if let Some(one) = &given {
        answer.push(("places", places_shown(counting, one.preset)));
    }
    if let Some(stop) = stopped {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}

pub const LEAST: &str = "noise least_wrong or likely, and search, go with rule totalistic or outer and any steps from 1 to 8. search exhaustive tries every table, at most 18 classes, the default up to 18; search local, the default past 18, scores every table whose births (under outer) and survivals are each one interval of counts or none (under totalistic one interval or none) on a window of the example cells, starts from the best of the 16 best scored on every example cell, then flips single entries and, when no single flip lowers the score, pairs of entries with at least one reached, keeping a flip only when it lowers the score, until neither does. Tables are scored on the host, 64 at a time, by running them k steps on the cells the example cells depend on; the search reads no clock and answers the same bytes for the same request. Under noise least_wrong the score is the wrong example cells. The noise model flips each cell of both frames at a rate p; the cells the law ran on are drawn from the seen input: with d = (seen density - p) / (1 - 2p), a seen live cell was dead with chance (1 - d)p / ((1 - d)p + d(1 - p)) and a seen dead cell live with chance dp / (dp + (1 - d)(1 - p)). The answer adds search, noise, wrong, share as [wrong, cells], margins in the shape of tables (how much flipping each entry alone raises the score), pinned in the shape of tables (the entry where the data pins it, null elsewhere, judged by surprise under the noise model at the rate whatever the score: its margin is above 0, the answer's entry makes the examples at least 20 nats more likely than the flip, again under 16 other seeded draws, and by at least 4 standard errors of its sum over blocks (each example halved along each axis it spans, the error from how the gap varies between blocks), the answer is wrong on at most the expected count plus four times its square root plus 4 of the cells the flip changes, and the best table found with the other value is less likely than the best found with the answer's value by the same three tests, the tables coming from single-flip descents by surprise from the answer, from the answer with each such entry flipped and held, from the other family starts and, under likely, from the least_wrong answer; descents are local, so a far table can be missed; without wrong cells under least_wrong the score is wrong cells and an entry with a margin is pinned unless a descent finds a table with no wrong cells and the other value; with far_above no entry is pinned), unpinned and unreached (entries the answer's own k-step run uses but the data does not pin, and entries it never uses, each count and under outer centre), work units (cell steps scored), and noise_estimate: flips_per_million, the rate that explains the wrong cells of the half of the cells with the fewest, grouped by input count and the law's output, each half of the examples picking the groups for the other, with input draws from the noise model run k steps through the law on 64 seeded draws; expected_wrong, basis_cells, and far_above, true past twice expected_wrong plus four times its square root plus 16, or when the rate needed is above the least share of live or dead cells of any seen frame plus four standard errors, with warning. The estimate assumes noise on both frames, so examples with a clean input frame read as a lower rate, and treats the seen input as noisy, not clean; far_above false is no proof of a counting rule or of the table: from 2 steps on another family or a quieter table can read as a counting rule with a high rate. exhaustive adds tied, consistent, checked and space; local adds family, moves and checked. noise likely starts from a sixteenth of the least_wrong estimate and searches again by surprise: minus the log chance, in millionths of a nat, of each example cell's output, the chance of a live output being p plus (1 - 2p) times the share of 16 seeded input draws from the noise model that end live after k steps, at least one in a million; it then fits the rate by 12 golden-section steps over an eighth to eight times it, at most the rate the seen frames allow, and searches again from its answer while the fit is more than a fifth off, at most three searches. It adds assumed_flips_per_million and surprise; margins, family and moves count surprise. A least_wrong estimate of 0 answers the least_wrong table with assumed_flips_per_million 0. A brake before each round of 64 batches, each pass of flips, the margins, each step of the estimate, each round of the pin descents, each step of the rate fit and each search stops on budget or cancel; a stop at any brake answers the best table so far with stopped budget or cancelled, complete false, noise as asked and no margins, pinned, unpinned, unreached, noise_estimate or warning, and is refused budget or cancelled when nothing was scored on every example. Under noise refuse a search that ends with wrong cells is refused no_rule_k with closest and search. search exhaustive past 18 classes is refused steps_space; search, or noise least_wrong or likely, under rule minimal, symmetric or linear is refused search_rule or noise_rule; noise vote with search is refused steps_noise";

pub const LEAST_FAR: &str = "the least-wrong table is wrong on far more example cells than cell flip noise on both frames explains at the estimated rate or at any rate the seen frames allow, no entry is pinned, so the examples are not this rule with noise: check steps, border, places and the rule, or use another rule";

struct Shape<'call> {
    asked: &'call Asked,
    held: &'call Json,
    channels: u32,
    given: Option<&'call Masked>,
}

fn search_of(held: &Json) -> Result<Option<Search>, Failed> {
    match held.get("search") {
        None => Ok(None),
        Some(one) => match one.text() {
            Some("exhaustive") => Ok(Some(Search::Exhaustive)),
            Some("local") => Ok(Some(Search::Local)),
            _ => Err(unread("search")),
        },
    }
}

fn entry_shown(counting: Counting, class: u32) -> Json {
    let counts = counting.counts();
    let mut fields = vec![("count", counted(class % counts))];
    if counting.tally == Tally::Outer {
        fields.push(("centre", counted(class / counts)));
    }
    map(fields)
}

fn recounted(stepping: &crate::steps::Stepping, samples: &[Sample<'_>], table: &[u32]) -> u64 {
    let counting = stepping.counting;
    let stencil = counting.stencil;
    let read = Bit { plane: 0, bit: 0 };
    samples
        .iter()
        .map(|sample| {
            let words: &[u32] = match sample.inputs.as_slice() {
                [one, ..] => one,
                [] => &[],
            };
            let mut grid: Vec<u32> = words
                .iter()
                .map(|word| (word >> stepping.read) & 1)
                .collect();
            for _ in 0..stepping.steps {
                let whole = Sample {
                    size: sample.size,
                    inputs: vec![grid.as_slice()],
                    from: [0; 3],
                    inner: sample.size,
                    outputs: Vec::new(),
                };
                let mut next = Vec::with_capacity(grid.len());
                for z in 0..sample.size[2] {
                    for y in 0..sample.size[1] {
                        for x in 0..sample.size[0] {
                            let row = stencil::row_at(
                                stencil,
                                std::slice::from_ref(&read),
                                &whole,
                                [x, y, z],
                                stepping.border,
                            );
                            let class = counting.class(row) as usize;
                            next.push(table.get(class).copied().unwrap_or(0));
                        }
                    }
                }
                grid = next;
            }
            let wanted: &[u32] = match sample.outputs.as_slice() {
                [one, ..] => one,
                [] => &[],
            };
            let [wide, deep, tall] = sample.inner;
            let mut wrong = 0_u64;
            for z in 0..tall {
                for y in 0..deep {
                    for x in 0..wide {
                        let inside = [sample.from[0] + x, sample.from[1] + y, sample.from[2] + z];
                        let got = grid
                            .get(stencil::index(sample.size, inside))
                            .copied()
                            .unwrap_or(0);
                        let want = wanted
                            .get(stencil::index(sample.inner, [x, y, z]))
                            .map_or(0, |word| (word >> stepping.write) & 1);
                        wrong += u64::from(got != want);
                    }
                }
            }
            wrong
        })
        .sum()
}

fn stop_refused(stop: Stop, message: String, told: Vec<(&'static str, Json)>) -> Failed {
    Box::new(Told(
        Refusal::new(
            stop.code(),
            if stop == Stop::Budget {
                "$.budget_ms"
            } else {
                "$"
            },
            message,
        ),
        told,
    ))
}

fn moves_shown(
    counting: Counting,
    learned: &crate::steps::least::Learned,
    unit: &'static str,
) -> Json {
    Json::List(
        learned
            .moves
            .iter()
            .map(|one| {
                map(vec![
                    (
                        "flipped",
                        Json::List(
                            one.flipped
                                .iter()
                                .map(|class| entry_shown(counting, *class))
                                .collect(),
                        ),
                    ),
                    (unit, exact(one.score)),
                ])
            })
            .collect(),
    )
}

fn doubt_shown(
    counting: Counting,
    learned: &crate::steps::least::Learned,
    answer: &mut Vec<(&'static str, Json)>,
) {
    let Some(doubt) = &learned.doubt else {
        return;
    };
    let classes = counting.classes();
    let margin = |class: u32| doubt.margins.get(class as usize).copied().unwrap_or(0);
    let used = |class: u32| doubt.used.get(class as usize).copied().unwrap_or(0);
    let pinned = |class: u32| doubt.pinned.get(class as usize).copied().unwrap_or(false);
    answer.push((
        "margins",
        Json::List(vec![by_count(counting, &|class| {
            Some(exact(margin(class)))
        })]),
    ));
    answer.push((
        "pinned",
        Json::List(vec![by_count(counting, &|class| {
            Some(if pinned(class) {
                counted(crate::steps::entry(learned.code, class))
            } else {
                Json::Null
            })
        })]),
    ));
    answer.push((
        "unpinned",
        Json::List(
            (0..classes)
                .filter(|class| !pinned(*class) && used(*class) > 0)
                .map(|class| entry_shown(counting, class))
                .collect(),
        ),
    ));
    answer.push((
        "unreached",
        Json::List(
            (0..classes)
                .filter(|class| used(*class) == 0)
                .map(|class| entry_shown(counting, class))
                .collect(),
        ),
    ));
}

fn least_law(
    stepping: &crate::steps::Stepping,
    samples: &[Sample<'_>],
    watch: &mut Watch<'_>,
    (mode, least, likely): (Search, bool, bool),
    shape: &Shape<'_>,
) -> Result<Json, Failed> {
    let counting = stepping.counting;
    let stencil = counting.stencil;
    let steps = stepping.steps;
    let classes = stepping.classes();
    let space = stepping.space();
    let asked = shape.asked;
    let learned = {
        let mut halted = || watch.braked(Point::Search);
        match (mode, likely) {
            (_, true) => crate::steps::least::likely(stepping, samples, mode, &mut halted),
            (Search::Exhaustive, false) => {
                crate::steps::least::exhaustive(stepping, samples, least, &mut halted)
            }
            (Search::Local, false) => {
                crate::steps::least::local(stepping, samples, least, &mut halted)
            }
        }
    };
    let Some(learned) = learned else {
        return Err(
            "a search thread of the host failed; this is a fault of the server"
                .to_owned()
                .into(),
        );
    };
    let Some(learned) = learned else {
        let stop = watch.stop.unwrap_or(Stop::Budget);
        return Err(stop_refused(
            stop,
            format!(
                "the {} search stopped on {} before it scored a table on every example; nothing was learned",
                mode.name(),
                stop.name()
            ),
            vec![("search", text(mode.name())), ("steps", counted(steps))],
        ));
    };
    let table = crate::steps::table_of(learned.code, classes);
    if recounted(stepping, samples, &table) != learned.wrong {
        return Err(format!(
            "the {} search counts {} wrong cells for table {}, and running it again on the host counts otherwise; this is a fault of the server",
            mode.name(),
            learned.wrong,
            learned.code
        )
        .into());
    }
    let stopped = watch.stop.filter(|_| !learned.complete);
    let mut searched = vec![
        ("search", text(mode.name())),
        ("checked", exact(learned.checked)),
    ];
    if mode == Search::Exhaustive {
        searched.push(("space", exact(space)));
    }
    searched.push(("steps", counted(steps)));
    if !least && learned.wrong > 0 {
        if let Some(stop) = stopped {
            return Err(stop_refused(
                stop,
                format!(
                    "the {} search stopped on {} after {} tables, before it found a table that applied {steps} times gives every example; nothing was learned",
                    mode.name(),
                    stop.name(),
                    learned.checked
                ),
                searched,
            ));
        }
        let mut told = vec![
            (
                "closest",
                map(vec![
                    ("tables", by_code(counting, learned.code)),
                    ("wrong", exact(learned.wrong)),
                ]),
            ),
            ("cells", exact(learned.cells)),
        ];
        told.extend(searched);
        return Err(Box::new(Told(
            Refusal::new(
                Code::NoRuleK,
                "$.examples",
                format!(
                    "the {} search found no table of rule {} that applied {steps} times gives every example; the closest it found is wrong on {} of {} example cells; a local search is no proof that none exists, and noise least_wrong answers the table with the fewest wrong cells",
                    mode.name(),
                    counting.tally.name(),
                    learned.wrong,
                    learned.cells
                ),
            ),
            told,
        )));
    }
    let inputs = stencil.positions();
    let gates = stencil::counted_gates(counting, 1, &table);
    if inputs <= TABLE_INPUTS {
        for row in 0..1_u32 << inputs {
            let wanted = table
                .get(counting.class(row) as usize)
                .copied()
                .unwrap_or(0);
            if stencil::answer(inputs, 1, &gates, row) != wanted {
                return Err(format!(
                    "the counting circuit answers row {row} other than the table; this is a fault of the server"
                )
                .into());
            }
        }
    }
    let circuit = Circuit {
        state: 0,
        inputs,
        outputs: 1,
        gates: gates.clone(),
    };
    let mut answer = super::law::chip_law(
        circuit,
        wiring(asked, shape.held, shape.channels),
        asked.copies,
    )?;
    let (read, write) = match (asked.reads.as_slice(), asked.writes.as_slice()) {
        ([one, ..], [two, ..]) => (*one, *two),
        _ => {
            return Err(
                "a k-step law has one read and one write; this is a fault of the server"
                    .to_owned()
                    .into(),
            );
        }
    };
    answer.extend([
        ("stencil", text(stencil.name())),
        ("row_order", text(stencil::ORDER)),
        ("border", text(border_name(asked.border))),
        ("runs_as", text(steps_runs(asked.border))),
        ("examples", map(vec![("cells", exact(learned.cells))])),
        (
            "chip",
            map(vec![
                ("inputs", counted(inputs)),
                ("outputs", counted(1)),
                ("gates", gates_shown(&gates)),
            ]),
        ),
        ("tables", by_code(counting, learned.code)),
        ("steps", counted(steps)),
        ("search", text(mode.name())),
        (
            "noise",
            text(match (least, likely) {
                (false, _) => "refuse",
                (true, true) => "likely",
                (true, false) => "least_wrong",
            }),
        ),
        ("wrong", exact(learned.wrong)),
        (
            "share",
            Json::List(vec![exact(learned.wrong), exact(learned.cells)]),
        ),
    ]);
    let unit = if learned.rate.is_some() {
        "surprise"
    } else {
        "wrong"
    };
    if let Some(rate) = learned.rate {
        answer.extend([
            ("assumed_flips_per_million", exact(rate)),
            ("surprise", exact(learned.score)),
        ]);
    } else if likely && learned.complete {
        answer.push(("assumed_flips_per_million", exact(0)));
    }
    match mode {
        Search::Exhaustive => answer.extend([
            ("tied", exact(learned.tied)),
            (
                "consistent",
                exact(if learned.wrong == 0 { learned.tied } else { 0 }),
            ),
            ("checked", exact(learned.checked)),
            ("space", exact(space)),
        ]),
        Search::Local => {
            if let Some((code, score)) = learned.family {
                answer.push((
                    "family",
                    map(vec![
                        ("tables", by_code(counting, code)),
                        (unit, exact(score)),
                    ]),
                ));
            }
            answer.push(("moves", moves_shown(counting, &learned, unit)));
            answer.push(("checked", exact(learned.checked)));
        }
    }
    if learned.complete {
        doubt_shown(counting, &learned, &mut answer);
    }
    if let Some(noise) = learned.noise.filter(|_| learned.complete) {
        answer.push((
            "noise_estimate",
            map(vec![
                ("flips_per_million", exact(noise.per_million)),
                ("expected_wrong", exact(noise.expected)),
                ("basis_cells", exact(noise.basis)),
                ("far_above", Json::Bool(noise.far_above)),
            ]),
        ));
        if noise.far_above {
            answer.push(("warning", text(LEAST_FAR)));
        }
    }
    answer.extend([
        ("work", map(vec![("units", exact(learned.units))])),
        ("stopped", text(stopped.map_or("complete", Stop::name))),
        ("complete", Json::Bool(learned.complete)),
        (
            "iterate",
            map(vec![
                (
                    "alternate",
                    Json::List(vec![counted(read.plane), counted(write.plane)]),
                ),
                ("frames", counted(steps)),
            ]),
        ),
    ]);
    if let Some(one) = shape.given {
        answer.push(("places", places_shown(counting, one.preset)));
    }
    if let Some(stop) = stopped {
        answer.push(("hint", text(stop.code().hint())));
    }
    Ok(map(answer))
}
