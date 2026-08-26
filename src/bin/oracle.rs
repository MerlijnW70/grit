//! `cargo run --release --bin oracle -- exam.txt` — write the exam, and the card writes the logic.
//!
//! # What changes here
//!
//! `src/bin/chip.rs` evolves a grid of logic gates against a task written into the file. This is the
//! same grid with the task taken out of it: what the circuit has to do arrives as text, at runtime,
//! and the kernels are built around it. `simdr` emits SPIR-V from Rust, so the number of inputs is
//! an argument rather than a constant — which is the whole of what makes this a tool instead of an
//! experiment.
//!
//! You do not say how. You say what.
//!
//! ```text
//!   # a comment
//!   inputs 4        how many inputs, one to six
//!   outputs 3       how many answer bits — one if left out
//!   gates and or not   the gates it draws from; wire is always in
//!   noise 16        break one gate in sixteen each generation
//!   lean 1          among equally right chips, prefer the one with fewer gates
//!   steps 4         cut the word into four, one piece a step — time
//!   state 2         rows wired from the last column back to the first — memory
//!   colony 65536    how many chips search at once — the way out of a local optimum
//!   island 256      how many of them share an arena — the colony if left out
//!   seed 1234       where the random grids start, which decides more than it looks
//!   parity          fill the table: the parity of the inputs
//!   majority        fill the table: more ones than noughts
//!   equal           fill the table: every input the same
//!   count           fill the table: how many ones there are, in binary
//!   add             fill the table: the first half plus the second
//!   10-1 -> 01      the inputs that pattern stands for, and what they answer
//!   ---- -> --1     a dash on the left is every value; a dash on the right is not asked
//! ```
//!
//! **A dash on the left is both.** `10-1` is two inputs at once and `----` is all sixteen, so one
//! line says what otherwise takes a table.
//!
//! **A dash on the right is silence.** The score counts answer *bits*, not cases, and a bit nobody
//! named counts for nothing either way — so `1--- -> 1-` asks about one output and leaves the other
//! alone, and the report says `16 answer bits marked of 32` rather than pretending to judge 32.
//!
//! **The answer comes out down the right-hand edge**, output nought on the top wire of the last
//! column and the rest under it, and the drawing marks them.
//!
//! What is left out is left out rather than assumed, which is what makes a partial exam a partial
//! score instead of a wrong one.
//!
//! # What comes back
//!
//! A grid of gates, drawn, and a number: how many of the marked cases it gets right. That number is
//! worked out twice, once by the card while it was evolving and once by the host afterwards from the
//! genome alone, and the report prints both. They have to agree.
//!
//! # Making it work under faults, which costs more than it buys
//!
//! Measuring a fault afterwards says what a circuit happens to survive. `noise 32` breaks one gate
//! in thirty-two while it evolves, so surviving is something to be selected for. Which gates are
//! broken comes from the tick and the cell — the same fault for every chip, so nobody is judged
//! against an easier grid — and it holds for `SETTLED` generations, because a score measured under
//! one pattern and compared under another selects for having been lucky. Drawing a fresh pattern
//! every generation did exactly that: it took eight starts in eight down to none.
//!
//! Held still, parity of four bits, eight starts, with the survey run afterwards on a clean grid:
//!
//! ```text
//!   noise    right     faults survived
//!       0    8 of 8    118 of 348   33%
//!      32    5 of 8    139 of 354   39%
//!      16    2 of 8    118 of 322   36%
//! ```
//!
//! **Six points of robustness for three of the eight answers.** It is a real effect and a poor
//! trade, and more noise is worse at both. Fault tolerance here is something to be paid for, not
//! something that emerges.
//!
//! # What the gate set decides, which is not what is expressible
//!
//! `gates` says which kinds the search draws from — the starting grids as well as the mutations,
//! because restricting one and not the other is not a restriction. Eight starts:
//!
//! ```text
//!                        every gate   xor only   and or not
//!   parity of four          8 of 8     8 of 8      0 of 8
//!   detect 1110             8 of 8     0 of 8      0 of 8
//! ```
//!
//! **And, or and not is functionally complete**: every one of these answers exists in that set, and
//! parity is a textbook construction in it. The search never arrives — 9 of 16 against a chance of
//! 8. Exclusive-or alone finds parity every start and never finds an *and* of three inputs.
//!
//! So the gate set decides what is findable rather than what is expressible, and which restriction
//! hurts depends on the task rather than on how complete the set is.
//!
//! # Reading the memory
//!
//! The state rows are the only thing crossing from one step to the next, so what a chip remembers is
//! in them and nowhere else. The report writes them down after every step. Parity of four bits, one
//! bit at a time, one row of state:
//!
//! ```text
//!   0000   state 1 -> 0 -> 1 -> 0
//!   1000   state 0 -> 1 -> 0 -> 1
//!   0110   state 1 -> 1 -> 1 -> 0
//!   1110   state 0 -> 0 -> 0 -> 1
//! ```
//!
//! **It is a parity counter and it is not the one anybody would write.** All-noughts goes 1, 0, 1, 0
//! — the state turns over every step whether or not it was shown a one. What it holds is the parity
//! so far *exclusive-or the step number*, and the gates that read it undo the alternation. Every row
//! above fits that and so does every row not shown.
//!
//! Nothing asked for a tidy encoding, so nothing produced one. What the state means has to be read
//! off the machine rather than assumed from what it is for.
//!
//! # Opening the box, pressing on it, and breaking it
//!
//! The report walks back from the wires the answer is read off — and from the state rows, which are
//! outputs into the next step — and marks every cell the answer can have come through. What is not
//! marked is drawn as a dot, and there is a great deal of it:
//!
//! ```text
//!     a !v!!!·^·v·^ <- 0
//!     b ^^·v·&·!·&·
//!     c ^·^·v!··^··
//!     d v^·v···v···
//!     0 ^&^·^······
//! ```
//!
//! Then every marked gate is turned into a wire in turn and the answer re-scored, which is the
//! cheapest fault there is and the one a dead transistor most resembles.
//!
//! **Two counts, and they are not the same count.** Reachable is structure: the answer *could* come
//! through here. Costly is function: breaking it *does* change the answer. On the parity of four
//! bits, eight starts:
//!
//! ```text
//!            reachable   costly   slack
//!   lean 0        43.5     28.8    14.8
//!   lean 1        32.0     23.5     8.5
//!   the leanest     13       13       0
//! ```
//!
//! `lean 1` scores `correct × (CELLS + 1) + (CELLS − used)`, so between two chips that are equally
//! right the one with fewer gates wins. It works, and it varies: the best run under pressure found
//! thirteen gates where the best without found thirty-three, and a hand-written parity of four bits
//! is three exclusive-ors. So it is smaller and it is not small.
//!
//! **There is no self-healing, and the numbers say what there is instead.** A gate that costs
//! nothing to break was reachable and masked — never doing anything — rather than a spare standing
//! by. Break one that matters and the answer breaks, every time, in every run. Pressure takes the
//! slack out and the leanest circuit has none at all: thirteen gates, thirteen of them costly.
//!
//! So minimisation and surviving a fault pull against each other here, and pressing on the gate
//! count presses on both.
//!
//! # The exam is the specification, and a partial one stays partial
//!
//! **A circuit is judged on the bits you marked and is free on the rest**, so what you get back is
//! the cheapest thing that fits what you said. That is not a flaw and it is not obvious, so the
//! report prints the whole truth table of the winner — every input, not only the asked ones.
//!
//! Take a four-bit vocabulary where `0000` and `0001` are safe and `1110` and `1111` are dangerous,
//! and ask only about those:
//!
//! ```text
//!   000- -> 0
//!   111- -> 1
//! ```
//!
//! It scores 4 of 4 inside twenty generations, and the table shows why: the answer is bit nought,
//! for all sixteen inputs. Every safe word begins with a nought and every dangerous one with a one,
//! so a wire passes the exam. The six neutral words that begin with a one set off the alarm too.
//!
//! Scatter them so no single bit separates safe from dangerous — `1100` and `0011` safe, `1010` and
//! `0101` dangerous — and it scores 4 of 4 again, on `xnor(a, c)`. Two of the four inputs, one gate,
//! and `b` and `d` are not read at all. Four constraints out of sixteen leave twelve free, and the
//! search spends none of its effort on what was not asked.
//!
//! Mark all sixteen — `parity`, say — and it scores 16 of 16 on a circuit that reads every input,
//! because now there is nowhere cheaper to go.
//!
//! # Making an exam watertight
//!
//! Say what everything does, and then say the exceptions. A dash pattern marks every input, and a
//! later line overrides what an earlier one said about the bits it names:
//!
//! ```text
//!   ---- -> 0       no word raises the alarm
//!   1110 -> 1       except this one
//!   1111 -> 1       and this one
//! ```
//!
//! Now all sixteen are marked and the circuit has nowhere to hide. It comes back at 16 of 16 from
//! every start, silent on the six neutral words beginning with a one that the loose version set off.
//!
//! **What it costs is that the answer has to be buildable.** The two dangerous words above share
//! three bits, so the circuit is `a and b and c` and the search finds it every time. Move them apart
//! — `1010` and `0101`, which share no bit at all — and the same watertight exam comes back at 15 of
//! 16 from eight starts out of eight: it builds a detector for one of the two and stays silent on
//! the other. One detector scores 15 and two score 16, and nothing between them scores 15 and a half.
//!
//! # Time, and the only memory there is
//!
//! `steps 4` cuts the input word into four and hands one piece over a step, so a chip that wants
//! the earlier pieces has to have kept them. `state 1` is what keeps: one row, wired from the last
//! column back to the first, carried across the seam between steps. Everything else in the first
//! column is put back to what the exam and the padding say, every step.
//!
//! The parity of four bits is the case for it, because a chip that sees one bit at a time and
//! remembers nothing can only answer about the last one. Five starts each:
//!
//! ```text
//!   steps   state   of 16               passes    a generation
//!       1       0   16 16 16 16 16          16          92 µs   all four bits at once
//!       4       0    8  8  8  8  8          52         330 µs   one at a time, no memory
//!       4       1   16 16 16 16 16          52         330 µs   one at a time, one row
//!       4       2   16 16 16 16 16
//! ```
//!
//! **Eight of sixteen is chance**, and it is what an answer that depends on the last bit alone
//! scores against parity. One row of memory takes it to sixteen from every start, and a second buys
//! nothing: the chip has evolved a thing that turns over on every one it is shown, which is the
//! whole of a parity counter.
//!
//! Four steps is four passes through the grid rather than one, so the frame is 52 stages instead of
//! 16 and a generation costs three and a half times as much. Memory itself is free — `state 1` and
//! `state 0` cost the same, because the row was always there and this only says where it comes from.
//!
//! **Chips are not wired to each other**, and that is a decision rather than an omission: coupling
//! them is a larger grid with a block in it, and the sweep above already says what more area buys.
//! The one coupling here is a chip to itself, across time.
//!
//! # How much memory a task needs, measured against what it should need
//!
//! A chip that has to answer about a sequence needs enough state to be a machine with as many
//! states as the sequence has positions to be in. Three tasks, five starts each, and the row the
//! answer needs against the row the arithmetic says:
//!
//! ```text
//!   task                          machine   needs   0    1    2    3    4
//!   parity, 4 steps of 1 bit     2 states     1     -    5/5  -    -    -
//!   detect 1011, 4 steps of 1    5 states     3     0/5  0/5  1/5  5/5  5/5
//!   detect 101101, 3 steps of 2  4 states     2     0/5  0/5  2/5  5/5  5/5
//! ```
//!
//! **The arithmetic is right about the floor and one row short of reliable.** Parity is a thing that
//! turns over, which is one bit, and one row does it every start. A four-symbol pattern is a
//! five-state machine, which is three bits, and three rows do it every start while two do it once in
//! five. A three-symbol pattern over a two-bit alphabet is a four-state machine, which is two bits —
//! and two rows manage it two starts in five where three manage it every time.
//!
//! Nothing makes the search pack its states densely, so the floor is where it starts working and one
//! row above is where it stops being luck.
//!
//! # Width and time together
//!
//! The last row above is both at once: six inputs cut into three steps of two, so each step is a
//! comparison against a two-bit symbol *and* a move along the pattern. It is harder than the
//! four-by-one that has more steps and fewer bits, and not because of the state:
//!
//! ```text
//!   colony   detect 101101, state 3
//!    4 096   1 of 5
//!   16 384   2 of 5
//!   65 536   5 of 5
//! ```
//!
//! The colony is the lever here as it was on the flat exams, and it costs what it costs: 40 stages
//! and 11 ms a generation at 65 536 chips against 40 stages and under a millisecond at 4 096.
//!
//! # Getting out of a local optimum
//!
//! That 15 is a trap rather than a slow start, and three levers were measured against it, eight
//! starts each.
//!
//! **Room helps once.** More columns give the two detectors somewhere to stand side by side:
//!
//! ```text
//!    8x12    88 gates   0 of 8
//!    8x24   184 gates   3 of 8
//!    8x36   280 gates   2 of 8
//!    8x48   376 gates   2 of 8
//! ```
//!
//! **Patience buys nothing at all.** Ten times the generations — forty thousand against four —
//! gives the same eight answers from the same eight starts. A lineage that is trapped is trapped,
//! and waiting is not what gets it out.
//!
//! **The colony is the lever.** It is many independent hill-climbs, so what a trapped one cannot do
//! with more time, more of them can do at once:
//!
//! ```text
//!    grid    colony   solved   a generation
//!    8x12     4 096    0 of 8        160 µs
//!    8x12    16 384    6 of 8        557 µs
//!    8x24     4 096    3 of 8
//!    8x24    16 384    6 of 8
//!    8x24    65 536    8 of 8      2 261 µs
//! ```
//!
//! # Arenas, which turned out not to matter
//!
//! If a bigger colony helps because it is more independent climbs, then splitting one colony into
//! arenas that barely talk should help at the same size. `island` says how many chips share one,
//! and every `MIGRATE` ticks the partner is drawn from the whole colony instead so a find can
//! travel. It does nothing:
//!
//! ```text
//!   colony    island   solved
//!    4 096     4 096   0 of 8      one arena, which is what this was before
//!    4 096     1 024   0 of 8
//!    4 096       256   0 of 8
//!    4 096        64   0 of 8
//!    4 096   256, no migration at all   0 of 8
//!
//!   16 384    16 384   6 of 8
//!   16 384     4 096   6 of 8
//!   16 384     1 024   5 of 8
//!   16 384       256   6 of 8
//! ```
//!
//! **So the colony helps by how many chips there are and not by how they are arranged.** Sixty-four
//! arenas of sixty-four search no better than one arena of four thousand, and cutting the migration
//! entirely changes nothing either. Whatever a bigger colony is buying, it is not the diversity that
//! isolation preserves.
//!
//! The knob stays because these numbers are the finding, and a finding is worth retesting on the
//! next exam rather than believing about it.
//!
//! Four times the colony on the narrow grid beats twice the depth, and sixteen times solves it
//! every start. It costs fourteen times as much a generation, which is the trade stated rather
//! than hidden — and `colony` is a line in the exam, so it is the writer's to make.
//!//! **So a circuit that answers your four questions has not learnt what they meant.** It has learnt
//! the least thing that answers them, which is what you asked for. The truth table is where the
//! difference shows.
//!
//! # What this is not
//!
//! # What it does, measured
//!
//! Four exams, 4096 chips, at most four thousand generations, on an RTX 4080:
//!
//! ```text
//!   exam                                    marked   three starts
//!   inputs 4, parity                            16    16  16  16
//!   inputs 4, outputs 2, one output = input a   16    16  16  16
//!   inputs 4, outputs 3, add                    48    40  40  41
//!   inputs 4, outputs 3, count                  48    41  43  43
//! ```
//!
//! The first two arrive inside twenty generations, every start. The last two do not arrive at all,
//! and the score is what it is: 43 of 48 is a chip right about most of the bits of most of the
//! answers, which the host confirms from the genome afterwards.
//!
//! # The grid, measured
//!
//! `ROWS` and `COLUMNS` say how much room a chip has. `count` again, four starts a size:
//!
//! ```text
//!   grid    gates   four starts        a generation
//!    8x8       56   41  39  39  40           51 us
//!    8x12      88   41  43  43  43           78 us
//!    8x16     120   43  41  42  43           99 us
//!    8x24     184   43  40  43  43          138 us
//!   16x12     176   39  41  40  41          133 us
//!   16x24     368   40  43  43  41          256 us
//! ```
//!
//! **Fifty-six gates is too few and eighty-eight is enough.** Past that the spreads sit on top of
//! one another and more room does not raise the ceiling — 368 gates reach what 88 reach and cost
//! three times as much. Sixteen rows are worse than eight at the same width, which is the diagonal
//! wiring having further to travel through rows holding nothing. So the shape is 8 by 12, and what
//! stops `count` at 43 is not the area.
//!
//! `ROWS` has to be a power of two: the invocation index is taken apart with shifts.
//!
//! # What this is not
//!
//! **It is not an oracle that always answers.** `src/bin/chip.rs` measured where the same search
//! stops: parity of three and four inputs it solves exactly, five it gets to 30 of 32, six to 56 of
//! 64 — and four different selection schemes all stopped at that 56. Parity is the hard case in
//! evolving circuits and nothing here changes that. What this does honestly is tell you the score
//! rather than claim one.
//!
//! **What it finds is not compact.** Four-input parity is three exclusive-ors written by hand; what
//! comes out of here is 88 gates of which most do nothing, and the drawing shows that plainly. It is
//! smaller than nothing and larger than anything a person would write.
//!
//! **No derivative is computed**, which is true and is the interesting part. There is no gradient,
//! no loss surface and no training: 4096 grids are drawn at random, the ones that lose are copied
//! over and one gate is turned, and the card runs a few thousand generations of that in a second.
//! `decisions/DR-0009` is why the pass that does the running is dispatched over what changed.
//!
//! Nothing else in the repository refers to this file. `rm src/bin/oracle.rs` removes it whole.

use std::time::Duration;

use grit::{Groups, Host};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};
use simdr::module::Id;

/// How many rows the chip has: the inputs, and a nought and a one to build with.
const ROWS: u32 = 8;
/// How many columns, of which the first is the inputs.
const COLUMNS: u32 = 12;
/// How many gates a chip holds, which is the length of a genome.
const CELLS: u32 = (COLUMNS - 1) * ROWS;
/// How many generations one fault pattern stands before another is drawn. A power of two.
const SETTLED: u32 = 64;

/// How many ticks between one migration and the next. A power of two.
const MIGRATE: u32 = 64;

/// How many kinds of gate there are.
const KINDS: u32 = 5;

/// How many gates a mutation turns over at once.
const MUTATIONS: u32 = 1;

/// How many generations to run.
const GENERATIONS: u32 = 4_000;
/// How many go into one submission.
const BATCH: u32 = 20;

/// How many words every kernel declares.
const BINDINGS: u32 = 9;

/// Four words: the tick and the parity.
const PARAMS: u32 = 0;
/// [`CELLS`] words a chip: what kind of gate sits in each.
const GENE: u32 = 1;
/// One word an input: what the answer should be. As many as the exam has inputs.
const TARGET: u32 = 2;
/// Every wire of every chip: `((chip × COLUMNS + column) × ROWS + row) × CASES + case`.
const WIRE: u32 = 3;
/// One word a chip: how many of the marked inputs it gets right.
const SCORE: u32 = 4;
/// One word a chip: its own pseudo-random state.
const SEED: u32 = 5;
/// The chips that changed this generation, which are the only ones worth running.
const CHANGED: u32 = 6;
/// One word an input: whether the exam asks about it at all.
const KEEP: u32 = 8;

/// Seven words: how many chips changed, then two workgroup counts of three words — one for a pass
/// over every wire of them, one for a pass over the chips themselves.
const COUNTS: u32 = 7;
// ---------- arithmetic the lane API has no operator for ----------

/// This invocation's organism, or its slot in a list: workgroup × width + lane.
fn mine(kernel: &mut Kernel<U32>, subgroup: u32) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(subgroup)?;
    let along = kernel.module().i_mul(uint, group, width)?;
    Ok(kernel.module().i_add(uint, along, lane)?)
}

/// `held` less `off`, floored at nought.
///
/// **There is no subtraction in the lane API.** Floor the value at what is coming off, then add the
/// two's complement — the shape `src/sand.rs` uses, for the reason `DESIGN.md` gives.
fn minus(kernel: &mut Kernel<U32>, held: Id, off: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let take = lanes.splat_id::<U32, 1>(off)?;
    let floored = lanes.max(now, take)?;
    let ones = lanes.splat_bits::<U32, 1>(u32::MAX)?;
    let back = lanes.mul(take, ones)?;
    Ok(lanes.add(floored, back)?.id())
}

/// The low `bits` of `held`: a modulo by a power of two, done in shifts.
fn low_bits(kernel: &mut Kernel<U32>, held: Id, bits: u32) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let by = lanes.splat_bits::<U32, 1>(bits)?;
    let up = lanes.shift_right_logical(now, by)?;
    let down = lanes.shift_left(up, by)?;
    let ones = lanes.splat_bits::<U32, 1>(u32::MAX)?;
    let back = lanes.mul(down, ones)?;
    Ok(lanes.add(now, back)?.id())
}

/// `held`, but no larger than `ceiling`.
fn at_most(kernel: &mut Kernel<U32>, held: Id, ceiling: u32) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let top = lanes.splat_bits::<U32, 1>(ceiling)?;
    Ok(lanes.min(now, top)?.id())
}

/// One step of a linear congruential generator, which wraps because unsigned multiply does.
fn stir(kernel: &mut Kernel<U32>, seed: Id) -> Result<Id, LaneError> {
    let uint = kernel.element();
    let by = kernel.module().constant_u32(1_664_525)?;
    let plus = kernel.module().constant_u32(1_013_904_223)?;
    let spun = kernel.module().i_mul(uint, seed, by)?;
    Ok(kernel.module().i_add(uint, spun, plus)?)
}

/// Which chip this invocation is acting on, and `COLONY` — the dump — where it is acting on none.
///
/// **The dump is the whole of the bounds check.** A dispatch is rounded up to whole workgroups, so
/// the last one holds lanes past the end of the list; they read a clamped slot, write into the chip
/// at `COLONY`, and nothing reads it. `crate::compact` calls the same slot a dump.
fn listed(kernel: &mut Kernel<U32>, slot: Id, held: Id, colony: u32) -> Result<Id, LaneError> {
    let safe = at_most(kernel, slot, colony - 1)?;
    let who = kernel.load_at(CHANGED, safe)?;

    let mut lanes = kernel.lanes()?;
    let index = lanes.splat_id::<U32, 1>(slot)?;
    let length = lanes.splat_id::<U32, 1>(held)?;
    let inside = lanes.greater_than(length, index)?;
    let real = lanes.splat_id::<U32, 1>(who)?;
    let dump = lanes.splat_bits::<U32, 1>(colony)?;
    Ok(lanes.select(inside, real, dump)?.id())
}

/// `held` shifted down by however much `by` says.
fn shift_down(kernel: &mut Kernel<U32>, held: Id, by: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let now = lanes.splat_id::<U32, 1>(held)?;
    let much = lanes.splat_id::<U32, 1>(by)?;
    Ok(lanes.shift_right_logical(now, much)?.id())
}

/// `held` shifted down by a constant.
fn down_by(kernel: &mut Kernel<U32>, held: Id, by: u32) -> Result<Id, LaneError> {
    let much = kernel.module().constant_u32(by)?;
    shift_down(kernel, held, much)
}

/// Bit `at` of `held`.
fn bit(kernel: &mut Kernel<U32>, held: Id, at: u32) -> Result<Id, LaneError> {
    let shifted = down_by(kernel, held, at)?;
    low_bits(kernel, shifted, 1)
}

/// One where `a` is above `b`, nought where it is not.
fn above(kernel: &mut Kernel<U32>, a: Id, b: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let x = lanes.splat_id::<U32, 1>(a)?;
    let y = lanes.splat_id::<U32, 1>(b)?;
    let hit = lanes.greater_than(x, y)?;
    let yes = lanes.splat_bits::<U32, 1>(1)?;
    let no = lanes.splat_bits::<U32, 1>(0)?;
    Ok(lanes.select(hit, yes, no)?.id())
}

/// One where `held` equals `want`, nought where it does not.
fn is(kernel: &mut Kernel<U32>, held: Id, want: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let x = lanes.splat_id::<U32, 1>(held)?;
    let y = lanes.splat_id::<U32, 1>(want)?;
    let hit = lanes.equal(x, y)?;
    let yes = lanes.splat_bits::<U32, 1>(1)?;
    let no = lanes.splat_bits::<U32, 1>(0)?;
    Ok(lanes.select(hit, yes, no)?.id())
}

/// `yes` where `flag` is not nought, `no` where it is.
fn pick(kernel: &mut Kernel<U32>, flag: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let f = lanes.splat_id::<U32, 1>(flag)?;
    let none = lanes.splat_bits::<U32, 1>(0)?;
    let on = lanes.greater_than(f, none)?;
    let a = lanes.splat_id::<U32, 1>(yes)?;
    let b = lanes.splat_id::<U32, 1>(no)?;
    Ok(lanes.select(on, a, b)?.id())
}



// ---------- the kernels ----------

/// Advance the tick and the parity, and clear the list of chips that changed.
fn advance(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;

    let at_tick = kernel.module().constant_u32(0)?;
    let now = kernel.load_at(PARAMS, at_tick)?;
    let ticked = kernel.module().i_add(uint, now, one)?;
    kernel.store_at(PARAMS, at_tick, ticked)?;

    let at_parity = kernel.module().constant_u32(1)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let bumped = kernel.module().i_add(uint, parity, one)?;
    let flipped = low_bits(&mut kernel, bumped, 1)?;
    kernel.store_at(PARAMS, at_parity, flipped)?;

    kernel.store_at(COUNTS, none, none)?;
    kernel.finish()
}

/// Take the neighbour's chip where the neighbour did better, change one gate, and say so.
///
/// **Half the colony at a time**, as the other two demonstrations do and for the same reason: a
/// genome is read while others are written, so the ones being read from stand still.
fn select(
    subgroup: u32,
    colony: u32,
    island: u32,
    allowed: &[u32],
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let who = mine(&mut kernel, subgroup)?;
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;

    let at_parity = kernel.module().constant_u32(1)?;
    let parity = kernel.load_at(PARAMS, at_parity)?;
    let turn = low_bits(&mut kernel, who, 1)?;
    let my_turn = is(&mut kernel, turn, parity)?;

    // **A different partner every generation, and always one of the other parity.** Measuring
    // against the chip next door only is an island of two: a good genome never travels and a bad
    // one loses to the same opponent for ever. The stride comes from the tick and is odd, so the
    // partner is the parity that is standing still this turn.
    //
    // **And it stays inside the island.** `island` is how many chips share an arena; the colony is
    // that many arenas side by side in the same buffer, and a genome crosses from one to the next
    // only on a migration tick. An island the size of the colony is one arena, which is what this
    // did before there were any.
    //
    // The parity guarantee survives the split: an island is a power of two, so wrapping inside one
    // keeps the parity that an odd stride flipped.
    let at_tick = kernel.module().constant_u32(0)?;
    let tick = kernel.load_at(PARAMS, at_tick)?;
    let spread = low_bits(&mut kernel, tick, island.trailing_zeros())?;
    let two = kernel.module().constant_u32(2)?;
    let doubled = kernel.module().i_mul(uint, spread, two)?;
    let stride = kernel.module().i_add(uint, doubled, one)?;
    let raised = kernel.module().i_add(uint, who, stride)?;
    let inside = low_bits(&mut kernel, who, island.trailing_zeros())?;
    let shore = minus(&mut kernel, who, inside)?;
    let walked = low_bits(&mut kernel, raised, island.trailing_zeros())?;
    let near = kernel.module().i_add(uint, shore, walked)?;
    // Every `MIGRATE` ticks the partner is drawn from the whole colony instead, so an island that
    // found something can hand it on and one that is stuck can be handed something.
    let far = low_bits(&mut kernel, raised, colony.trailing_zeros())?;
    let season = low_bits(&mut kernel, tick, MIGRATE.trailing_zeros())?;
    let none_yet = kernel.module().constant_u32(0)?;
    let crossing = is(&mut kernel, season, none_yet)?;
    let neighbour = pick(&mut kernel, crossing, far, near)?;
    let ours = kernel.load_at(SCORE, who)?;
    let theirs = kernel.load_at(SCORE, neighbour)?;
    let better = above(&mut kernel, theirs, ours)?;
    let take = kernel.module().i_mul(uint, my_turn, better)?;
    // **Only what lost changes.** Mutating whenever a chip is not *ahead* of its neighbour means two
    // equals churn for ever and a good genome is destroyed by the next turn it takes; this way a
    // chip that drew or won keeps what it has, and only one that was beaten copies and is changed.
    //
    // **Except on the first tick**, where every score is nought, nobody is better than anybody, and
    // so nothing would ever be put on the list to be evaluated at all — which is a colony that sits
    // at nought for ever, and is what this did before the line below.
    let first = is(&mut kernel, tick, one)?;
    let ahead = above(&mut kernel, ours, theirs)?;
    let behind = minus(&mut kernel, one, ahead)?;
    let churn = kernel.module().i_mul(uint, my_turn, behind)?;
    let either = kernel.module().i_add(uint, churn, first)?;
    let restless = at_most(&mut kernel, either, 1)?;

    // **Three gates at a time, not one.** With one the colony settles at 56 of 64 by generation two
    // hundred and four times the colony does not move it: a single gate turned over is too small a
    // step to leave the hollow it is in. `MUTATIONS` is what that number cost.
    let mut seed = kernel.load_at(SEED, who)?;
    let mut spots = Vec::with_capacity(MUTATIONS as usize);
    for _ in 0..MUTATIONS {
        seed = stir(&mut kernel, seed)?;
        let drawn = low_bits(&mut kernel, seed, CELLS.next_power_of_two().trailing_zeros())?;
        let spot = at_most(&mut kernel, drawn, CELLS - 1)?;
        seed = stir(&mut kernel, seed)?;
        // **Drawn from what the exam allows, not from every gate there is.** The index is picked
        // and then turned into a kind, so a set of three is three kinds and never a fourth.
        let wide = u32::try_from(allowed.len().max(1)).unwrap_or(1);
        let rolled = low_bits(&mut kernel, seed, wide.next_power_of_two().trailing_zeros())?;
        let which = at_most(&mut kernel, rolled, wide - 1)?;
        let mut fresh = kernel.module().constant_u32(allowed.first().copied().unwrap_or(0))?;
        for (index, &kind) in allowed.iter().enumerate().skip(1) {
            let want = kernel.module().constant_u32(u32::try_from(index).unwrap_or(0))?;
            let hit = is(&mut kernel, which, want)?;
            let value = kernel.module().constant_u32(kind)?;
            fresh = pick(&mut kernel, hit, value, fresh)?;
        }
        spots.push((spot, fresh));
    }
    let again = seed;

    let span = kernel.module().constant_u32(CELLS)?;
    let ours_at = kernel.module().i_mul(uint, who, span)?;
    let theirs_at = kernel.module().i_mul(uint, neighbour, span)?;
    for cell in 0..CELLS {
        let off = kernel.module().constant_u32(cell)?;
        let here = kernel.module().i_add(uint, ours_at, off)?;
        let there = kernel.module().i_add(uint, theirs_at, off)?;
        let held = kernel.load_at(GENE, here)?;
        let other = kernel.load_at(GENE, there)?;
        let mut settled = pick(&mut kernel, take, other, held)?;
        for &(spot, fresh) in &spots {
            let at_spot = is(&mut kernel, spot, off)?;
            let changing = kernel.module().i_mul(uint, at_spot, restless)?;
            settled = pick(&mut kernel, changing, fresh, settled)?;
        }
        kernel.store_at(GENE, here, settled)?;
    }
    let last = stir(&mut kernel, again)?;
    kernel.store_at(SEED, who, last)?;

    // **The ones that did not change go to a slot past the end of the list.** Sending them to slot
    // nought instead makes every one of them read that slot and write it back, which races with the
    // one entry that belongs there: a chip that did change is erased from the list, keeps last
    // generation's score, and the same seed stops giving the same answer. It did.
    let slot = kernel.atomic_add_at(COUNTS, none, restless)?;
    let safe = at_most(&mut kernel, slot, colony - 1)?;
    let past = kernel.module().constant_u32(colony)?;
    let landing = pick(&mut kernel, restless, safe, past)?;
    let keeping = pick(&mut kernel, restless, who, past)?;
    kernel.store_at(CHANGED, landing, keeping)?;
    kernel.finish()
}

/// Turn the length of that list into two workgroup counts: one a wire, one a chip.
fn ready(subgroup: u32, cases: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.element();
    let one = kernel.module().constant_u32(1)?;
    let none = kernel.module().constant_u32(0)?;
    let pad = kernel.module().constant_u32(subgroup - 1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let by = subgroup.trailing_zeros();

    // A pass over every wire of every changed chip runs `ROWS × CASES` invocations for each of
    // them, and a pass over the chips runs one.
    let wide = kernel
        .module()
        .constant_u32((ROWS * cases).trailing_zeros())?;
    let wires = {
        let mut lanes = kernel.lanes()?;
        let now = lanes.splat_id::<U32, 1>(held)?;
        let much = lanes.splat_id::<U32, 1>(wide)?;
        lanes.shift_left(now, much)?.id()
    };
    for (slot, count) in [(1, wires), (4, held)] {
        let bumped = kernel.module().i_add(uint, count, pad)?;
        let groups = down_by(&mut kernel, bumped, by)?;
        let at_x = kernel.module().constant_u32(slot)?;
        kernel.store_at(COUNTS, at_x, groups)?;
        let at_y = kernel.module().constant_u32(slot + 1)?;
        kernel.store_at(COUNTS, at_y, one)?;
        let at_z = kernel.module().constant_u32(slot + 2)?;
        kernel.store_at(COUNTS, at_z, one)?;
    }
    kernel.finish()
}

/// This invocation's chip, row and input, taken apart from one index.
///
/// A pass over wires runs one invocation for every (chip, row, input) there is among the changed
/// chips, and `CASES` and `ROWS` are powers of two, so this is two shifts and two masks.
fn wire_at(
    kernel: &mut Kernel<U32>,
    subgroup: u32,
    cases: u32,
    colony: u32,
    held: Id,
) -> Result<(Id, Id, Id), LaneError> {
    let index = mine(kernel, subgroup)?;
    let case = low_bits(kernel, index, cases.trailing_zeros())?;
    let rest = down_by(kernel, index, cases.trailing_zeros())?;
    let row = low_bits(kernel, rest, ROWS.trailing_zeros())?;
    let slot = down_by(kernel, rest, ROWS.trailing_zeros())?;
    let chip = listed(kernel, slot, held, colony)?;
    Ok((chip, row, case))
}

/// Where one wire of one chip lives.
fn wire(
    kernel: &mut Kernel<U32>,
    chip: Id,
    column: u32,
    row: Id,
    case: Id,
    cases: u32,
) -> Result<Id, LaneError> {
    let uint = kernel.index_type();
    let wide = kernel.module().constant_u32(COLUMNS)?;
    let along = kernel.module().i_mul(uint, chip, wide)?;
    let at = kernel.module().constant_u32(column)?;
    let placed = kernel.module().i_add(uint, along, at)?;
    let tall = kernel.module().constant_u32(ROWS)?;
    let down = kernel.module().i_mul(uint, placed, tall)?;
    let seated = kernel.module().i_add(uint, down, row)?;
    let many = kernel.module().constant_u32(cases)?;
    let spread = kernel.module().i_mul(uint, seated, many)?;
    Ok(kernel.module().i_add(uint, spread, case)?)
}

/// Put one step's inputs into the first column, and carry the state rows across from the last.
///
/// # Where time comes from
///
/// With `steps 1` the whole input word arrives at once and this is what it always was. With more,
/// the word is cut into that many pieces and one arrives a step, so a chip that wants the earlier
/// pieces has to have kept them: the `state` rows below the inputs are wired from the **last**
/// column back to the first, which is the only feedback in the design and the only memory.
///
/// The last column still holds the previous step when this runs, so there is no second buffer —
/// the state is read out of where the answer was.
///
/// `first` is the step that has nothing behind it, where the state rows start at nought.
fn feed(
    subgroup: u32,
    cases: u32,
    colony: u32,
    step: u32,
    fan: u32,
    state: u32,
    first: bool,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, colony, held)?;

    // This step's slice of the word: bit `step * fan + row` of the input.
    let along = kernel.module().constant_u32(step * fan)?;
    let uint = kernel.index_type();
    let reach = kernel.module().i_add(uint, row, along)?;
    let shifted = shift_down(&mut kernel, case, reach)?;
    let arriving = low_bits(&mut kernel, shifted, 1)?;

    // What was on this row when the last step finished, or nought where there was no last step.
    let carried = if first {
        none
    } else {
        let at = wire(&mut kernel, chip, COLUMNS - 1, row, case, cases)?;
        kernel.load_at(WIRE, at)?
    };

    let fan_at = kernel.module().constant_u32(fan)?;
    let feeding = above(&mut kernel, fan_at, row)?;
    let past = kernel.module().constant_u32(fan + state)?;
    let within = above(&mut kernel, past, row)?;
    let not_input = minus(&mut kernel, one, feeding)?;
    let remembering = kernel.module().i_mul(uint, within, not_input)?;
    let bottom = kernel.module().constant_u32(ROWS - 1)?;
    let lowest = is(&mut kernel, row, bottom)?;

    let padding = pick(&mut kernel, lowest, one, none)?;
    let kept = pick(&mut kernel, remembering, carried, padding)?;
    let settled = pick(&mut kernel, feeding, arriving, kept)?;
    let at = wire(&mut kernel, chip, 0, row, case, cases)?;
    kernel.store_at(WIRE, at, settled)?;
    kernel.finish()
}

/// One column of gates, fed by the column to its left.
///
/// **Eleven of these are generated**, one a column, which is what an emitter buys over a shader
/// language. Every gate is computed and the genome picks which answer is kept: a `Predicate` is not
/// a value, and `decisions/DR-0003` has no branches.
fn propagate(
    subgroup: u32,
    column: u32,
    cases: u32,
    colony: u32,
    noise: u32,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, colony, held)?;

    let up = {
        let back = kernel.module().constant_u32(ROWS - 1)?;
        let raised = kernel.module().i_add(uint, row, back)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let down = {
        let raised = kernel.module().i_add(uint, row, one)?;
        low_bits(&mut kernel, raised, ROWS.trailing_zeros())?
    };
    let at_a = wire(&mut kernel, chip, column - 1, up, case, cases)?;
    let at_b = wire(&mut kernel, chip, column - 1, down, case, cases)?;
    let at_w = wire(&mut kernel, chip, column - 1, row, case, cases)?;
    let a = kernel.load_at(WIRE, at_a)?;
    let b = kernel.load_at(WIRE, at_b)?;
    let straight = kernel.load_at(WIRE, at_w)?;

    // `min` is and and `max` is or, for values that are nought or one; exclusive-or is the low bit
    // of the sum, because the lane API has neither an exclusive-or nor a bitwise and.
    let (both, either) = {
        let mut lanes = kernel.lanes()?;
        let x = lanes.splat_id::<U32, 1>(a)?;
        let y = lanes.splat_id::<U32, 1>(b)?;
        let small = lanes.min(x, y)?.id();
        let large = lanes.max(x, y)?.id();
        (small, large)
    };
    let sum = kernel.module().i_add(uint, a, b)?;
    let odd = low_bits(&mut kernel, sum, 1)?;
    let inverted = minus(&mut kernel, one, straight)?;

    let span = kernel.module().constant_u32(CELLS)?;
    let mine_at = kernel.module().i_mul(uint, chip, span)?;
    let cell = kernel.module().constant_u32((column - 1) * ROWS)?;
    let along = kernel.module().i_add(uint, mine_at, cell)?;
    let gene_at = kernel.module().i_add(uint, along, row)?;
    let kind = kernel.load_at(GENE, gene_at)?;

    let mut answer = straight;
    for (number, value) in [(1, both), (2, either), (3, odd), (4, inverted)] {
        let want = kernel.module().constant_u32(number)?;
        let hit = is(&mut kernel, kind, want)?;
        answer = pick(&mut kernel, hit, value, answer)?;
    }

    // **A gate that is broken this generation passes its left neighbour through**, which is the
    // cheapest fault there is. Which gates are broken comes from the tick and the cell, so it is the
    // same fault for every chip — nobody is scored against an easier grid than anybody else — and a
    // different one next generation. A chip that leans on one gate is punished the generation that
    // gate is out; one that does not, is not.
    if noise > 0 {
        let at_tick = kernel.module().constant_u32(0)?;
        let tick = kernel.load_at(PARAMS, at_tick)?;
        // **The fault pattern holds for `SETTLED` generations.** Drawing a new one every generation
        // makes a chip carry a score measured under one pattern into a tournament judged under
        // another, which selects for having been lucky. Held still, the scores inside an epoch are
        // about the same grid, and it is over epochs that leaning on one gate is punished.
        let epoch = down_by(&mut kernel, tick, SETTLED.trailing_zeros())?;
        let apart = kernel.module().constant_u32(2_654_435_761)?;
        let spread = kernel.module().i_mul(uint, epoch, apart)?;
        let cell_at = kernel.module().constant_u32((column - 1) * ROWS)?;
        let placed = kernel.module().i_add(uint, cell_at, row)?;
        let mixed = kernel.module().i_add(uint, spread, placed)?;
        let drawn = stir(&mut kernel, mixed)?;
        let narrowed = low_bits(&mut kernel, drawn, noise.trailing_zeros())?;
        let none_left = kernel.module().constant_u32(0)?;
        let broken = is(&mut kernel, narrowed, none_left)?;
        answer = pick(&mut kernel, broken, straight, answer)?;
    }
    let at = wire(&mut kernel, chip, column, row, case, cases)?;
    kernel.store_at(WIRE, at, answer)?;
    kernel.finish()
}

/// How many of the inputs the top-right wire got right.
/// How many marked answer bits this chip gets right, and — where the exam asks for it — how few
/// gates it took.
///
/// # Pressing on the gate count
///
/// Nothing selects against a gate that does nothing, so a grid that answers keeps every gate it was
/// born with. `lean` makes the score `correct × (CELLS + 1) + (CELLS − used)`: correctness still
/// decides, and between two chips that are equally right the one with fewer gates wins. The host
/// takes it apart again for the report.
fn tally(
    subgroup: u32,
    cases: u32,
    outputs: u32,
    colony: u32,
    lean: bool,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let slot = mine(&mut kernel, subgroup)?;
    let chip = listed(&mut kernel, slot, held, colony)?;

    // **One tap a row, down the right-hand edge.** Output nought is the top wire of the last column,
    // output one the wire under it, and so on — the same place a one-bit exam reads, extended.
    let mut fit = kernel.module().constant_u32(0)?;
    for case in 0..cases {
        let which = kernel.module().constant_u32(case)?;
        let want = kernel.load_at(TARGET, which)?;
        let asked = kernel.load_at(KEEP, which)?;
        for output in 0..outputs {
            let row = kernel.module().constant_u32(output)?;
            let at = wire(&mut kernel, chip, COLUMNS - 1, row, which, cases)?;
            let said = kernel.load_at(WIRE, at)?;
            // **Both sides carry a mask.** A bit the exam left out counts for nothing either way, on
            // the input side and the output side alike, so a partial specification gives a partial
            // score rather than a wrong one.
            let wanted = bit(&mut kernel, want, output)?;
            let marked = bit(&mut kernel, asked, output)?;
            let hit = is(&mut kernel, said, wanted)?;
            let counted = kernel.module().i_mul(uint, hit, marked)?;
            fit = kernel.module().i_add(uint, fit, counted)?;
        }
    }
    let final_score = if lean {
        let span = kernel.module().constant_u32(CELLS)?;
        let base = kernel.module().i_mul(uint, chip, span)?;
        let mut spare = kernel.module().constant_u32(0)?;
        for cell in 0..CELLS {
            let off = kernel.module().constant_u32(cell)?;
            let at = kernel.module().i_add(uint, base, off)?;
            let kind = kernel.load_at(GENE, at)?;
            // A wire is not a gate, so it is what is left over rather than what is spent.
            let idle = is(&mut kernel, kind, none)?;
            spare = kernel.module().i_add(uint, spare, idle)?;
        }
        let weight = kernel.module().constant_u32(CELLS + 1)?;
        let scaled = kernel.module().i_mul(uint, fit, weight)?;
        kernel.module().i_add(uint, scaled, spare)?
    } else {
        fit
    };
    kernel.store_at(SCORE, chip, final_score)?;
    kernel.finish()
}

// ---------- the exam, which is the only thing the user writes ----------

/// What the user asked for: which inputs matter and what each should answer.
struct Exam {
    /// How many inputs, one to six.
    bits: u32,
    /// How many answer bits. Output nought is the top wire of the last column.
    outputs: u32,
    /// How many chips are in the arena, a power of two.
    ///
    /// **This is the way out of a local optimum, and patience is not.** On the scattered pair below,
    /// ten times the generations changes nothing and the same starts still fail; four times the
    /// colony takes three starts in eight to six, and sixteen times takes it to eight.
    colony: u32,
    /// Which gates the search draws from. Wire is always in it, because a grid needs somewhere
    /// for a signal to go.
    allowed: Vec<u32>,
    /// One gate in this many is broken each generation, or nought for none. A power of two.
    noise: u32,
    /// Whether a chip with fewer gates beats an equally correct one with more.
    lean: bool,
    /// How many pieces the input word is cut into, one arriving a step.
    steps: u32,
    /// How many rows are wired from the last column back to the first, which is all the memory there
    /// is.
    state: u32,
    /// How many chips share an arena. The colony is this many arenas side by side.
    island: u32,
    /// Where the random grids start.
    ///
    /// **It matters more than anything else in this file.** Five starts on the parity of six inputs
    /// came out at 52, 60, 62, 62 and 64 of 64, so one run says nothing on its own.
    ///
    /// The numbers this file first carried were taken with a race in `select`, where a chip that
    /// did not change wrote to slot nought of the changed list against whichever chip belonged
    /// there. The same seed now gives the same answer four runs out of four.
    seed: u32,
    /// The wanted answer for every input, its bits packed one an output.
    want: Vec<u32>,
    /// Which of those bits the exam asks about, packed the same way.
    ask: Vec<u32>,
    /// What to call it in the report.
    name: String,
}

impl Exam {
    /// How many inputs there are altogether.
    fn cases(&self) -> u32 {
        1 << self.bits
    }

    /// How many answer bits are marked, which is what a perfect score is.
    fn asked(&self) -> u32 {
        self.ask.iter().map(|it| it.count_ones()).sum()
    }
}

/// The example, printed and used when no file is named.
const WRITTEN: &str =
    "# The exam. Everything the machine is told, and nothing about how.\ninputs 4\noutputs 3\ncount\n";

/// Every input a pattern of noughts, ones and dashes stands for.
///
/// **A dash is both.** `10-1` is two inputs and `----` is all sixteen, so one line can say what
/// otherwise takes a table — and the leftmost character is input nought, the row it enters on.
fn spread_of(pattern: &str) -> Vec<u32> {
    let free: Vec<u32> = pattern
        .chars()
        .enumerate()
        .filter(|&(_, it)| it == '-')
        .filter_map(|(at, _)| u32::try_from(at).ok())
        .collect();
    let fixed = pattern
        .chars()
        .enumerate()
        .filter(|&(_, it)| it == '1')
        .filter_map(|(at, _)| u32::try_from(at).ok())
        .fold(0_u32, |held, at| held | (1 << at));
    (0..1_u32 << free.len())
        .map(|combination| {
            free.iter()
                .enumerate()
                .filter_map(|(which, &at)| u32::try_from(which).ok().map(|it| (it, at)))
                .fold(fixed, |held, (which, at)| {
                    held | (((combination >> which) & 1) << at)
                })
        })
        .collect()
}

/// Read an exam.
///
/// # The whole language
///
/// ```text
///   # a comment
///   inputs 4        how many inputs, one to six
///   outputs 3       how many answer bits — one if left out
///   gates and or not   the gates it draws from; wire is always in
///   noise 16        break one gate in sixteen each generation
///   lean 1          among equally right chips, prefer the one with fewer gates
///   steps 4         cut the word into four, one piece a step — time
///   state 2         rows wired from the last column back to the first — memory
///   colony 65536    how many chips search at once — the way out of a local optimum
///   island 256      how many of them share an arena — the colony if left out
///   seed 1234       where the random grids start, which decides more than it looks
///   parity          fill the table: the parity of the inputs
///   majority        fill the table: more ones than noughts
///   equal           fill the table: every input the same
///   count           fill the table: how many ones there are, in binary
///   add             fill the table: the first half plus the second
///   10-1 -> 01      the inputs that pattern stands for, and what they answer
///   ---- -> --1     a dash on the left is every value; a dash on the right is not asked
/// ```
///
/// **A bit nobody names is not scored**, on either side. Write four lines and the answer is judged
/// on those; write `count` and it is judged on everything. What is left out is left out rather than
/// assumed, which is what makes a partial exam a partial score instead of a wrong one.
fn read(text: &str) -> Result<Exam, String> {
    let mut bits = None;
    let mut outputs = 1_u32;
    let mut seed = 0x0AC1_E000_u32;
    let mut colony = 1_u32 << 12;
    let mut allowed: Option<Vec<u32>> = None;
    let mut noise = 0_u32;
    let mut lean = false;
    let mut steps = 1_u32;
    let mut state = 0_u32;
    let mut island = None;
    let mut want: Vec<u32> = Vec::new();
    let mut ask: Vec<u32> = Vec::new();
    let mut name = String::from("an exam");

    for (number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let say = |what: &str| format!("line {}: {what}", number + 1);
        let mut word = line.split_whitespace();
        match (word.next(), bits) {
            (Some("gates"), _) => {
                let mut kinds = vec![0_u32];
                for name in word.by_ref() {
                    let kind = match name {
                        "wire" => 0,
                        "and" => 1,
                        "or" => 2,
                        "xor" => 3,
                        "not" => 4,
                        _ => return Err(say("a gate is wire, and, or, xor or not")),
                    };
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                }
                if kinds.len() < 2 {
                    return Err(say("`gates` wants at least one gate besides wire"));
                }
                allowed = Some(kinds);
            }
            (Some("noise"), _) => {
                noise = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| it == 0 || it.is_power_of_two())
                    .ok_or_else(|| say("`noise` wants nought or a power of two"))?;
            }
            (Some("lean"), _) => {
                lean = word.next() != Some("0");
            }
            (Some("steps"), _) => {
                steps = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..=8).contains(&it))
                    .ok_or_else(|| say("`steps` wants a number from one to eight"))?;
            }
            (Some("state"), _) => {
                state = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| it < ROWS)
                    .ok_or_else(|| say("`state` wants a number under eight"))?;
            }
            (Some("colony"), _) => {
                colony = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|it| it.is_power_of_two() && (64..=1 << 17).contains(it))
                    .ok_or_else(|| say("`colony` wants a power of two from 64 to 131072"))?;
            }
            (Some("island"), _) => {
                island = Some(
                    word.next()
                        .and_then(|it| it.parse::<u32>().ok())
                        .filter(|it| it.is_power_of_two() && (2..=1 << 17).contains(it))
                        .ok_or_else(|| say("`island` wants a power of two of at least two"))?,
                );
            }
            (Some("seed"), _) => {
                seed = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`seed` wants a number"))?;
            }
            (Some("outputs"), _) => {
                outputs = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..ROWS).contains(&it))
                    .ok_or_else(|| say("`outputs` wants a number from one to seven"))?;
            }
            (Some("inputs"), _) => {
                let how = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..=6).contains(&it))
                    .ok_or_else(|| say("`inputs` wants a number from one to six"))?;
                bits = Some(how);
                want = vec![0; 1 << how];
                ask = vec![0; 1 << how];
            }
            (Some(_), None) => return Err(say("`inputs` has to come first")),
            (Some(filler @ ("parity" | "majority" | "equal" | "count" | "add")), Some(how)) => {
                let widest = match filler {
                    "count" => u32::BITS - how.leading_zeros(),
                    "add" => how / 2 + 1,
                    _ => 1,
                };
                if outputs < widest {
                    return Err(say(&format!("`{filler}` wants `outputs {widest}` or more")));
                }
                if filler == "add" && how % 2 != 0 {
                    return Err(say("`add` wants an even number of inputs"));
                }
                name = filler.to_owned();
                let full = (1_u32 << outputs) - 1;
                let half = how / 2;
                for case in 0..1_u32 << how {
                    let ones = case.count_ones();
                    let answer = match filler {
                        "parity" => ones & 1,
                        "majority" => u32::from(ones * 2 > how),
                        "equal" => u32::from(ones == 0 || ones == how),
                        "count" => ones,
                        _ => (case & ((1 << half) - 1)) + (case >> half),
                    };
                    if let (Some(a), Some(b)) =
                        (want.get_mut(case as usize), ask.get_mut(case as usize))
                    {
                        *a = answer;
                        *b = full;
                    }
                }
            }
            (Some(pattern), Some(how)) => {
                if pattern.len() != how as usize || pattern.chars().any(|it| !"01-".contains(it)) {
                    return Err(say("expected a pattern of noughts, ones and dashes"));
                }
                if word.next() != Some("->") {
                    return Err(say("expected `->` after the pattern"));
                }
                let answer = word
                    .next()
                    .filter(|it| it.len() == outputs as usize)
                    .filter(|it| it.chars().all(|c| "01-".contains(c)))
                    .ok_or_else(|| {
                        say("expected an answer of noughts, ones and dashes, `outputs` long")
                    })?;
                let (mut wants, mut asks) = (0_u32, 0_u32);
                for (at, it) in answer.chars().enumerate() {
                    let Ok(at) = u32::try_from(at) else { continue };
                    match it {
                        '1' => {
                            wants |= 1 << at;
                            asks |= 1 << at;
                        }
                        '0' => asks |= 1 << at,
                        _ => {}
                    }
                }
                for case in spread_of(pattern) {
                    if let (Some(a), Some(b)) =
                        (want.get_mut(case as usize), ask.get_mut(case as usize))
                    {
                        // What a line says replaces what it says about, and leaves the rest alone.
                        *a = (*a & !asks) | wants;
                        *b |= asks;
                    }
                }
                if name == "an exam" {
                    name = String::from("a table");
                }
            }
            (None, _) => {}
        }
    }
    let bits = bits.ok_or_else(|| String::from("the exam says nothing about `inputs`"))?;
    if bits % steps != 0 {
        return Err(format!("`steps {steps}` does not divide `inputs {bits}`"));
    }
    if bits / steps + state >= ROWS {
        return Err(format!(
            "{} input rows and {state} state rows leave no room for the one on the bottom",
            bits / steps
        ));
    }
    let island = island.unwrap_or(colony).min(colony);
    let allowed = allowed.unwrap_or_else(|| (0..KINDS).collect());
    Ok(Exam { bits, outputs, allowed, noise, lean, steps, state, colony, island, seed, want, ask, name })
}

// ---------- the host, which lays out the arena and then asks almost nothing ----------

/// One step of the same generator the kernels use.
fn stirred(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *seed
}

/// The chip, run again here exactly as the columns run it on the card.
///
/// **Including the seam between steps**, where the state rows carry and everything else is put back
/// to what the exam and the padding say.
fn runs(genome: &[u32], case: u32, exam: &Exam) -> u32 {
    let fan = exam.bits / exam.steps;
    let mut last: Vec<u32> = vec![0; ROWS as usize];
    for step in 0..exam.steps {
        let keep = |row: u32| last.get(row as usize).copied().unwrap_or(0);
        let mut column: Vec<u32> = (0..ROWS)
            .map(|row| {
                if row < fan {
                    (case >> (step * fan + row)) & 1
                } else if row < fan + exam.state {
                    if step == 0 { 0 } else { keep(row) }
                } else if row == ROWS - 1 {
                    1
                } else {
                    0
                }
            })
            .collect();
        for at in 1..COLUMNS {
            let read = |row: u32| column.get((row % ROWS) as usize).copied().unwrap_or(0);
            let next: Vec<u32> = (0..ROWS)
                .map(|row| {
                    let (up, down, straight) = (read(row + ROWS - 1), read(row + 1), read(row));
                    match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                        Some(1) => up.min(down),
                        Some(2) => up.max(down),
                        Some(3) => (up + down) & 1,
                        Some(4) => 1 - straight,
                        _ => straight,
                    }
                })
                .collect();
            column = next;
        }
        last = column;
    }
    // The answer is the top `outputs` wires after the last step, packed as the exam packs it.
    (0..exam.outputs)
        .filter(|&at| last.get(at as usize).copied().unwrap_or(0) == 1)
        .fold(0_u32, |held, at| held | (1 << at))
}
/// How many of the marked answer bits this chip gets right, worked out on the host.
///
/// **One a bit, not one a case**, so a chip right about two outputs of three scores two.
fn scored(genome: &[u32], exam: &Exam) -> u32 {
    (0..exam.cases())
        .map(|case| {
            let said = runs(genome, case, exam);
            let want = exam.want.get(case as usize).copied().unwrap_or(0);
            let ask = exam.ask.get(case as usize).copied().unwrap_or(0);
            u32::try_from(
                (0..exam.outputs)
                    .filter(|&at| (ask >> at) & 1 == 1 && (said >> at) & 1 == (want >> at) & 1)
                    .count(),
            )
            .unwrap_or(0)
        })
        .sum()
}

/// What the state rows hold after each step, for one input.
///
/// # Reading a machine off a grid
///
/// The state rows are the only thing that crosses from one step to the next, so whatever the chip
/// remembers is in them and nowhere else. Running an input and writing the rows down after every
/// step is the state machine, in the encoding the search happened to pick — which is nobody's idea
/// of a good one and is still the whole of it.
fn remembers(genome: &[u32], case: u32, exam: &Exam) -> Vec<u32> {
    let fan = exam.bits / exam.steps;
    let mut last: Vec<u32> = vec![0; ROWS as usize];
    let mut seen = Vec::with_capacity(exam.steps as usize);
    for step in 0..exam.steps {
        let keep = |row: u32| last.get(row as usize).copied().unwrap_or(0);
        let mut column: Vec<u32> = (0..ROWS)
            .map(|row| {
                if row < fan {
                    (case >> (step * fan + row)) & 1
                } else if row < fan + exam.state {
                    if step == 0 { 0 } else { keep(row) }
                } else if row == ROWS - 1 {
                    1
                } else {
                    0
                }
            })
            .collect();
        for at in 1..COLUMNS {
            let read = |row: u32| column.get((row % ROWS) as usize).copied().unwrap_or(0);
            let next: Vec<u32> = (0..ROWS)
                .map(|row| {
                    let (up, down, straight) = (read(row + ROWS - 1), read(row + 1), read(row));
                    match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                        Some(1) => up.min(down),
                        Some(2) => up.max(down),
                        Some(3) => (up + down) & 1,
                        Some(4) => 1 - straight,
                        _ => straight,
                    }
                })
                .collect();
            column = next;
        }
        let held = (fan..fan + exam.state)
            .filter(|&row| column.get(row as usize).copied().unwrap_or(0) == 1)
            .fold(0_u32, |bits, row| bits | (1 << (row - fan)));
        seen.push(held);
        last = column;
    }
    seen
}
/// Take a score apart: how much of it is being right, and how many gates were spent.
///
/// With `lean` off a score is the answer bits and nothing else. With it on the gates are packed
/// underneath, low enough that being right always outranks being small.
fn unpack(score: u32, lean: bool) -> (u32, u32) {
    if lean {
        (score / (CELLS + 1), CELLS - (score % (CELLS + 1)))
    } else {
        (score, 0)
    }
}

/// Which cells the answer actually depends on, traced backwards from the wires it is read off.
///
/// # Opening the box
///
/// A grid of 88 gates answers with a handful of them and the rest are scenery: nothing selects
/// against a gate that does nothing, so nothing removes it. This walks back from the output rows of
/// the last column — and from the state rows, which are outputs into the next step — and marks every
/// wire an answer can have come through. A gate reads its two diagonals, or the cell straight left
/// where it is a wire or a not, so which cells it wants depends on what it is.
fn live(genome: &[u32], exam: &Exam) -> Vec<bool> {
    let fan = exam.bits / exam.steps;
    let mut wanted = vec![false; (COLUMNS * ROWS) as usize];
    let at = |column: u32, row: u32| (column * ROWS + row % ROWS) as usize;
    for row in 0..exam.outputs.max(fan + exam.state) {
        let reads_it = row < exam.outputs || (row >= fan && row < fan + exam.state);
        if let Some(slot) = wanted.get_mut(at(COLUMNS - 1, row)).filter(|_| reads_it) {
            *slot = true;
        }
    }
    let mut used = vec![false; (CELLS) as usize];
    for column in (1..COLUMNS).rev() {
        for row in 0..ROWS {
            if !wanted.get(at(column, row)).copied().unwrap_or(false) {
                continue;
            }
            let cell = ((column - 1) * ROWS + row) as usize;
            let kind = genome.get(cell).copied().unwrap_or(0);
            if let Some(slot) = used.get_mut(cell) {
                *slot = kind != 0;
            }
            let reaches: [u32; 2] = if kind == 0 || kind == 4 {
                [row, row]
            } else {
                [row + ROWS - 1, row + 1]
            };
            for back in reaches {
                if let Some(slot) = wanted.get_mut(at(column - 1, back)) {
                    *slot = true;
                }
            }
        }
    }
    used
}

/// What one gate failing costs, over every gate there is.
///
/// # What self-healing would look like
///
/// A gate is broken by turning it into a wire — the cheapest fault there is, and the one a dead
/// transistor most resembles. A circuit with nothing spare loses a point for every live gate broken
/// and nothing for a dead one; a circuit with something spare keeps its score through a fault that
/// should have cost it. The count of faults survived is the difference.
fn faults(genome: &[u32], exam: &Exam) -> (u32, u32, u32) {
    let whole = scored(genome, exam);
    let alive = live(genome, exam);
    let mut survived = 0;
    let mut counted = 0;
    for (cell, &on) in alive.iter().enumerate() {
        if !on {
            continue;
        }
        counted += 1;
        let mut broken = genome.to_vec();
        if let Some(slot) = broken.get_mut(cell) {
            *slot = 0;
        }
        if scored(&broken, exam) == whole {
            survived += 1;
        }
    }
    (counted, survived, whole)
}
/// The chip as something to look at: one letter a gate, the inputs down the left.
///
/// **An arrow marks an answer wire.** Output nought is the top row of the last column, output one
/// the row under it, and the exam says how many there are.
fn drawn(genome: &[u32], bits: u32, outputs: u32, alive: &[bool]) -> String {
    let mut out = String::new();
    for row in 0..ROWS {
        out.push_str("    ");
        out.push(if row + 1 == ROWS {
            '1'
        } else if row < bits {
            char::from(b'a' + u8::try_from(row).unwrap_or(0))
        } else {
            '0'
        });
        out.push(' ');
        for at in 1..COLUMNS {
            // **A dead gate is drawn as nothing**, because that is what it does. What is left is the
            // circuit the answer travels through, and it is a great deal smaller than the grid.
            let cell = ((at - 1) * ROWS + row) as usize;
            let on = alive.get(cell).copied().unwrap_or(false);
            out.push(match genome.get(cell).copied() {
                _ if !on => '·',
                Some(1) => '&',
                Some(2) => 'v',
                Some(3) => '^',
                Some(4) => '!',
                _ => '-',
            });
        }
        if row < outputs {
            out.push_str(" <- ");
            out.push(char::from(b'0' + u8::try_from(row).unwrap_or(0)));
        }
        out.push('\n');
    }
    out
}
/// The colony's scores as a row of bars, so the collapse can be watched rather than read.
fn spread(scores: &[u32], perfect: u32) -> String {
    const BINS: usize = 24;
    let mut held = [0_u32; BINS];
    for &score in scores {
        let at = (score as usize * (BINS - 1)) / (perfect.max(1) as usize);
        if let Some(slot) = held.get_mut(at.min(BINS - 1)) {
            *slot += 1;
        }
    }
    let most = held.iter().copied().max().unwrap_or(1).max(1);
    held.iter()
        .map(|&count| {
            let height = (count * 8) / most;
            char::from_u32(0x2581 + height.min(7)).unwrap_or(' ')
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // **The exam is the only input.** Name a file and it is read; name nothing and the one below is
    // used, printed, so there is something to copy and change.
    let named = std::env::args().nth(1);
    let text = match &named {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            println!("no exam named, so this one is used — put it in a file to change it:\n");
            for line in WRITTEN.lines() {
                println!("    {line}");
            }
            println!();
            WRITTEN.to_owned()
        }
    };
    let exam = read(&text).map_err(|why| format!("the exam does not read: {why}"))?;
    let cases = exam.cases();
    let colony = exam.colony;
    let perfect = exam.asked();
    if perfect == 0 {
        return Err("the exam asks about nothing".into());
    }

    let Some(host) = Host::open()? else {
        println!("no Vulkan device");
        return Ok(());
    };
    let width = host.width();
    println!("{} — subgroup {width}", host.name());
    println!(
        "  {}: {} inputs, {} outputs, {perfect} answer bits marked of {}",
        exam.name,
        exam.bits,
        exam.outputs,
        cases * exam.outputs
    );

    // **One pass through the grid a step, and as many steps as the exam asks for.** The state rows
    // carry across the seam between them and nothing else does.
    let fan = exam.bits / exam.steps;
    let mut modules = vec![
        advance(width)?,
        select(width, colony, exam.island, &exam.allowed)?,
        ready(width, cases)?,
    ];
    for step in 0..exam.steps {
        modules.push(feed(width, cases, colony, step, fan, exam.state, step == 0)?);
        for column in 1..COLUMNS {
            modules.push(propagate(width, column, cases, colony, exam.noise)?);
        }
    }
    modules.push(tally(width, cases, exam.outputs, colony, exam.lean)?);
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();

    let wires = Groups::From { binding: COUNTS as usize, at: 1 };
    let chips = Groups::From { binding: COUNTS as usize, at: 4 };
    let mut stages: Vec<Vec<Groups>> = vec![
        vec![Groups::Counting(1)],
        vec![Groups::Fixed(colony / width)],
        vec![Groups::Counting(1)],
    ];
    for _ in 0..exam.steps {
        for _ in 0..COLUMNS {
            stages.push(vec![wires]);
        }
    }
    stages.push(vec![chips]);
    let groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
    println!(
        "  {colony} chips of {COLUMNS}×{ROWS} gates — {} passes in {} stages\n",
        modules.len(),
        stages.len()
    );

    let sizes = [
        4,
        (colony * CELLS) as usize,
        cases as usize,
        ((colony + 1) * COLUMNS * ROWS * cases) as usize,
        (colony + 1) as usize,
        (colony + 1) as usize,
        (colony + 1) as usize,
        7,
        cases as usize,
    ];
    let mut arena = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        arena.write(binding, &vec![0_u32; *words])?;
    }
    arena.write(TARGET as usize, &exam.want)?;
    arena.write(KEEP as usize, &exam.ask)?;
    let mut seed = exam.seed;
    // **Drawn from what the exam allows**, or the restriction is only half a restriction: the grids
    // would start full of gates the mutation is never allowed to put back.
    let genes: Vec<u32> = (0..colony * CELLS)
        .map(|_| {
            let at = stirred(&mut seed) as usize % exam.allowed.len().max(1);
            exam.allowed.get(at).copied().unwrap_or(0)
        })
        .collect();
    arena.write(GENE as usize, &genes)?;
    let seeds: Vec<u32> = (0..=colony).map(|_| stirred(&mut seed)).collect();
    arena.write(SEED as usize, &seeds)?;

    let batches = GENERATIONS / BATCH;
    let mut spent = Duration::ZERO;
    println!("  {:>10}  {:>5}  {:>24}  {:>10}", "generation", "best", "the colony", "us a tick");
    let mut best;
    let mut ran = 0;
    for batch in 0..batches {
        ran = batch + 1;
        spent += arena.staged_frames(&groups, BATCH)?;
        let held = arena.read(SCORE as usize, colony as usize)?;
        let scores: Vec<u32> = held.iter().map(|&it| unpack(it, exam.lean).0).collect();
        best = scores.iter().copied().max().unwrap_or(0);
        // Densely at the start, where the colony is actually moving, and thinly after.
        if batch < 8 || batch % (batches / 8).max(1) == 0 || best == perfect || batch + 1 == batches {
            println!(
                "  {:>10}  {best:>5}  {}  {:>10.3}",
                (batch + 1) * BATCH,
                spread(&scores, perfect),
                arena.on_device().unwrap_or_default().as_secs_f64() * 1e6
            );
        }
        // With `lean` on, being right is where the work starts rather than where it stops.
        if best == perfect && !exam.lean {
            break;
        }
    }

    let raw = arena.read(SCORE as usize, colony as usize)?;
    let genes = arena.read(GENE as usize, (colony * CELLS) as usize)?;
    let (winner, _) = raw.iter().enumerate().max_by_key(|&(_, s)| *s).unwrap_or((0, &0));
    let genome = genes
        .get(winner * CELLS as usize..(winner + 1) * CELLS as usize)
        .unwrap_or(&[]);
    let alive = live(genome, &exam);
    println!("\n{}", drawn(genome, exam.bits, exam.outputs, &alive));
    println!("    - wire   & and   v or   ^ xor   ! not   · dead   (<- marks an answer wire)");
    let (live_gates, survived, whole) = faults(genome, &exam);
    println!(
        "\n  {live_gates} of {CELLS} gates carry the answer; the rest are scenery."
    );
    println!(
        "  Breaking one live gate into a wire: {} of {live_gates} faults change nothing, {} cost the answer.",
        survived,
        live_gates - survived
    );
    let _ = whole;
    // **What it does with everything, not only what was asked.** A circuit is judged on the marked
    // bits and is free on the rest, so the table below is the difference between the exam and the
    // answer — and it is where an encoding that collapses to one wire shows itself.
    if cases <= 16 {
        if exam.state > 0 {
            println!("  what it remembers, step by step:");
            for case in 0..cases.min(8) {
                let pattern: String = (0..exam.bits)
                    .map(|at| if (case >> at) & 1 == 1 { '1' } else { '0' })
                    .collect();
                let held: Vec<String> = remembers(genome, case, &exam)
                    .iter()
                    .map(|it| format!("{it}"))
                    .collect();
                println!("    {pattern}   state {}", held.join(" -> "));
            }
            println!();
        }
        println!("  every input, and what it answers:");
        for case in 0..cases {
            let pattern: String = (0..exam.bits)
                .map(|at| if (case >> at) & 1 == 1 { '1' } else { '0' })
                .collect();
            let said = runs(genome, case, &exam);
            let ask = exam.ask.get(case as usize).copied().unwrap_or(0);
            let want = exam.want.get(case as usize).copied().unwrap_or(0);
            let says: String = (0..exam.outputs)
                .map(|at| if (said >> at) & 1 == 1 { '1' } else { '0' })
                .collect();
            let asked: String = (0..exam.outputs)
                .map(|at| {
                    if (ask >> at) & 1 == 0 {
                        '-'
                    } else if (want >> at) & 1 == 1 {
                        '1'
                    } else {
                        '0'
                    }
                })
                .collect();
            let mark = if ask == 0 { "not asked" } else { "" };
            println!("    {pattern} -> {says}   exam said {asked}  {mark}");
        }
        println!();
    }
    let again = scored(genome, &exam);
    println!(
        "\n  worked out on the host it gets {again} of {perfect} — the card said {}{}",
        unpack(raw.get(winner).copied().unwrap_or(0), exam.lean).0,
        if again == perfect { ", and that is every case the exam asked about" } else { "" }
    );
    println!(
        "  {:.3} us a generation of host clock",
        spent.as_secs_f64() * 1e6 / f64::from(ran.max(1))
    );
    Ok(())
}
