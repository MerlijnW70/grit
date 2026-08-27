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
//!   apart 3         read the answers three rows apart instead of next to each other
//!   weigh dynamic   add the answers up by sum, by the weakest of them, or by weight
//!   slack 1         read the answer as a number and accept it within one
//!   ladder 1        reveal one output bit at a time, widening when it is mastered
//!   gates and or sum   the gates it draws from; wire is always in
//!   noise 16        break one gate in sixteen each generation
//!   lean wires      among equally right chips prefer the cheaper: 1 by gates, wires by wires
//!   steps 4         cut the word into four, one piece a step — time
//!   state 2         rows wired from the last column back to the first — memory
//!   hold 4          four more steps after the word, with nothing arriving
//!   linger 1        during a hold, keep the last slice on the wires instead
//!   band 8,24,24,8  split the rows into bands that cannot read each other, one an answer
//!   feeds 1         show the word to the first band only and starve the rest
//!   apiece 1        select each band on its own answer, not on the chip's total
//!   keep grid.txt   write the winner's grid out when the run ends
//!   start g.txt 64  put 64 copies of that grid into the drawn colony
//!   gates ... over  the one gate that crosses a band, one offset up — a carry
//!   fold 1          read the answers off one row, one a step, instead of one row each
//!   renew 8         after eight batches with no gain, draw the colony again but for the best
//!   colony 65536    how many chips search at once — the way out of a local optimum
//!   island 256      how many of them share an arena — the colony if left out
//!   seed 1234       where the random grids start, which decides more than it looks
//!   parity          fill the table: the parity of the inputs
//!   majority        fill the table: more ones than noughts
//!   equal           fill the table: every input the same
//!   count           fill the table: how many ones there are, in binary
//!   times           fill the table: the low half of the word times the high half
//!   shift           fill the table: the low half shifted left by the high half
//!   mac             fill the table: the first third times the second, plus the third
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
//! # A curriculum, and what it did not move
//!
//! `ladder 1` asks about output nought only to begin with and adds another when the colony has
//! mastered what it has — or when it has stopped improving for `PATIENCE` batches, because waiting
//! for mastery leaves a run stranded on a rung that is itself a wall. When the rung moves the whole
//! colony is re-scored: a chip that did not change carries a score measured against the easier exam,
//! and comparing that against the harder one hands the rung to whoever was asked less.
//!
//! It climbs, and it does not help:
//!
//! ```text
//!   count, of 48        colony 4 096   colony 16 384
//!   ladder 0                      42              42
//!   ladder 1                      42              42
//! ```
//!
//! Neither does the colony, which was the lever everywhere else. So something in `count` is not
//! what any of these knobs are for, and taking it apart says what:
//!
//! ```text
//!   asked on its own          eight starts
//!   bit 0, the parity         16 16 16 16 16 16 16 16   solved every time
//!   bit 1                     14 14 14 14 15 14 15 14   never
//!   bit 2, all four ones      16 16 16 16 16 16 16 16   solved every time
//! ```
//!
//! **Two things, and the ladder addresses neither.** One output bit is a wall on its own — bit one
//! of a count of four is *two or three of them*, which is neither monotone nor a parity and never
//! comes. And the three interfere: separately they are worth 16, 14.5 and 16, which is 46.5, and
//! together they are worth 42.5. Four bits go missing in the sharing.
//!
//! # Where the four bits go, which is not where this file said
//!
//! This file used to guess that the answers were read off adjacent rows and so came out of very
//! nearly the same cells. Both halves of that are now measured and the second one is wrong.
//!
//! `apart` moves the answers onto rows nought, three and six instead of nought, one and two. Over
//! eight starts it is worth 41.6, 42.5 and 42.1 of 48 at one, two and three rows apart, which is
//! not monotone and inside the spread of the starts. Spacing them changes nothing.
//!
//! Sharing is real and it is not the cost. Tracing each answer's cone back from the row it is read
//! off with the three of them three rows apart, they share 89% of their gates at twelve columns,
//! 68% at eight and 20% at five, because a cone widens a row a column on each side and eleven
//! columns is wider than eight rows. The grids that share
//! least score worst: 42, 42, 43, 41 at twelve columns against 36, 38, 36, 37 at five. The shared
//! cone is what the answers are made of, not what they lose.
//!
//! What the four bits are spent on is the score itself. Trying every gate in every allowed kind on
//! the finished chip and scoring each answer alone, bit one is short on all four starts — 10, 13,
//! 12 and 12 of 16 while the outer two sit at 14 to 16 — and of the changes that raise it, 24 of
//! 26, 1 of 2, 1 of 1 and 1 of 1 are refused by the total, because the same change costs an outer
//! bit. The interference is in the adding up, not in the silicon.
//!
//! # Three ways of adding the answers up, and what each is worth
//!
//! If the total refuses the repair, the thing to change is the total. `weigh` takes `sum`, which is
//! what it was, `weakest`, where the lowest answer decides and the total only breaks ties, and
//! `dynamic`, where the host writes a weight an answer each batch from how far behind the colony is
//! on it. All three were run on the same twelve starts of `count`, and the card's count and the
//! host's agree on all thirty-six.
//!
//! ```text
//!               bit one       the total     against sum, start by start
//!   sum         12.08 of 16   41.75 of 48
//!   weakest     12.17         41.42         bit one better in 3, worse in 4, level in 5
//!   dynamic     12.75         41.17         bit one better in 4, worse in 1, level in 7
//! ```
//!
//! **`weakest` does nothing measurable.** **`dynamic` moves bits and does not make them**: two
//! thirds of a bit onto the short answer, and rather more than half a bit off the total, which is
//! the same bits arriving somewhere else. It is the only one of the three that ever reached 14 on
//! bit one, and it did it twice — and on one of those starts bit *nought* came out at 10 and the
//! short answer was a different one. The weighting moves which answer is starved.
//!
//! And the ceiling is not the score's to lift. Asked entirely alone — one output, one job, the
//! whole grid and the whole colony — bit one comes out at 14, 14, 14, 14, 15, 14, 15, 14 of 16 on
//! eight starts. It never reaches 16 with nothing else on the chip at all, and no way of adding the
//! three up gets it past 14 with the other two there.
//!
//! # Time as space, which is where the wall gives
//!
//! Counting four bits at once is one function of four inputs. Counting them one at a time is an
//! increment done four times, and the grid already has both halves of that: `steps` cuts the word
//! into pieces that arrive a step apart, and `state` wires rows from the last column back to the
//! first. Nothing here is new code. All of it is the same twelve-by-eight grid.
//!
//! ```text
//!   steps  state  apart              the total       bit one
//!       1      0      3   at once     42.0 of 48     12.3 of 16
//!       2      2      3               42.0           11.0
//!       4      2      3               43.3           12.6
//!       4      3      3               43.8           13.5
//!       4      4      3               44.3           14.0
//!       4      5      3               43.8           13.8
//! ```
//!
//! Six starts each. **Four memory rows is where it stops helping**, and a fifth is worth less than
//! the row it costs.
//!
//! **`apart` turns out to have a job after all.** The answers are read off the last column and the
//! memory rows are wired back from the last column, so at `apart 1` two of the three answers *are*
//! memory bits and have to be an accumulator and a result at once. On `steps 4 state 2` that is
//! 37.0 of 48 against 42.8 with one answer shared and 43.3 with none.
//!
//! **And with a colony behind it the wall goes.** At `steps 4 state 4 apart 3` and 16 384 chips the
//! total is 45.6 and bit one is 15.0 of 16 over six starts; at 65 536 it is 46.2 and 15.2 over
//! four, with one start at 47 of 48. Bit one asked entirely alone never beat 15. Asked together
//! with two others, one piece of the word at a time, it does better than it ever did by itself.
//!
//! The colony is what it costs: 2610 us a generation of host clock at 65 536 against about 80 at
//! 4096, for sixteen times the chips and forty-eight passes a tick instead of twelve.
//!
//! # How little copper a perfect accumulator needs, which is not little
//!
//! `2x2+1` comes out on every start, so the size pressure can be turned up without losing it.
//! Eight starts at 65 536 chips, counting only the chips that scored 128 of 128:
//!
//! ```text
//!                 perfect   smallest
//!   no pressure   8 of 8    209 gates, 462 wires
//!   lean wires    8 of 8    196 gates, 407 wires
//!   lean 1        8 of 8    195 gates, 433 wires
//! ```
//!
//! Unlike the multiplier, pressing here costs no perfect chips at all. **And what is left is not
//! slack.** Breaking one live gate of the smallest into a wire costs the answer 134 times of 196,
//! so two thirds of it is load-bearing one gate at a time.
//!
//! Compressing a solved chip rather than finding a lean one is worse: handed back to the colony,
//! a 488-wire winner comes out at 477 with every chip seeded from it and 449 with a thousand
//! copies, against 407 for a run that was lean from the start.
//!
//! # Two more ways of giving it room, and neither is one
//!
//! The six-input accumulator sits at 243.1 of 256 and its report says where: answers nought and
//! three are 64 of 64 on every start and answers one and two are 56 to 59. So two of the four
//! bands are doing the work.
//!
//! **In time.** Three steps — `a`, then `b`, then `c` — instead of all six bits at once gives 238.8
//! at four state rows and 240.0 at five, against 243.1 for the word arriving whole.
//!
//! **In rows.** `band` now takes a list of sizes rather than one size, so the two bands that are
//! short can be given the rows the two that are finished do not need. The wrap inside a band is a
//! comparison rather than a shift, because uneven bands cannot be divided into with a shift, and an
//! answer is read off its band's top row, which is why `apart` no longer comes into it.
//!
//! ```text
//!   32 rows   8,8,8,8         243.1 of 256
//!   32 rows   7,9,9,7         242.6
//!   32 rows   9,7,7,9         243.1
//!   64 rows   16,16,16,16     240.2
//!   64 rows   8,24,24,8       238.2
//! ```
//!
//! **Rows do not help wherever they are put.** Two given to the short bands and two given to the
//! finished ones come out the same, and sixteen given to the short ones is worse than sharing them
//! out evenly — which is itself worse than not having them. Equal bands reproduce what they gave
//! before the list existed, start for start.
//!
//! # The accumulator solved, by moving the separation into the score
//!
//! `2x2+1` sat at 121.3 of 128 and would not move for the colony, for depth, for the kit or for
//! anything else tried. The report said where: bit nought is 32 of 32 on every start and the six or
//! seven missing bits are spread over the other three, which is the carry chain of the addition.
//! Bit one is worst, at 27 to 29, and it is the hard product bit with a carry term on top of it —
//! `(a0·b1 xor a1·b0) xor (a0·b0 and c)`.
//!
//! Three knobs were measured against it and none of them is the answer. Twelve, eighteen and
//! twenty-four columns give 122.0, 121.5 and 122.6. One, two, four and eight gates turned over by a
//! mutation give 126.1, 126.3, 126.1 and 126.5. `maj` in the kit is worth 0.7 and `mux` costs 11.5.
//!
//! **What was wrong is that `band` separates the copper and not the score.** A chip whose band one
//! is the best in the colony still loses to a chip that is better on the total, and its band one
//! goes down with it. Three changes put the separation into the selection as well:
//!
//! - `apiece` makes each band copy from the neighbour's band only when *that band* is the better
//!   one, reading the two numbers out of `SPLIT` where `select` read one out of `SCORE`.
//! - A chip that took a band has changed, so it goes on the changed list — without that it could
//!   keep a score describing a grid it no longer had.
//! - A redraw keeps the best band from whichever chip held it, so what it carries over is a grid
//!   that nothing in the colony was.
//!
//! ```text
//!                                       of 128    perfect chips
//!   as it was                            121.3    0 of 6
//!   apiece                               125.6    0
//!   apiece, best band kept, renew 8      127.0    2
//!   apiece, best band kept, renew 2      127.8    5
//!   the same at 65 536 chips             128.0    12 of 12
//! ```
//!
//! **Every start, every input, every answer bit**, with the host's count and the card's agreeing on
//! all twelve. It costs 7509 us a generation and redraws itself about sixty times on the way.
//!
//! At 16 384 chips it is seven of twelve. And it carries: `2x2+2`, the six-input accumulator that
//! sat at 227.0 of 256, comes out at 244.0 on the same recipe, still with no perfect chip.
//!
//! # Three ways of being thriftier, and what each of them costs
//!
//! Four bands, each holding the whole word and answering one bit, is the widest arrangement there
//! is. `2x2+1` sits at 121.3 of 128 in it. Three economies were measured against that, six starts
//! each, and all three cost.
//!
//! **Sharing the time instead of the copper.** The same exam cut into five steps on one eight-row
//! grid, the answers folded onto one row or read off separate ones:
//!
//! ```text
//!   folded, hold 3, state 4                     108.6 of 128
//!   folded, hold 3, state 6                     109.1
//!   folded, hold 5, state 6                     105.8
//!   folded, hold 3, state 6, linger             104.6
//!   unfolded, apart 2, state 3, one overlap     107.5
//!   unfolded, apart 2, state 1, no overlap      102.6
//!   unfolded, apart 1, state 3, three overlaps   98.3
//! ```
//!
//! Twelve to twenty-three of 128 worse than the bands, whichever way it is arranged.
//!
//! **Sharing the colony.** A solved two-by-two multiplier put into the drawn colony as a minority:
//!
//! ```text
//!   aliens        0      1     64   1024   16 384
//!   of 128    121.3  121.3  120.8  120.8    115.6
//! ```
//!
//! One alien is swamped and gives the drawn colony's answer to the digit. Sixty-four and a thousand
//! cost half a point. Filling the colony costs 5.7. **There is no fraction at which handing an
//! earlier answer to a later exam is worth anything here.**
//!
//! **Sharing the word.** The bands as stages of a pipeline rather than four copies, with `over` in
//! the kit so what a band knows can cross:
//!
//! ```text
//!                                of 128   crossings kept
//!   every band fed, no over      121.3     0
//!   every band fed, with over    119.5    32.0
//!   two bands fed, with over     117.8    36.8
//!   one band fed, with over      115.8    38.6
//! ```
//!
//! The crossings survive in every one of them and are paid for in every one of them. On an adder,
//! where a starved band would otherwise answer with a constant, the crossing was worth twelve of
//! 48; here nothing is starved unless it is made to be, and making it so costs 5.5.
//!
//! # Where the accumulator breaks, which is one bit after the multiply
//!
//! `mac` at three inputs is one bit times one bit plus one, and it grows from there. Six starts
//! each in four bands of eight, `and or xor not sum` with `renew 8` at 16 384 chips:
//!
//! ```text
//!                                       of its perfect   perfect chips
//!   1x1+1, 3 inputs, 2 answers          16.0 of 16       6 of 6
//!   1x1+2, 4 inputs, 3 answers          47.5 of 48       3
//!   2x2,   4 inputs, 4 answers          63.1 of 64       3
//!   2x2+1, 5 inputs, 4 answers         121.3 of 128      0
//!   2x2+2, 6 inputs, 4 answers         227.0 of 256      0
//! ```
//!
//! **A two-by-two multiply comes out and a two-by-two multiply plus one bit does not.** Sixteen
//! times the colony does not move it: 121.6 of 128 at 65 536 chips against 121.3 at 16 384.
//!
//! # Starving the bands, which is what makes the crossing necessary
//!
//! `over` bought nothing on a multiply-accumulate, and the reason offered was that every band holds
//! the whole word and can work a carry out for itself. `feeds N` shows the word to the first N
//! bands only. Six starts on `add` in three bands:
//!
//! ```text
//!                                    of 48   perfect   crossings
//!   every band fed, no over          47.3    3 of 6    0
//!   one band fed, no over            34.0    0         0
//!   one band fed, with over          46.0    0         30.8
//! ```
//!
//! **34.0 on every one of the six starts, to the point.** A starved band with nothing crossing into
//! it can only answer with a constant, and the constant that fits best is worth exactly that much.
//! With one wire allowed across it recovers 12 of 48 and comes within 1.3 of a chip that was never
//! starved at all.
//!
//! So the crossing is worth twelve of 48 when it is the only way in and nothing when it is not.
//! What it does not buy is size: with `lean wires`, starved and crossing is 280.5 wires against
//! 290.0 fed and crossing, for 45.3 of 48 against 47.6.
//!
//! # Stacking one run on another, which buys nothing either way
//!
//! `keep` writes the winner's grid to a file and `start` fills the colony from it, which is the
//! only way of handing one run's answer to another. Handing a solved two-by-two multiplier to
//! `2x2+1`, six starts:
//!
//! ```text
//!   drawn at random                  121.3 of 128
//!   every chip the kept grid         115.6
//!   KEPT copies of it, spread        120.8
//! ```
//!
//! **Filling the colony with one answer is worse than not stacking at all**, because a colony of
//! identical chips has no spread left for the selection to work on. Spreading `KEPT` copies through
//! a drawn colony — the same shape `renew` uses — gets back to where drawing got to, and no
//! further.
//!
//! The same on a rung where the layout does not change at all: two answer bits of `times` handed to
//! four gives 63.3 of 64 against 63.1 drawn, three perfect chips in six either way.
//!
//! # A multiply-accumulate, which bands help and do not finish
//!
//! `mac` fills the table with the first third of the word times the second, plus the third. At
//! `inputs 6` that is two bits by two bits plus two, sixty-four inputs and a four-bit answer, which
//! is the arithmetic an accelerator is made of. Six starts, `and or xor not sum` with `renew 8`:
//!
//! ```text
//!                                    of 256    inputs entirely right
//!   one band, 4096 chips             200.3     20.8 of 64
//!   four bands of 8, 4096            224.3     38.0
//!   four bands of 8, 16 384          227.0     39.6
//! ```
//!
//! **Bands are worth twenty-four of 256 and no perfect chip.** Sixty-four inputs of six bits is a
//! bigger exam than sixteen of four, and separating the answers does not make it a small one.
//!
//! # One wire that crosses, which buys finding and costs size
//!
//! A band cannot read its neighbour, and a carry is exactly a thing that has to. `over` is the one
//! gate that crosses: it reads the same offset one band up, and it is a wire rather than a gate of
//! its own, so it is the least leakage that can be allowed.
//!
//! **It does not help the thing it was built for.** With it in the kit the multiply-accumulate is
//! 221.8 of 256 against 227.0 without, over six starts, and ten crossings survive in the winner.
//! Every band already holds the whole word, so a band can always work a carry out for itself, and
//! the wire is a convenience the search pays for in width.
//!
//! **On an adder it buys something else.** `add` in bands, twelve starts with `lean wires`:
//!
//! ```text
//!                              perfect chips   smallest one   crossings
//!   and or xor not sum           1 of 12       124 gates      0
//!   and or xor not sum over      8 of 12       141 gates      25.5
//! ```
//!
//! Eight times as likely to come out and bigger when it does. The leak buys finding, not size.
//!
//! # How small a perfect multiplier gets, which is not very
//!
//! Eight starts on the banded multiplier at 65 536 chips, counting only the chips that scored 64 of
//! 64:
//!
//! ```text
//!                 perfect   smallest
//!   no pressure   7 of 8    159 gates, 338 wires
//!   lean wires    5 of 8    147 gates, 303 wires
//!   lean 1        4 of 8    167 gates, 364 wires
//! ```
//!
//! **Pressing on the size costs perfect chips and buys ten per cent.** And it is not short of time:
//! sixteen thousand generations instead of four thousand gives the same chip, 182 gates and 366
//! wires, on the start that was checked.
//!
//! # The full multiplier, solved by giving each answer its own copper
//!
//! The four answers were built from 96 to 100 per cent the same gates. `band` makes that
//! impossible: a band is a chip of its own, its gates cannot read across the boundary, and it gets
//! its own copy of the word and its own bottom row. `apart` puts one answer on each band's top row.
//! The report reads zero shared gates where it read sixty-one.
//!
//! Six starts each, `and or xor not sum` with `renew 8`. **The last three rows want `ROWS` set to
//! 32**, which is the one constant the grid is derived from and is left at eight:
//!
//! ```text
//!                                              of 64    perfect chips
//!   8 rows, one band, 16 384 chips             59.5     0 of 6
//!   32 rows, one band, 16 384                  58.6     0
//!   32 rows, four bands of 8, 4096             62.3     1
//!   32 rows, four bands of 8, 16 384           63.1     3
//!   32 rows, four bands of 8, 65 536           64.0     6
//! ```
//!
//! **Every start, every input, every answer bit.** The host's count and the card's agree, and it
//! costs 4150 us a generation. The smallest of them carries the answer in 159 gates reading 338
//! wires of the 352 cells.
//!
//! Two things this is not. It is not more room: thirty-two rows in one band is worse than eight
//! rows in one band. And it is not a better layout: putting the two operands' bits in a different
//! order — interleaved, or mirrored — gives 58.0 and 58.3 against 59.5 for the order it had.
//!
//! # Within one, which was free before any of this
//!
//! Every one of the four product bits comes out at 16 of 16 on its own. All four at once has not
//! come out once. Six starts each, `and or xor not sum` with `renew 8`:
//!
//! ```text
//!   16 384 chips, asked at once            59.5 of 64
//!   65 536 chips                           59.1
//!   with `weigh dynamic`                   57.8
//!   with `ladder`                          57.5
//!   folded into four steps, `linger 1`     56.8
//!   over 24 columns                        59.8
//!   over 16 rows                           58.6
//!   cut into steps with state              55.1 to 56.8
//! ```
//!
//! **Nothing beats asking for it plainly**, and the report says where the missing bits are: the
//! per-answer line reads `[16, 12, 16, 16]` or `[16, 12, 16, 15]` on start after start. Bit one is
//! four short and every other bit is whole, and bit one alone is sixteen of sixteen. The four
//! answers are built from 96 to 100 per cent the same gates.
//!
//! **Within one it is free.** `slack 1` reads both answers as numbers and asks for the product to
//! within one, and that comes out 16 of 16 on all six starts. It is worth being plain about what
//! that is: those chips are *exactly* right on 0.6 inputs of 16 on average. They are never more
//! than one out and they are almost never right, which is the tolerance being spent rather than a
//! multiplier being approximated.
//!
//! `linger` is what made the folded row of that table possible: a hold blanks the input rows, which
//! asks the chip to remember the question as well as answer it, and with `linger` the question
//! stays put. Folding it also found a fault — the step was clamped to the last slice of the word
//! before a folded exam worked out which answer to catch, so it caught none. The card and the host
//! disagreeing is what said so.
//!
//! # Breeding a clock, and how long a tune it can whistle
//!
//! Folded, the answers are one row read at several steps, so an exam whose answer is the same for
//! every input is an exam about time alone. `0 -> 0101` and `1 -> 0101` asks for a wire that goes
//! high, low, high, low whatever it is shown, which is an oscillator. It comes out 8 of 8 on every
//! start, and the state trace reads 4, 5, 4, 5.
//!
//! It is not limited to two. `010101`, `001001`, `01010101`, `000100010001` and `011010011010` all
//! come out perfect on six starts of six, so a repeating tune of twenty-four steps is no harder
//! than one of four. What is harder is a tune that does not repeat. Eight starts, `state 4`, on the
//! prefixes of one arbitrary string:
//!
//! ```text
//!   length      8    10    12    14
//!   perfect   8/8   5/8   4/8   0/8
//! ```
//!
//! **Length is not what bounds it; being incompressible is.** Twenty-four steps of `011010` is
//! easy and fourteen steps of nothing in particular is out of reach.
//!
//! Some of that is the machine and the rest is the search. A chip with `state` rows has 2^state
//! states, so an arbitrary tune longer than that cannot be played at all: twelve steps at `state 3`
//! is nought of six and at `state 4` is two. But twenty steps at `state 5` is thirty-two states for
//! a twenty-step tune, and it is still nought of six — there the machine can and the search does
//! not.
//!
//! **Being makeable by a small machine is worth nothing.** A four-bit shift register with feedback
//! plays a fifteen-step tune that looks like noise and is exactly what four state rows are for. It
//! comes out at 24.5 of 30 on eight starts and no perfect chip; at `state 5`, none; at 65 536 chips
//! and `state 6` — sixty-four states for a fifteen-step tune — 27.6 of 30 and none. Fifteen steps
//! of Thue-Morse is 26 of 30 on every one of eight starts and none. Fifteen arbitrary steps is
//! 24.4 and none. **Fifteen steps of a five-step tune played three times is thirty of thirty on
//! eight starts of eight.**
//!
//! So it is not the length, not the state, not the colony and not whether some small machine could
//! play it. It is whether the tune repeats inside the window it is asked for.
//!
//! There is nothing to separate here the way `band` separates the answers of a multiplier: folded,
//! every answer comes off one row, so a tune is the case where the sharing is total by
//! construction.
//!
//! Two checks were in the way and both were about rows. `outputs` was capped at seven and answers
//! had to be a row apart; folded there is one answer row and the cap is that an answer packs into
//! one word.
//!
//! # Folding the answers into time, which costs more than the rows it frees
//!
//! Three answers need three rows of the last column. `fold` takes them off one row instead, one a
//! step, over the last `outputs` steps — so the chip has to know which step it is on, and nothing
//! but the state rows can tell it. `feed` already reads the last column at the top of every step,
//! so catching an answer on its way past costs one store and no pass of its own.
//!
//! Six starts on `count`, `steps 4`, the kit with `maj` and `sum`, at the same number of passes on
//! both sides:
//!
//! ```text
//!                                        of 48    inputs entirely right
//!   hold 3, state 4, apart 3             44.1     13.1 of 16
//!   hold 3, state 4, folded              38.0      6.5
//!   hold 3, state 6, folded              38.3      8.1
//!   hold 6, state 4, folded              37.6      7.5
//! ```
//!
//! **It works and it is a bad trade here.** The host's count and the card's agree on every run, so
//! the answers are being caught where they are meant to be; it costs six of 48 and buys two rows,
//! and two more state rows give back half a bit of that.
//!
//! # Sixteen rows, which cost half as much again and buy nothing
//!
//! `ROWS` is the one number the whole file is derived from, so doubling it is a constant. It
//! doubles the cells, the genome and the wire buffer, and it costs 345.4 us a generation on the
//! device against 212.8 for the same forty-eight passes — 1.62 times, for twice the grid.
//!
//! Six starts each at 4096 chips, the kit with `maj` and `sum`:
//!
//! ```text
//!                                          of its perfect     inputs entirely right
//!   times, apart 2                         58.5 of 64         10.6 of 16
//!   times, apart 4                         58.6               11.1
//!   times, steps 2 state 2, apart 4        56.5                9.3
//!   times, steps 2 state 4, apart 4        56.8               10.0
//!   times, steps 4 state 4, apart 4        55.8                9.3
//!   count, apart 3                         43.8 of 48         12.0
//! ```
//!
//! At eight rows and the same colony `times` gives 58.8 and `count` 44.3. **Nothing moved.** Eight
//! starts on the sequential counter at four, eight and twelve state rows give 14.2, 13.7 and 14.0
//! of 16, so the extra memory is not doing anything either.
//!
//! `ROWS` is left at eight. What sixteen rows bought was the measurement that says the multiplier
//! was never short of rows.
//!
//! # A gate that adds, which is the half of an adder that was missing
//!
//! `maj` of the wire above, the wire below and the wire straight ahead is the carry out of a full
//! adder. `sum` is the exclusive-or of the same three, which is the other output, so a cell of each
//! is one bit of addition and nothing in the kit before this could add at all.
//!
//! Counting is adding, and it shows. Six starts, `count` asked all at once, 16 384 chips:
//! `and or xor not maj` gives 43.0 of 48 and the same kit with `sum` gives 44.3, which is the most
//! this exam has ever given without being cut into steps. Twelve starts on the sequential counter
//! give 14.1 of 16 inputs entirely right against 13.0 for the four-gate kit — and no perfect chip
//! in the twelve, where the four-gate kit found one.
//!
//! **It does nothing for the multiplier.** Six starts on `times` give 59.5 of 64 with it and 59.5
//! without. Multiplying is not adding three wires together, whatever counting is.
//!
//! # Stretching the memory, and why its length is free
//!
//! `hold N` runs N further steps after the word has arrived, with the input rows at nought, so
//! whatever the chip still knows has to be in the state rows. Six starts on the sequential counter:
//!
//! ```text
//!   hold      0     1     2     4     8    16
//!   of 48  44.3  42.8  44.0  43.5  43.3  43.5
//!   exact  13.1  12.1  12.8  12.6  12.3  12.6   of 16 inputs
//! ```
//!
//! **Sixteen empty steps cost nothing measurable**, and that is 176 columns of gates the answer
//! passes through after the last bit of the word. It is not monotone and `hold 1` is the worst of
//! the six, which is the spread of the starts rather than anything about holding.
//!
//! **Because after the first empty step there is nothing left to do.** The report says when the
//! state rows stop changing and when the answer rows do, and on eight starts at `hold 16` the two
//! are the same step every time — 4/4, 3/3, 3/3, 3/3, 3/4, 4/4, 19/19, 3/3 of 20. There is no
//! answer sitting parked while the state churns under it.
//!
//! The report also scores the winner under holds it was never selected under, which is host
//! arithmetic on the same genome. Bred at `hold 16`, four starts:
//!
//! ```text
//!   hold           0    1    2    3    4   16   32
//!   seed 3        28   42   42   42   42   42   42
//!   seed 7        42   42   42   42   42   42   42
//!   seed 11       44   44   44   44   44   44   44
//!   seed 31       41   39   43   39   43   43   43
//! ```
//!
//! **The length of a hold is worth nothing and its existence is worth up to fourteen of 48.** One
//! empty step puts the chip on a fixed point of the blank-input map and every further step is the
//! identity, so two, four, sixteen and thirty-two are the same exam — and nought is a different
//! one, which start 3 loses 42 to 28 on.
//!
//! Start 31 is the exception that names the rule. It is the one whose state was still moving at the
//! last step, and what it found is not a fixed point but a cycle of two: 39 at every odd hold and
//! 43 at every even one. It was bred at sixteen.
//!
//! What it does cost is its passes, which are the whole of the frame: 48 passes a tick at 212.8 us
//! on the device, 96 at 400.4 and 240 at 970.0, the lowest of three starts each.
//!
//! # Asking for the product a bit at a time, and for a shift instead
//!
//! Fewer answer bits than a filler needs is now the low bits of the answer rather than an error, so
//! `times` can be asked for one bit of the product, or two, or three. `shift` is the low half of
//! the word shifted left by the high half, which is the other thing a multiplier is made of.
//! Six starts each, 16 384 chips, `and or xor not sum` with `renew 8`:
//!
//! ```text
//!                      of its perfect   perfect chips
//!   times, outputs 1   16.0 of 16       6 of 6
//!   times, outputs 2   30.6 of 32       2
//!   times, outputs 3   44.0 of 48       0
//!   times, outputs 4   59.5 of 64       0
//!   shift, outputs 2   30.5 of 32       1
//!   shift, outputs 3   44.6 of 48       0
//!   shift, outputs 5   70.1 of 80       0
//! ```
//!
//! **Shifting is not the easier half.** It is within a bit of multiplying at every width, so the
//! detour is not a detour.
//!
//! And 65 536 chips do not change it: `times` at four outputs gives 59.1 of 64 there against 59.5
//! at 16 384. What breaks is not any one bit — each of the four is worth 16 of 16 on its own — but
//! asking for more than about two of them at once.
//!
//! # The price of the third wire, which is nothing on the card and something in silicon
//!
//! A gate that reads three wires is more expensive than one that reads two, everywhere except
//! here. `propagate` works out every arm for every cell of every chip and then picks one, so the
//! arms are the cheap part and the three loads and the store are not.
//!
//! Restricting the kit now stops the forbidden arms being emitted at all, and it makes no
//! difference that can be measured. `gates and` against all six, the lowest device figure of five
//! starts each: 201.42 us a generation against 201.05. The widest kit is not the slower one.
//!
//! So if the third wire is to cost anything the exam has to charge for it. `lean wires` prices a
//! gate at the number of wires it reads — nought for a wire, one for `not`, two for `and`, `or` and
//! `xor`, three for `maj` and `mux` — where `lean 1` charges one for any gate at all. Six starts on
//! the sequential counter with all seven gates in the kit:
//!
//! ```text
//!                 wires read   three-wire gates   inputs entirely right
//!   lean 1        160.0        26.0               13.6 of 16
//!   lean wires    150.1        21.8               13.6
//! ```
//!
//! **Both are down on every one of the six starts and correctness does not move.** The wiring comes
//! in about six per cent cheaper and the three-wire gates about fourteen, for nothing.
//!
//! # Growing a multiplier, which comes out three quarters right
//!
//! `times` fills the table with the low half of the word multiplied by the high half. At
//! `inputs 4` that is two bits by two bits, sixteen inputs and a four-bit product, and `1111` is
//! three times three is nine.
//!
//! ```text
//!                                              of 64      inputs entirely right
//!   and or xor not, 4096 chips                 58.6       10.8 of 16
//!   all seven, 4096                            58.3       11.0
//!   and or xor not maj, 4096                   58.8       11.0
//!   and or xor not maj, 16 384                 59.5       11.5
//!   the same with lean                         59.3       11.3
//!   the same over 18 columns                   59.6       11.6
//!   the same over 24 columns                   59.8       11.8
//!   steps 2 state 2, 16 384                    56.3        8.8
//!   steps 4 state 2, 16 384                    55.1        8.0
//! ```
//!
//! Six starts each. **It does not come out, and neither more depth nor more time is why.** Twice
//! the columns is worth 0.3 of 64, and counting in time — which is what solved `count` — loses
//! three of 64 here.
//!
//! This file first said the reason was that the grid had run out of rows, four answers and two
//! input rows and the bottom row being seven of the eight. That was a guess and it is wrong: at
//! sixteen rows, where `steps 2 state 2 apart 4` puts the state nowhere near an answer, the
//! sequential multiplier is 56.5 of 64 against 58.6 for the same grid asked all at once.
//!
//! **Three of the four product bits come out perfect and one never does.** The report reads
//! `[16, 12, 16, 16]` on start after start, and the short one is bit one, which is
//! `a0·b1` exclusive-or `a1·b0` — the only one of the four that is two levels deep.
//!
//! This file then said that bit, asked entirely alone, comes out at 15 of 16 on all eight starts,
//! and called it a wall. **It is not a wall, it is the kit.** Eight starts, that bit alone:
//!
//! ```text
//!                            renew 0   renew 8
//!   and or xor not           15.8      16.0
//!   and or xor not maj       15.0      15.2
//!   and or xor not sum       16.0
//!   and or xor not maj sum   16.0      16.0
//! ```
//!
//! The four-gate kit was already getting seven perfect chips in eight. **Adding `maj` to it takes
//! that to none**, and adding `sum` — or drawing the colony again — takes it to eight of eight. A
//! gate that can only widen what is reachable made a solved exam unsolvable, and the eight starts
//! that all said 15 were eight starts run with it in the kit.
//!
//! Each of the four product bits alone comes out at 16 of 16 on all six starts with
//! `and or xor not sum` and `renew 8`. Every part of the multiplier is solved; the multiplier is
//! not.
//!
//! # Widening the kit, where what matters is the third wire and not the gate
//!
//! Every gate up to here read two wires: `and`, `or` and `xor` take the diagonal above and the
//! diagonal below, and `not` and `wire` take the one straight ahead. None of them mixed the two.
//! `maj` is the majority of all three and `mux` passes one diagonal or the other depending on the
//! wire straight ahead, and both of them do.
//!
//! On the exam solved by nothing — `count` at once, `apart 3`, 4096 chips, eight starts:
//!
//! ```text
//!                                the total      bit one
//!   and or xor not               42.1 of 48     12.3 of 16
//!   and or xor not maj           42.2           14.2
//!   and or xor not mux           41.6           14.1
//!   all seven                    41.1           14.0
//!   maj alone                    33.0           10.0
//! ```
//!
//! **`maj` can also be a trap.** On one product bit of `times` the same kit without it gets seven
//! perfect chips in eight and with it none — see the multiplier below.
//!
//! **The two macro gates are worth the same, which says it is not the gate.** A cell could always
//! read the wire straight ahead and it could always read the two diagonals; what it could not do is
//! use them together, and doing that is worth about two bits on the answer nothing else moved. The
//! total does not move with it, which is the redistribution the score always makes.
//!
//! At 16 384 chips `and or xor not maj` gives 43.0 and bit one 14.8, past the 14.25 that bit one
//! managed with a whole grid and a whole colony to itself.
//!
//! **`maj` alone is 33.0 of 48 on every one of the eight starts.** Majority is monotone, so parity
//! is not among the things it can build and every start walks to the same place.
//!
//! It buys nothing on the sequential counter: twelve starts give 13.4 of 16 inputs entirely right
//! with all seven gates against 13.0 with four, and the perfect chip came from the four.
//!
//! # The exam solved, eleven times in twelve
//!
//! A run is over long before it stops. On the sequential counter the best reaches 46 of 48 by
//! generation 40 and does not move for the remaining 3960, so what a run is worth is one draw from
//! the starting grids — not the four thousand generations it then spends on it.
//!
//! `renew N` draws the whole colony again after N batches with no improvement, and keeps the best.
//! Two things had to be right before it kept anything:
//!
//! - **The scores go with the genomes.** `select` runs at the top of a frame and reads the scores
//!   the last one left, so fresh grids carrying their predecessors' numbers were copied over the
//!   one chip worth keeping. 46 of 48 became 42.
//! - **One copy of the best does not survive a generation.** Every chip is a mutant of itself or
//!   its neighbour and the best is no exception. 46 became 43 until `KEPT` copies were spread
//!   across the colony instead of one.
//!
//! With both right, eight starts at 4096 chips:
//!
//! ```text
//!   renew        0      2      4      8
//!   redraws      0     85     42     20
//!   of 16     14.3   14.7   14.8   15.0     inputs entirely right
//!   perfect    1/8    1/8    1/8    1/8
//! ```
//!
//! **It lifts the floor and not the ceiling**, and the ceiling is one input: on four starts of five
//! the single input a near-perfect chip gets wrong is `1111`, four ones, the one input of sixteen
//! where the count has to reach four. Which answer bit goes wrong there varies by start.
//!
//! The colony is what gets past it. `renew 8` at 4096, 16 384 and 65 536 chips gives 1, 2 and 7
//! perfect chips in eight starts, and twelve starts at 65 536 give **eleven perfect and one at 15
//! of 16** — mean 15.9 — with the host's count and the card's agreeing on all twelve.
//!
//! It costs 2198 us a generation of host clock at that colony against about 220 at 4096.
//!
//! # The one that was found first, once in twelve
//!
//! With `lean` on, being right is where the search starts rather than where it stops, and on one
//! start in twelve that is enough. `inputs 4 / outputs 3 / apart 3 / steps 4 / state 4 / lean 1 /
//! count` came out at 48 of 48 — every one of the sixteen inputs, all three answer bits, the host's
//! count and the card's agreeing — in 48 gates. The state rows read 6, 4, 2 as the ones arrive, so
//! what the search built is a counter, in an encoding nobody would have chosen.
//!
//! Twelve starts give 14, 14, 16, 12, 11, 14, 13, 13, 12, 14, 12, 12 inputs entirely right, which
//! is 13.0 of 16 and one perfect. Sixteen thousand chips instead of four gives 13.6 and still one.
//!
//! # Embracing the error, which buys a guarantee and not a saving
//!
//! A score of one a bit does not care how wrong it is. `slack 1` reads both answers as binary
//! numbers and counts an input when they are within one of each other, so the exam stops asking
//! for right and starts asking for close. Six starts each, same grid:
//!
//! ```text
//!                       entirely right   the worst it was ever out
//!   one a bit           13.1 of 16       2.8
//!   slack 1              5.1             1 on every start
//!   slack 2              6.1             2 on every start
//! ```
//!
//! **The bound is the thing that was bought.** A chip scored a bit at a time is right more often
//! and, when it is wrong, is wrong by nearly three — a count of four reported as one. A chip scored
//! within one is never once more than one out, on any input, on any start.
//!
//! It is a little cheaper too, and less than the usual claim: with `lean` on, 49.1 gates against
//! 57.3, for 6.3 inputs entirely right against 13.5.
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
/// How many batches without improvement before the ladder moves on anyway.
const PATIENCE: u32 = 20;

/// How many generations one fault pattern stands before another is drawn. A power of two.
const SETTLED: u32 = 64;

/// How many ticks between one migration and the next. A power of two.
const MIGRATE: u32 = 64;

/// How many kinds of gate there are.
const KINDS: u32 = 9;

/// How many gates a mutation turns over at once.
const MUTATIONS: u32 = 1;

/// How many generations to run.
const GENERATIONS: u32 = 4_000;
/// How many go into one submission.
const BATCH: u32 = 20;

/// How many copies of the best chip a redraw spreads across the colony.
///
/// **One is not enough.** Every chip is a mutant of itself or its neighbour, the best included, so
/// a single copy is gone the generation after it is made.
const KEPT: u32 = 64;

/// How many words every kernel declares.
const BINDINGS: u32 = 11;

/// Four words: the tick and the parity.
const PARAMS: u32 = 0;

/// Where in [`PARAMS`] the per-answer weights start, after the tick and the parity.
const WEIGHTS: u32 = 4;
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

/// What each answer alone is worth, one word an answer a chip.
///
/// **The score is one number and the three answers behind it are not.** Anything that weighs the
/// answers against each other has to see them apart, and so does anything that reports on one.
const SPLIT: u32 = 9;

/// The answer bits a folded exam caught on their way past, one a step.
///
/// **The last chip's block is where the writes that are not answers go.** Every row of a step
/// stores here and only the answer row's value is wanted, so the others are addressed past the end
/// rather than made conditional, which would be a read-modify-write between them.
const SAID: u32 = 10;

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

/// Whichever of the two is less.
///
/// `min` is on the lane API where subtraction is not, so this is the one comparison that needs no
/// working around.
fn smallest(kernel: &mut Kernel<U32>, a: Id, b: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let one = lanes.splat_id::<U32, 1>(a)?;
    let other = lanes.splat_id::<U32, 1>(b)?;
    Ok(lanes.min(one, other)?.id())
}

/// Whichever of the two is more.
fn largest(kernel: &mut Kernel<U32>, a: Id, b: Id) -> Result<Id, LaneError> {
    let mut lanes = kernel.lanes()?;
    let one = lanes.splat_id::<U32, 1>(a)?;
    let other = lanes.splat_id::<U32, 1>(b)?;
    Ok(lanes.max(one, other)?.id())
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
    exam: &Exam,
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
    // **What is ahead is left alone.** `above` is a strict `>`, so a chip that beats its neighbour
    // keeps what it has; a chip that drew is mutated but copies nothing, and one that lost copies
    // and is mutated. This paragraph used to say a draw was left alone as well, which the line
    // below has never done.
    //
    // **Except on the first tick**, where every score is nought, nobody is better than anybody, and
    // so nothing would ever be put on the list to be evaluated at all — which is a colony that sits
    // at nought for ever, and is what this did before the line below.
    // **A band's own verdict, where the exam asked for one.** The cell's band is known while the
    // kernel is being written, so each band's copy decision reads that band's own two numbers out
    // of `SPLIT` instead of the chip's one out of `SCORE`.
    let mut apiece = Vec::new();
    if exam.apiece {
        let wide = kernel.module().constant_u32(exam.outputs)?;
        let ours_split = kernel.module().i_mul(uint, who, wide)?;
        let theirs_split = kernel.module().i_mul(uint, neighbour, wide)?;
        for which in 0..exam.outputs {
            let off = kernel.module().constant_u32(which)?;
            let here = kernel.module().i_add(uint, ours_split, off)?;
            let there = kernel.module().i_add(uint, theirs_split, off)?;
            let mine_now = kernel.load_at(SPLIT, here)?;
            let theirs_now = kernel.load_at(SPLIT, there)?;
            let better = above(&mut kernel, theirs_now, mine_now)?;
            apiece.push(kernel.module().i_mul(uint, my_turn, better)?);
        }
    }

    let first = is(&mut kernel, tick, one)?;
    let ahead = above(&mut kernel, ours, theirs)?;
    let behind = minus(&mut kernel, one, ahead)?;
    let churn = kernel.module().i_mul(uint, my_turn, behind)?;
    let mut either = kernel.module().i_add(uint, churn, first)?;
    // **A band that was copied is a chip that changed.** Without this a chip whose total was ahead
    // could take a band from its neighbour and stay off the changed list, keeping a score that
    // describes a grid it no longer has.
    for &taken in &apiece {
        either = kernel.module().i_add(uint, either, taken)?;
    }
    let restless = at_most(&mut kernel, either, 1)?;

    // **One gate at a time**, which is what `MUTATIONS` says and what this paragraph used to
    // contradict. On `2x2+1` in four bands, six starts each, one, two, four and eight turned over
    // at once give 126.1, 126.3, 126.1 and 126.5 of 128 — the number does not matter there.
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
        let which = banded(&exam.band, cell % ROWS).0;
        let take = apiece.get(which).copied().unwrap_or(take);
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
    exam: &Exam,
) -> Result<Vec<u32>, LaneError> {
    let (fan, state) = (exam.bits / exam.steps, exam.state);
    let (first, blank) = (step == 0, step >= exam.steps && !exam.linger);

    // Lingering, every step past the word repeats its last slice rather than arriving at nought.
    // **This is the slice and not the step**: which answer a folded exam catches here is a question
    // about the step, and clamping the two together silently caught nothing.
    let slice = step.min(exam.steps - 1);
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, colony, held)?;
    // Every row question is a question about where a row sits in its band, not the grid.
    let (which_band, _, size, inside) = where_in(&mut kernel, row, &exam.band)?;

    // This step's slice of the word: bit `step * fan + row` of the input. **After `hold` has
    // started there is no slice left**, and the input rows arrive at nought so that everything the
    // chip still knows is in the state rows and nowhere else.
    let uint = kernel.index_type();
    let arriving = if blank {
        none
    } else {
        let along = kernel.module().constant_u32(slice * fan)?;
        let reach = kernel.module().i_add(uint, inside, along)?;
        let shifted = shift_down(&mut kernel, case, reach)?;
        low_bits(&mut kernel, shifted, 1)?
    };

    // What was on this row when the last step finished, or nought where there was no last step.
    let carried = if first {
        none
    } else {
        let at = wire(&mut kernel, chip, COLUMNS - 1, row, case, cases)?;
        kernel.load_at(WIRE, at)?
    };

    // **Folded, this step's read of the last column is also an answer.** `feed` already has it, so
    // catching it here costs one store and no pass of its own.
    let total = exam.steps + exam.hold;
    let caught = step.checked_sub(1).filter(|_| exam.fold).and_then(|was| {
        was.checked_sub(total - exam.outputs).filter(|slot| *slot + 1 < exam.outputs)
    });
    if let Some(slot) = caught {
        let uint = kernel.index_type();
        let wide = kernel.module().constant_u32(exam.outputs * cases)?;
        let base = kernel.module().i_mul(uint, chip, wide)?;
        let along = kernel.module().constant_u32(slot * cases)?;
        let held = kernel.module().i_add(uint, base, along)?;
        let mine = kernel.module().i_add(uint, held, case)?;
        let spare = kernel.module().constant_u32(colony * exam.outputs * cases)?;
        let top = is(&mut kernel, row, none)?;
        let at = pick(&mut kernel, top, mine, spare)?;
        kernel.store_at(SAID, at, carried)?;
    }

    let fan_at = kernel.module().constant_u32(fan)?;
    let mut feeding = above(&mut kernel, fan_at, inside)?;
    // **A starved band's input rows are not input rows.** Everything it is going to know has to
    // arrive over the boundary.
    if exam.feeds > 0 {
        let many = kernel.module().constant_u32(exam.feeds)?;
        let fed = above(&mut kernel, many, which_band)?;
        feeding = kernel.module().i_mul(uint, feeding, fed)?;
    }
    let past = kernel.module().constant_u32(fan + state)?;
    let within = above(&mut kernel, past, inside)?;
    let not_input = minus(&mut kernel, one, feeding)?;
    let remembering = kernel.module().i_mul(uint, within, not_input)?;
    let bottom = minus(&mut kernel, size, one)?;
    let lowest = is(&mut kernel, inside, bottom)?;

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
    allowed: &[u32],
    band: &[u32],
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let held = kernel.load_at(COUNTS, none)?;
    let (chip, row, case) = wire_at(&mut kernel, subgroup, cases, colony, held)?;

    // **The wrap is inside the band.** The top row of a band reads the bottom row of the same band
    // and never the row above it, so no signal crosses a boundary in either direction.
    let (_, base, size, inside) = where_in(&mut kernel, row, band)?;
    let up = {
        let last = minus(&mut kernel, size, one)?;
        let before = minus(&mut kernel, inside, one)?;
        let at_top = is(&mut kernel, inside, none)?;
        let held = pick(&mut kernel, at_top, last, before)?;
        kernel.module().i_add(uint, base, held)?
    };
    let down = {
        let after = kernel.module().i_add(uint, inside, one)?;
        let over_edge = is(&mut kernel, after, size)?;
        let held = pick(&mut kernel, over_edge, none, after)?;
        kernel.module().i_add(uint, base, held)?
    };
    let at_a = wire(&mut kernel, chip, column - 1, up, case, cases)?;
    let at_b = wire(&mut kernel, chip, column - 1, down, case, cases)?;
    let at_w = wire(&mut kernel, chip, column - 1, row, case, cases)?;
    // **The one thing that crosses a band.** Everything else wraps inside its own band; this reads
    // the same offset one band up, which is where a carry has to come from.
    let over_at = {
        let back = kernel.module().constant_u32(ROWS - band.first().copied().unwrap_or(ROWS))?;
        let raised = kernel.module().i_add(uint, row, back)?;
        let held = low_bits(&mut kernel, raised, ROWS.trailing_zeros())?;
        wire(&mut kernel, chip, column - 1, held, case, cases)?
    };
    let a = kernel.load_at(WIRE, at_a)?;
    let crossing = if allowed.contains(&8) { Some(kernel.load_at(WIRE, over_at)?) } else { None };
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
    // **The two that read three wires.** Everything above is a gate of the wire above and the wire
    // below; these two also read the wire straight ahead, which was only ever a thing to invert.
    // **A gate the exam forbids is not emitted at all.** Every arm here is worked out for every
    // cell of every chip whether the genome asks for it or not, so an arm nothing can select is
    // the whole grid's time spent on nothing.
    let most = if allowed.contains(&5) {
        let with_a = smallest(&mut kernel, a, straight)?;
        let with_b = smallest(&mut kernel, b, straight)?;
        let either_with = largest(&mut kernel, with_a, with_b)?;
        Some(largest(&mut kernel, both, either_with)?)
    } else {
        None
    };
    let chosen = if allowed.contains(&6) {
        Some(pick(&mut kernel, straight, a, b)?)
    } else {
        None
    };
    // **The other half of a full adder.** `maj` of the same three wires is the carry out, so a cell
    // of each is one bit of addition, and nothing before this could add.
    let carried = if allowed.contains(&7) {
        let two = kernel.module().i_add(uint, sum, straight)?;
        Some(low_bits(&mut kernel, two, 1)?)
    } else {
        None
    };

    let span = kernel.module().constant_u32(CELLS)?;
    let mine_at = kernel.module().i_mul(uint, chip, span)?;
    let cell = kernel.module().constant_u32((column - 1) * ROWS)?;
    let along = kernel.module().i_add(uint, mine_at, cell)?;
    let gene_at = kernel.module().i_add(uint, along, row)?;
    let kind = kernel.load_at(GENE, gene_at)?;

    let mut answer = straight;
    let arms = [(1, Some(both)), (2, Some(either)), (3, Some(odd)), (4, Some(inverted)), (5, most),
        (6, chosen), (7, carried), (8, crossing)];
    for (number, value) in arms.into_iter().filter_map(|(at, held)| {
        held.filter(|_| allowed.contains(&at)).map(|held| (at, held))
    }) {
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
fn tally(subgroup: u32, cases: u32, colony: u32, exam: &Exam) -> Result<Vec<u32>, LaneError> {
    let Exam { outputs, lean, weigh, slack, fold, .. } = *exam;
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
    let uint = kernel.index_type();
    let none = kernel.module().constant_u32(0)?;
    let held = kernel.load_at(COUNTS, none)?;
    let slot = mine(&mut kernel, subgroup)?;
    let chip = listed(&mut kernel, slot, held, colony)?;

    // **One tap a row, down the right-hand edge.** Output nought is the top wire of the last column,
    // output one the wire under it, and so on — the same place a one-bit exam reads, extended.
    let mut apiece = Vec::with_capacity(outputs as usize);
    for output in 0..outputs {
        let row = kernel.module().constant_u32(if fold { 0 } else { exam.answer_row(output) })?;
        let mut got = kernel.module().constant_u32(0)?;
        for case in 0..cases {
            let which = kernel.module().constant_u32(case)?;
            let want = kernel.load_at(TARGET, which)?;
            let asked = kernel.load_at(KEEP, which)?;
            // Folded, every answer but the last was caught by `feed` on the step it went past.
            let said = if fold && output + 1 < outputs {
                let wide = kernel.module().constant_u32(outputs * cases)?;
                let base = kernel.module().i_mul(uint, chip, wide)?;
                let along = kernel.module().constant_u32(output * cases + case)?;
                let at = kernel.module().i_add(uint, base, along)?;
                kernel.load_at(SAID, at)?
            } else {
                let at = wire(&mut kernel, chip, COLUMNS - 1, row, which, cases)?;
                kernel.load_at(WIRE, at)?
            };
            // **Both sides carry a mask.** A bit the exam left out counts for nothing either way, on
            // the input side and the output side alike, so a partial specification gives a partial
            // score rather than a wrong one.
            let wanted = bit(&mut kernel, want, output)?;
            let marked = bit(&mut kernel, asked, output)?;
            let hit = is(&mut kernel, said, wanted)?;
            let counted = kernel.module().i_mul(uint, hit, marked)?;
            got = kernel.module().i_add(uint, got, counted)?;
        }
        // Written down apart as well as added up, because the host reports on them one at a time.
        let lane = kernel.module().constant_u32(outputs)?;
        let base = kernel.module().i_mul(uint, chip, lane)?;
        let off = kernel.module().constant_u32(output)?;
        let where_ = kernel.module().i_add(uint, base, off)?;
        kernel.store_at(SPLIT, where_, got)?;
        apiece.push(got);
    }
    let mut summed = kernel.module().constant_u32(0)?;
    for &got in &apiece {
        summed = kernel.module().i_add(uint, summed, got)?;
    }
    // **Approximate computing.** Above nought `slack` reads both answers as binary numbers and
    // counts the case when they are within it of each other, so a chip that is never exactly right
    // and never far wrong scores where an exact one scores nothing.
    let mut near = kernel.module().constant_u32(0)?;
    if slack > 0 {
        let ceiling = kernel.module().constant_u32(slack)?;
        for case in 0..cases {
            let which = kernel.module().constant_u32(case)?;
            let want = kernel.load_at(TARGET, which)?;
            let mut says = kernel.module().constant_u32(0)?;
            for output in 0..outputs {
                let row = kernel.module().constant_u32(exam.answer_row(output))?;
                let at = wire(&mut kernel, chip, COLUMNS - 1, row, which, cases)?;
                let said = kernel.load_at(WIRE, at)?;
                let place = kernel.module().constant_u32(1 << output)?;
                let scaled = kernel.module().i_mul(uint, said, place)?;
                says = kernel.module().i_add(uint, says, scaled)?;
            }
            let wanted = low_bits(&mut kernel, want, outputs)?;
            let over = minus(&mut kernel, says, wanted)?;
            let under = minus(&mut kernel, wanted, says)?;
            let gap = largest(&mut kernel, over, under)?;
            // `min(gap, slack) == gap` is `gap <= slack`, which is the comparison the lanes have.
            let held = smallest(&mut kernel, gap, ceiling)?;
            let within = is(&mut kernel, held, gap)?;
            near = kernel.module().i_add(uint, near, within)?;
        }
    }
    let fit = match weigh {
        // **The weakest link.** The lowest answer decides and the total only breaks ties, so a
        // change that lifts the answer nobody can do is taken even when it costs one that works.
        1 => {
            let mut worst = apiece.first().copied().unwrap_or(summed);
            for &got in apiece.iter().skip(1) {
                worst = smallest(&mut kernel, worst, got)?;
            }
            let span = kernel.module().constant_u32(cases * outputs + 1)?;
            let raised = kernel.module().i_mul(uint, worst, span)?;
            kernel.module().i_add(uint, raised, summed)?
        }
        // **Dynamic weighting.** The host writes a weight an answer each batch, from how far behind
        // the colony is on it, so what the score is made of moves while the search runs.
        2 => {
            let mut held = kernel.module().constant_u32(0)?;
            for (output, &got) in apiece.iter().enumerate() {
                let at = kernel
                    .module()
                    .constant_u32(WEIGHTS + u32::try_from(output).unwrap_or(0))?;
                let weight = kernel.load_at(PARAMS, at)?;
                let scaled = kernel.module().i_mul(uint, got, weight)?;
                held = kernel.module().i_add(uint, held, scaled)?;
            }
            held
        }
        _ if slack > 0 => near,
        _ => summed,
    };
    let final_score = if lean > 0 {
        let budget = if lean == 2 { CELLS * price(5) } else { CELLS };
        let span = kernel.module().constant_u32(CELLS)?;
        let base = kernel.module().i_mul(uint, chip, span)?;
        let one_ = kernel.module().constant_u32(1)?;
        let three = kernel.module().constant_u32(price(5))?;
        let five = kernel.module().constant_u32(5)?;
        let two = kernel.module().constant_u32(2)?;
        let mut spent = kernel.module().constant_u32(0)?;
        for cell in 0..CELLS {
            let off = kernel.module().constant_u32(cell)?;
            let at = kernel.module().i_add(uint, base, off)?;
            let kind = kernel.load_at(GENE, at)?;
            // A wire is not a gate, so it is what is left over rather than what is spent.
            let held = smallest(&mut kernel, kind, one_)?;
            let a_gate = is(&mut kernel, held, one_)?;
            let cost = if lean == 2 {
                // Two wires for `and`, `or` and `xor`, one for `not`, three for `maj` and `mux`.
                let capped = smallest(&mut kernel, kind, three)?;
                let low = is(&mut kernel, capped, kind)?;
                let second = kernel.module().i_mul(uint, a_gate, low)?;
                let far = smallest(&mut kernel, kind, five)?;
                let wide = is(&mut kernel, far, five)?;
                let third = kernel.module().i_mul(uint, wide, two)?;
                let held = kernel.module().i_add(uint, a_gate, second)?;
                kernel.module().i_add(uint, held, third)?
            } else {
                a_gate
            };
            spent = kernel.module().i_add(uint, spent, cost)?;
        }
        let ceiling = kernel.module().constant_u32(budget)?;
        let spare = minus(&mut kernel, ceiling, spent)?;
        let weight = kernel.module().constant_u32(budget + 1)?;
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
#[derive(Clone)]
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
    /// How many rows apart the answers are read. One is next to each other, which is what it was.
    ///
    /// **It is worth nothing while the chip has no memory.** Eight starts on `count` give 41.6,
    /// 42.5 and 42.1 of 48 at one, two and three apart.
    ///
    /// **With memory it is the difference between an answer and an accumulator.** The state rows
    /// are wired back from the last column, which is where the answers are read, so on
    /// `steps 4 state 2` the three spacings give 37.0, 42.8 and 43.3 of 48.
    apart: u32,
    /// How the answers are added into one score: nought the plain total, one the weakest of them,
    /// two a weight an answer the host rewrites each batch.
    ///
    /// **The total is what refuses the repair.** Of the changes that raise the short answer of
    /// `count`, 24 of 26, 1 of 2, 1 of 1 and 1 of 1 lower the total on the four starts measured, so
    /// the other two ask a different question of the same colony.
    ///
    /// **What they buy is a redistribution.** Over twelve starts `weakest` is worth +0.08 on the
    /// short answer and −0.33 on the total, and `dynamic` +0.67 and −0.58.
    weigh: u32,
    /// How far off the answer may be and still count, as a number rather than as bits.
    ///
    /// **Nought is exact and is what it was.** Above nought the score is one a case instead of one
    /// a bit: the chip's answer and the wanted answer are read as binary numbers and the case
    /// counts when they are within this of each other.
    slack: u32,
    /// Whether the exam is revealed one output bit at a time instead of all at once.
    ladder: bool,
    /// Which gates the search draws from. Wire is always in it, because a grid needs somewhere
    /// for a signal to go.
    allowed: Vec<u32>,
    /// One gate in this many is broken each generation, or nought for none. A power of two.
    noise: u32,
    /// What a chip is charged for, among equally correct ones: nought nothing, one a gate a gate,
    /// two a wire a wire.
    ///
    /// **A three-wire gate is free on the card and it is not free in silicon.** One gate in the kit
    /// and six give the same floor, 201.42 against 201.05 us a generation on the device, because
    /// the pass is bound by the three loads and the store and not by which arm it picks. Charging
    /// by wires is the only place the difference can be made to exist.
    lean: u32,
    /// How many pieces the input word is cut into, one arriving a step.
    steps: u32,
    /// After how many batches with no improvement the colony is drawn again, or nought for never.
    ///
    /// **A run is over long before it stops.** On the sequential counter the best reaches 46 of 48
    /// by generation 40 and does not move for the remaining 3960, so what a run is worth is one
    /// draw from the starting grids and not the four thousand generations it then spends.
    /// Redrawing everything but the best turns the one draw into many without losing the best.
    renew: u32,
    /// Whether a band is selected on its own answer instead of on the chip's total.
    ///
    /// **Bands separate the copper and not the score.** A chip whose band one is the best in the
    /// colony still loses if its band three is poor, and its band one goes with it. With this, a
    /// band copies from the neighbour's band only when that band is the better one.
    apiece: bool,
    /// How many bands are shown the word, or nought for all of them.
    ///
    /// **A band that is fed can work anything out for itself.** Starving the rest is what makes a
    /// wire across the boundary necessary rather than convenient.
    feeds: u32,
    /// Where to write the winner's grid, so that a later exam can start from it.
    ///
    /// **A run is one draw, and this is how a draw is kept.** Evolving a small exam and handing
    /// its answer to a bigger one is the only way of stacking runs there is here.
    keep: Option<String>,
    /// A grid to put into the drawn colony, from an earlier exam's `keep`, and how many copies.
    ///
    /// **A colony of one answer has no spread to select on.** Filling every chip with a kept grid
    /// is worse than not stacking at all, so what is stacked is a minority.
    start: Option<String>,
    /// How many copies of `start` go in, or nought for [`KEPT`].
    aliens: u32,
    /// How many rows each band is, or one band of `ROWS`, which is what it was.
    ///
    /// **They need not be equal.** On the six-input accumulator answers nought and three come out
    /// perfect on every start and one and two do not, so the rows are worth more to some bands than
    /// to others.
    ///
    /// **A band is a chip of its own.** Its gates cannot read across the boundary, it gets its own
    /// copy of the word and its own bottom row, and an answer read off its top row is built out of
    /// cells no other answer can reach. It is what `apart` looked like it was and never was.
    band: Vec<u32>,
    /// Whether the last slice of the word stays on the input rows during a hold.
    ///
    /// **A hold blanks the inputs, which asks the chip to remember the question as well as answer
    /// it.** With this the question stays put and the extra steps are only extra time.
    linger: bool,
    /// Whether the answers come off one row a step instead of off one row each.
    ///
    /// **Folded, the chip has to know which step it is on.** Answer `k` is whatever the top wire of
    /// the last column held at step `steps + hold − outputs + k`, so nothing but the state rows
    /// tells one step from the next.
    fold: bool,
    /// How many further steps run after the word has arrived, with the input rows at nought.
    ///
    /// **This is the only thing that asks the memory to hold rather than to work.** During a hold
    /// nothing arrives, so whatever the chip still knows is in the state rows.
    hold: u32,
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
    /// The most a chip can be charged, which is what the score is packed above.
    fn budget(&self) -> u32 {
        match self.lean {
            0 => 0,
            2 => CELLS * price(5),
            _ => CELLS,
        }
    }

    /// Which row answer `at` is read off.
    ///
    /// **With bands it is the band's top row and `apart` does not come into it**, because uneven
    /// bands do not start at a fixed spacing and a single number cannot say where they start.
    fn answer_row(&self, at: u32) -> u32 {
        if self.band.len() > 1 {
            return self.band.iter().take(at as usize).sum();
        }
        at * self.apart
    }

    /// How many inputs there are altogether.
    fn cases(&self) -> u32 {
        1 << self.bits
    }

    /// How many answer bits are marked, which is what a perfect score is — or how many cases there
    /// are, when `slack` scores a whole answer at a time instead of a bit at a time.
    fn asked(&self) -> u32 {
        if self.slack > 0 {
            return self.cases();
        }
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
///   apart 3         read the answers three rows apart instead of next to each other
///   ladder 1        reveal one output bit at a time, widening when it is mastered
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
    let mut apart = 1_u32;
    let mut ladder = false;
    let mut weigh = 0;
    let mut slack = 0;
    let mut hold = 0;
    let mut fold = false;
    let mut linger = false;
    let mut band: Vec<u32> = vec![ROWS];
    let mut feeds = 0;
    let mut apiece = false;
    let mut keep = None;
    let mut start = None;
    let mut aliens = 0;
    let mut renew = 0;
    let mut allowed: Option<Vec<u32>> = None;
    let mut noise = 0_u32;
    let mut lean = 0;
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
            (Some("apart"), _) => {
                apart = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .filter(|&it| (1..ROWS).contains(&it))
                    .ok_or_else(|| say("`apart` wants a number from one to seven"))?;
            }
            (Some("slack"), _) => {
                slack = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`slack` wants a number"))?;
            }
            (Some("renew"), _) => {
                renew = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`renew` wants a number of batches"))?;
            }
            (Some("apiece"), _) => {
                apiece = word.next() != Some("0");
            }
            (Some("feeds"), _) => {
                feeds = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`feeds` wants a number of bands"))?;
            }
            (Some("keep"), _) => {
                keep = word.next().map(str::to_owned);
            }
            (Some("start"), _) => {
                start = word.next().map(str::to_owned);
                aliens = word.next().and_then(|it| it.parse().ok()).unwrap_or(0);
            }
            (Some("band"), _) => {
                let held: Vec<u32> = word
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|it| it.parse().ok())
                    .collect();
                if held.iter().sum::<u32>() != ROWS || held.iter().any(|it| *it < 2) {
                    return Err(say("`band` wants sizes of two or more that add up to the rows"));
                }
                band = held;
            }
            (Some("linger"), _) => {
                linger = word.next() != Some("0");
            }
            (Some("fold"), _) => {
                fold = word.next() != Some("0");
            }
            (Some("hold"), _) => {
                hold = word
                    .next()
                    .and_then(|it| it.parse::<u32>().ok())
                    .ok_or_else(|| say("`hold` wants a number"))?;
            }
            (Some("ladder"), _) => {
                ladder = word.next() != Some("0");
            }
            (Some("weigh"), _) => {
                weigh = match word.next() {
                    Some("sum") => 0,
                    Some("weakest") => 1,
                    Some("dynamic") => 2,
                    _ => return Err(say("`weigh` wants sum, weakest or dynamic")),
                };
            }
            (Some("gates"), _) => {
                let mut kinds = vec![0_u32];
                for name in word.by_ref() {
                    let kind = match name {
                        "wire" => 0,
                        "and" => 1,
                        "or" => 2,
                        "xor" => 3,
                        "maj" => 5,
                        "mux" => 6,
                        "sum" => 7,
                        "over" => 8,
                        "not" => 4,
                        _ => return Err(say("a gate is wire, and, or, xor, not, maj, mux, sum or over")),
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
                lean = match word.next() {
                    Some("0") => 0,
                    Some("wires") => 2,
                    _ => 1,
                };
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
                    // Rows are what bounds this unfolded, and the row check below says so;
                    // folded, the bound is that an answer is packed into one word.
                    .filter(|&it| (1..32).contains(&it))
                    .ok_or_else(|| say("`outputs` wants a number from one to thirty-one"))?;
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
            (Some(filler @ ("parity" | "majority" | "equal" | "count" | "add" | "times" | "shift"
                | "mac")), Some(how)) => {
                let widest = match filler {
                    "count" => u32::BITS - how.leading_zeros(),
                    "add" => how / 2 + 1,
                    "times" => how,
                    "shift" => how / 2 + (1 << (how / 2)) - 1,
                    "mac" => 2 * (how / 3),
                    _ => 1,
                };
                // **Fewer answer bits than the table needs is the low bits of it.** That is what
                // the packing already does, and asking for two bits of a product instead of four
                // is a smaller exam rather than a broken one.
                if outputs == 0 || outputs > widest {
                    return Err(say(&format!(
                        "`{filler}` has {widest} answer bits and `outputs {outputs}` asks for more"
                    )));
                }
                if filler == "mac" && !how.is_multiple_of(3) {
                    return Err(say("`mac` wants a number of inputs divisible by three"));
                }
                if matches!(filler, "add" | "times" | "shift") && how % 2 != 0 {
                    return Err(say(&format!("`{filler}` wants an even number of inputs")));
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
                        "add" => (case & ((1 << half) - 1)) + (case >> half),
                        "times" => (case & ((1 << half) - 1)) * (case >> half),
                        "mac" => {
                            let third = how / 3;
                            let each = |at: u32| (case >> (at * third)) & ((1 << third) - 1);
                            each(0) * each(1) + each(2)
                        }
                        _ => (case & ((1 << half) - 1)) << (case >> half),
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
    if band.iter().min().is_some_and(|it| bits / steps + state >= *it) {
        return Err(format!(
            "{} input rows and {state} state rows do not fit in the narrowest band",
            bits / steps
        ));
    }
    if band.len() > 1 && band.len() != outputs as usize {
        return Err(format!("{} bands and {outputs} answers is not one each", band.len()));
    }
    if bits / steps + state >= ROWS {
        return Err(format!(
            "{} input rows and {state} state rows leave no room for the one on the bottom",
            bits / steps
        ));
    }
    // Folded, every answer comes off the same row, so how many there are is a question about steps
    // rather than about rows.
    if !fold && outputs > 0 && (outputs - 1) * apart >= ROWS - 1 {
        return Err(format!(
            "{outputs} answers {apart} rows apart do not fit above the bottom row"
        ));
    }
    if weigh != 0 && ladder {
        return Err(
            "`weigh` and `ladder` ask opposite things: one levels the answers, the other hides one"
                .to_string(),
        );
    }
    if weigh != 0 && outputs < 2 {
        return Err("`weigh` needs more than one answer to weigh".to_string());
    }
    if apiece && band.len() < 2 {
        return Err("`apiece` wants bands, so that an answer lives in one".to_string());
    }
    if fold && state == 0 {
        return Err(
        "`fold` needs `state`: without it the chip cannot tell one step from the next".to_string(),
        );
    }
    if fold && steps + hold < outputs {
        return Err(format!("`fold` wants {outputs} steps to answer in, and there are {}", steps + hold));
    }
    if hold != 0 && state == 0 {
        return Err("`hold` with no `state` holds nothing: the rows are the only memory".to_string());
    }
    if slack != 0 && weigh != 0 {
        return Err("`slack` scores a whole answer at a time, so there is nothing to weigh".to_string());
    }
    if slack != 0 && ask.iter().any(|it| it.count_ones() != outputs) {
        return Err("`slack` reads the answer as a number, so every bit of it has to be asked for".to_string());
    }
    if slack >= 1 << outputs {
        return Err(format!("`slack {slack}` is wider than the {outputs} bits of the answer"));
    }
    let island = island.unwrap_or(colony).min(colony);
    let allowed = allowed.unwrap_or_else(|| (0..KINDS).collect());
    Ok(Exam {
        bits, outputs, apart, band, feeds, apiece, keep, start, aliens, ladder, weigh, slack,
        allowed, noise, lean, steps, hold, linger,
        fold,
        renew,
        state,
        colony, island, seed, want, ask, name,
    })
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
    let total = exam.steps + exam.hold;
    let mut caught = vec![0_u32; exam.outputs as usize];
    let mut last: Vec<u32> = vec![0; ROWS as usize];
    for step in 0..exam.steps + exam.hold {
        let keep = |row: u32| last.get(row as usize).copied().unwrap_or(0);
        let mut column: Vec<u32> = (0..ROWS)
            .map(|row| {
                let (which, base, size) = banded(&exam.band, row);
                let inside = row - base;
                let fed = exam.feeds == 0 || (which as u32) < exam.feeds;
                if inside < fan && fed {
                    if step >= exam.steps && !exam.linger {
                        0
                    } else {
                        (case >> (step.min(exam.steps - 1) * fan + inside)) & 1
                    }
                } else if inside < fan + exam.state {
                    if step == 0 { 0 } else { keep(row) }
                } else if inside == size - 1 {
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
                    // The wrap is inside the band, so a boundary is as good as an edge.
                    let (_, base, size) = banded(&exam.band, row);
                    let round = |at: u32| base + at % size;
                    let (up, down, straight) = (
                        read(round(row - base + size - 1)),
                        read(round(row - base + 1)),
                        read(row),
                    );
                    match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                        Some(1) => up.min(down),
                        Some(2) => up.max(down),
                        Some(3) => (up + down) & 1,
                        Some(4) => 1 - straight,
                        Some(5) => (up + down + straight) / 2,
                        Some(6) => {
                            if straight == 1 {
                                up
                            } else {
                                down
                            }
                        }
                        Some(7) => (up + down + straight) & 1,
                        Some(8) => read(row + ROWS - exam.band.first().copied().unwrap_or(ROWS)),
                        _ => straight,
                    }
                })
                .collect();
            column = next;
        }
        if let Some(held) = (step + exam.outputs)
            .checked_sub(total)
            .and_then(|slot| caught.get_mut(slot as usize))
        {
            *held = column.first().copied().unwrap_or(0);
        }
        last = column;
    }
    // The answer is the top `outputs` wires after the last step, packed as the exam packs it — or,
    // folded, the top wire at the last `outputs` steps, which `caught` collected on the way.
    if exam.fold {
        return caught
            .iter()
            .enumerate()
            .filter(|&(_, &it)| it == 1)
            .fold(0_u32, |held, (at, _)| held | (1 << at));
    }
    (0..exam.outputs)
        .filter(|&at| last.get(exam.answer_row(at) as usize).copied().unwrap_or(0) == 1)
        .fold(0_u32, |held, at| held | (1 << at))
}
/// How many of the marked answer bits this chip gets right, worked out on the host.
///
/// **One a bit, not one a case**, so a chip right about two outputs of three scores two.
fn scored(genome: &[u32], exam: &Exam) -> u32 {
    if exam.slack > 0 {
        let mask = (1 << exam.outputs) - 1;
        return u32::try_from(
            (0..exam.cases())
                .filter(|&case| {
                    let said = runs(genome, case, exam);
                    let want = exam.want.get(case as usize).copied().unwrap_or(0) & mask;
                    said.abs_diff(want) <= exam.slack
                })
                .count(),
        )
        .unwrap_or(0);
    }
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

/// After which step the state rows stop changing, and on how many inputs they never do.
///
/// **A hold that costs nothing is a hold that is not being asked for anything.** If the state
/// settles while the word is still arriving, everything after that is a latch holding a fixed
/// point, and it would hold for any number of steps rather than for the ones that were measured.
fn settles(genome: &[u32], exam: &Exam, answers: bool) -> (u32, u32) {
    let (mut latest, mut never) = (0, 0);
    for case in 0..exam.cases() {
        let watched = |held: (u32, u32)| if answers { held.1 } else { held.0 };
        let seen: Vec<u32> = remembers(genome, case, exam).into_iter().map(watched).collect();
        let last = seen.last().copied().unwrap_or(0);
        match seen.iter().rposition(|&it| it != last) {
            Some(at) if at + 1 < seen.len() => {
                latest = latest.max(u32::try_from(at).unwrap_or(0) + 1);
            }
            Some(_) => never += 1,
            None => {}
        }
    }
    (latest, never)
}

/// On how many inputs every marked bit of the answer is right, and how far wrong it is when it is
/// not.
///
/// **A score of one a bit does not say this.** A chip right about two bits of three on every input
/// scores two thirds and is never once right, and one that is entirely right on two inputs in three
/// scores the same. Where the answer is a number, how far off it is is the thing worth knowing.
fn exactly(genome: &[u32], exam: &Exam) -> (u32, u32) {
    let mask = (1 << exam.outputs) - 1;
    let (mut whole, mut worst) = (0, 0);
    for case in 0..exam.cases() {
        let said = runs(genome, case, exam);
        let want = exam.want.get(case as usize).copied().unwrap_or(0) & mask;
        let ask = exam.ask.get(case as usize).copied().unwrap_or(0);
        if said & ask == want & ask {
            whole += 1;
        }
        worst = worst.max(said.abs_diff(want));
    }
    (whole, worst)
}

/// What the state rows hold after each step, for one input.
///
/// # Reading a machine off a grid
///
/// The state rows are the only thing that crosses from one step to the next, so whatever the chip
/// remembers is in them and nowhere else. Running an input and writing the rows down after every
/// step is the state machine, in the encoding the search happened to pick — which is nobody's idea
/// of a good one and is still the whole of it.
fn remembers(genome: &[u32], case: u32, exam: &Exam) -> Vec<(u32, u32)> {
    let fan = exam.bits / exam.steps;
    let mut last: Vec<u32> = vec![0; ROWS as usize];
    let mut seen = Vec::with_capacity(exam.steps as usize);
    for step in 0..exam.steps + exam.hold {
        let keep = |row: u32| last.get(row as usize).copied().unwrap_or(0);
        let mut column: Vec<u32> = (0..ROWS)
            .map(|row| {
                let (which, base, size) = banded(&exam.band, row);
                let inside = row - base;
                let fed = exam.feeds == 0 || (which as u32) < exam.feeds;
                if inside < fan && fed {
                    if step >= exam.steps && !exam.linger {
                        0
                    } else {
                        (case >> (step.min(exam.steps - 1) * fan + inside)) & 1
                    }
                } else if inside < fan + exam.state {
                    if step == 0 { 0 } else { keep(row) }
                } else if inside == size - 1 {
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
                    // The wrap is inside the band, so a boundary is as good as an edge.
                    let (_, base, size) = banded(&exam.band, row);
                    let round = |at: u32| base + at % size;
                    let (up, down, straight) = (
                        read(round(row - base + size - 1)),
                        read(round(row - base + 1)),
                        read(row),
                    );
                    match genome.get(((at - 1) * ROWS + row) as usize).copied() {
                        Some(1) => up.min(down),
                        Some(2) => up.max(down),
                        Some(3) => (up + down) & 1,
                        Some(4) => 1 - straight,
                        Some(5) => (up + down + straight) / 2,
                        Some(6) => {
                            if straight == 1 {
                                up
                            } else {
                                down
                            }
                        }
                        Some(7) => (up + down + straight) & 1,
                        Some(8) => read(row + ROWS - exam.band.first().copied().unwrap_or(ROWS)),
                        _ => straight,
                    }
                })
                .collect();
            column = next;
        }
        let held = (fan..fan + exam.state)
            .filter(|&row| column.get(row as usize).copied().unwrap_or(0) == 1)
            .fold(0_u32, |bits, row| bits | (1 << (row - fan)));
        // **And what the answer rows say at the same moment**, which is a different question: the
        // state can churn under an answer that has already arrived and stopped moving.
        let saying = (0..exam.outputs)
            .filter(|&at| column.get(exam.answer_row(at) as usize).copied().unwrap_or(0) == 1)
            .fold(0_u32, |bits, at| bits | (1 << at));
        seen.push((held, saying));
        last = column;
    }
    seen
}
/// Take a score apart: how much of it is being right, and how many gates were spent.
///
/// With `lean` off a score is the answer bits and nothing else. With it on the gates are packed
/// underneath, low enough that being right always outranks being small.
fn unpack(score: u32, budget: u32) -> (u32, u32) {
    if budget > 0 {
        (score / (budget + 1), budget - (score % (budget + 1)))
    } else {
        (score, 0)
    }
}

/// Where a row sits in its band, worked out on the card.
///
/// **Equal bands would be a shift.** Uneven ones are a chain of comparisons instead: each boundary
/// past the row's own leaves the answer alone, and the last one that is not past it wins.
fn where_in(
    kernel: &mut Kernel<U32>,
    row: Id,
    bands: &[u32],
) -> Result<(Id, Id, Id, Id), LaneError> {
    let uint = kernel.index_type();
    let first = bands.first().copied().unwrap_or(ROWS);
    let mut base = kernel.module().constant_u32(0)?;
    let mut size = kernel.module().constant_u32(first)?;
    let mut which = kernel.module().constant_u32(0)?;
    let mut edge = first;
    for (at, &held) in bands.iter().enumerate().skip(1) {
        let past = kernel.module().constant_u32(edge - 1)?;
        let hit = above(kernel, row, past)?;
        let there = kernel.module().constant_u32(edge)?;
        let wide = kernel.module().constant_u32(held)?;
        let index = kernel.module().constant_u32(u32::try_from(at).unwrap_or(0))?;
        base = pick(kernel, hit, there, base)?;
        size = pick(kernel, hit, wide, size)?;
        which = pick(kernel, hit, index, which)?;
        edge += held;
    }
    let inside = minus(kernel, row, base)?;
    let _ = uint;
    Ok((which, base, size, inside))
}

/// Which band a row is in, where that band starts and how many rows it has.
///
/// **Equal bands are a shift and unequal ones are not**, so this is worked out with comparisons
/// rather than by dividing, on the host and in the kernel alike.
fn banded(bands: &[u32], row: u32) -> (usize, u32, u32) {
    let mut base = 0;
    for (which, &size) in bands.iter().enumerate() {
        if row < base + size {
            return (which, base, size);
        }
        base += size;
    }
    (0, 0, bands.first().copied().unwrap_or(ROWS))
}

/// What a gate of this kind costs, which is how many wires it reads.
///
/// **A wire is not a gate**, so it is free, and the two that read three cost three.
fn price(kind: u32) -> u32 {
    u32::try_from(reads(kind, 0, &[ROWS]).len()).unwrap_or(0) * u32::from(kind != 0)
}

/// Which wires of the column before a gate of this kind reads.
///
/// **Two of the seven read three.** `maj` and `mux` take the wire straight ahead as well as the two
/// diagonals, so a cone traced back through one of them is a row wider than through the others.
fn reads(kind: u32, row: u32, band: &[u32]) -> Vec<u32> {
    let (_, base, size) = banded(band, row);
    let round = |at: u32| base + at % size;
    let (up, down) = (round(row - base + size - 1), round(row - base + 1));
    match kind {
        0 | 4 => vec![row],
        5..=7 => vec![up, row, down],
        8 => vec![row + ROWS - band.first().copied().unwrap_or(ROWS)],
        _ => vec![up, down],
    }
}

/// The cells one answer alone depends on, traced back from the row it is read off.
///
/// **As [`live`], but for a single output.** Two answers that share most of their cells are two
/// answers computed by the same circuit, and a change that helps one moves the other whether or not
/// that was wanted. Counting the sharing is what says whether that is happening.
fn cone(genome: &[u32], exam: &Exam, output: u32) -> Vec<bool> {
    let mut wanted = vec![false; (COLUMNS * ROWS) as usize];
    let at = |column: u32, row: u32| (column * ROWS + row % ROWS) as usize;
    if let Some(slot) = wanted.get_mut(at(COLUMNS - 1, exam.answer_row(output))) {
        *slot = true;
    }
    let mut used = vec![false; CELLS as usize];
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
            let reaches = reads(kind, row, &exam.band);
            for back in reaches {
                if let Some(slot) = wanted.get_mut(at(column - 1, back)) {
                    *slot = true;
                }
            }
        }
    }
    used
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
    // **Every row, not the first few.** This counted up to `outputs` or the state rows, whichever
    // was further, and an answer read off a row past that was left out of its own cone: at
    // `apart 3` it missed the third answer and at `apart 8` it missed three of four.
    for row in 0..ROWS {
        let (which, base, size) = banded(&exam.band, row);
        let reads_it = (0..exam.outputs).any(|at| exam.answer_row(at) == row)
            || (row >= base + fan && row < base + fan + exam.state);
        let _ = (which, size);
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
            let reaches = reads(kind, row, &exam.band);
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
/// How many marked bits each answer gets right on its own.
fn each(genome: &[u32], exam: &Exam) -> Vec<u32> {
    let mut held = vec![0_u32; exam.outputs as usize];
    for case in 0..exam.cases() {
        let said = runs(genome, case, exam);
        let want = exam.want.get(case as usize).copied().unwrap_or(0);
        let ask = exam.ask.get(case as usize).copied().unwrap_or(0);
        for at in 0..exam.outputs {
            let right = (ask >> at) & 1 == 1 && (said >> at) & 1 == (want >> at) & 1;
            if let (true, Some(slot)) = (right, held.get_mut(at as usize)) {
                *slot += 1;
            }
        }
    }
    held
}

/// What every single-gate change would do to each answer separately.
///
/// # Where the missing bits went
///
/// The three answers of `count` score forty-six and a half asked one at a time and forty-two and a
/// half asked together, and the four bits between those numbers had to go somewhere. Sharing cells
/// is not where: the answers share ninety per cent of their gates and the ones that share least
/// score worst. This counts the other candidate — a change that raises one answer and lowers
/// another — by trying every gate in every allowed kind on the chip the search finished with, and
/// scoring each answer on its own before and after.
fn pulls(genome: &[u32], exam: &Exam) -> (u32, u32, u32, u32, usize, u32, u32) {
    let was = each(genome, exam);
    let whole = scored(genome, exam);
    let (mut trades, mut gains, mut losses, mut flat) = (0, 0, 0, 0);
    let (mut lifts, mut refused) = (0, 0);
    let weakest = (0..exam.outputs as usize)
        .min_by_key(|&at| was.get(at).copied().unwrap_or(0))
        .unwrap_or(0);
    let mut tried = genome.to_vec();
    for cell in 0..CELLS as usize {
        let held = genome.get(cell).copied().unwrap_or(0);
        for &kind in &exam.allowed {
            if kind == held {
                continue;
            }
            if let Some(slot) = tried.get_mut(cell) {
                *slot = kind;
            }
            let now = each(&tried, exam);
            let up = (0..exam.outputs as usize).any(|at| now.get(at) > was.get(at));
            let down = (0..exam.outputs as usize).any(|at| now.get(at) < was.get(at));
            match (up, down) {
                (true, true) => trades += 1,
                (true, false) => gains += 1,
                (false, true) => losses += 1,
                (false, false) => flat += 1,
            }
            if now.get(weakest) > was.get(weakest) {
                lifts += 1;
                // **The exam's own total, not the sum of the parts**, because `slack` scores a
                // whole answer at a time and the parts do not add up to it.
                if scored(&tried, exam) < whole {
                    refused += 1;
                }
            }
        }
        if let Some(slot) = tried.get_mut(cell) {
            *slot = held;
        }
    }
    (trades, gains, losses, flat, weakest, lifts, refused)
}

/// The chip as something to look at: one letter a gate, the inputs down the left.
///
/// **An arrow marks an answer wire.** Output nought is the top row of the last column, output one
/// the row under it, and the exam says how many there are.
fn drawn(genome: &[u32], bits: u32, exam: &Exam, alive: &[bool]) -> String {
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
                Some(5) => '#',
                Some(6) => '?',
                Some(7) => '+',
                Some(8) => '|',
                _ => '-',
            });
        }
        if let Some(at) = (0..exam.outputs).find(|&at| exam.answer_row(at) == row) {
            out.push_str(" <- ");
            out.push(char::from(b'0' + u8::try_from(at).unwrap_or(0)));
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
    let mut modules = vec![
        advance(width)?,
        select(width, colony, exam.island, &exam.allowed, &exam)?,
        ready(width, cases)?,
    ];
    for step in 0..exam.steps + exam.hold {
        modules.push(feed(width, cases, colony, step, &exam)?);
        for column in 1..COLUMNS {
            modules.push(propagate(
                width,
                column,
                cases,
                colony,
                exam.noise,
                &exam.allowed,
                &exam.band,
            )?);
        }
    }
    modules.push(tally(width, cases, colony, &exam)?);
    let held: Vec<&[u32]> = modules.iter().map(Vec::as_slice).collect();

    let wires = Groups::From { binding: COUNTS as usize, at: 1 };
    let chips = Groups::From { binding: COUNTS as usize, at: 4 };
    let mut stages: Vec<Vec<Groups>> = vec![
        vec![Groups::Counting(1)],
        vec![Groups::Fixed(colony / width)],
        vec![Groups::Counting(1)],
    ];
    for _ in 0..exam.steps + exam.hold {
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
        (WEIGHTS + exam.outputs) as usize,
        (colony * CELLS) as usize,
        cases as usize,
        ((colony + 1) * COLUMNS * ROWS * cases) as usize,
        (colony + 1) as usize,
        (colony + 1) as usize,
        (colony + 1) as usize,
        7,
        cases as usize,
        ((colony + 1) * exam.outputs) as usize,
        ((colony + 1) * exam.outputs * cases) as usize,
    ];
    let mut arena = host.resident(&sizes, &held)?;
    for (binding, words) in sizes.iter().enumerate() {
        arena.write(binding, &vec![0_u32; *words])?;
    }
    arena.write(TARGET as usize, &exam.want)?;
    // **The ladder.** With it, only the low output bits are asked about to begin with and another is
    // added when the colony has the ones it has. Without it, everything is asked at once, which is
    // what this was.
    let mut rung = if exam.ladder { 1 } else { exam.outputs };
    let masked = |rung: u32| -> Vec<u32> {
        let keep = if rung >= 32 { u32::MAX } else { (1 << rung) - 1 };
        exam.ask.iter().map(|it| it & keep).collect()
    };
    let asked_at = |rung: u32| -> u32 { masked(rung).iter().map(|it| it.count_ones()).sum() };
    arena.write(KEEP as usize, &masked(rung))?;
    let mut seed = exam.seed;
    // **Drawn from what the exam allows**, or the restriction is only half a restriction: the grids
    // would start full of gates the mutation is never allowed to put back.
    let draw = |seed: &mut u32| -> Vec<u32> {
        (0..colony * CELLS)
            .map(|_| {
                let at = stirred(seed) as usize % exam.allowed.len().max(1);
                exam.allowed.get(at).copied().unwrap_or(0)
            })
            .collect()
    };
    // **Stacked, the colony starts where an earlier exam finished.** The kept grid goes in `KEPT`
    // times, spread out, and the rest of the colony is drawn as usual — filling every chip with it
    // leaves a colony with no spread in it at all, and that is measurably worse than not stacking.
    let genes = match exam.start.as_deref() {
        Some(path) => {
            let held: Vec<u32> = std::fs::read_to_string(path)?
                .split_ascii_whitespace()
                .filter_map(|it| it.parse().ok())
                .collect();
            if held.len() != CELLS as usize {
                return Err(format!(
                    "`start {path}` holds {} cells and this grid has {CELLS}",
                    held.len()
                )
                .into());
            }
            let many = if exam.aliens == 0 { KEPT } else { exam.aliens.min(colony) };
            println!("  starting from {path}, {} cells in {many} chips", held.len());
            let mut fresh = draw(&mut seed);
            let span = CELLS as usize;
            for slot in (0..colony as usize).step_by((colony / many).max(1) as usize) {
                if let Some(now) = fresh.get_mut(slot * span..(slot + 1) * span) {
                    now.copy_from_slice(&held);
                }
            }
            fresh
        }
        None => draw(&mut seed),
    };
    arena.write(GENE as usize, &genes)?;
    let seeds: Vec<u32> = (0..=colony).map(|_| stirred(&mut seed)).collect();
    arena.write(SEED as usize, &seeds)?;

    // One each to begin with, so `dynamic` starts as the plain total and moves off it.
    arena.write_at(PARAMS as usize, WEIGHTS as usize, &vec![1_u32; exam.outputs as usize])?;
    let batches = GENERATIONS / BATCH;
    let mut spent = Duration::ZERO;
    println!("  {:>10}  {:>5}  {:>24}  {:>10}", "generation", "best", "the colony", "us a tick");
    let mut best;
    let mut high = 0;
    let mut stuck = 0;
    // **The ladder and the redraw both wait for a stall and they must not share the wait.** With
    // one counter the shorter of the two keeps resetting it and the longer never fires.
    let mut waited = 0;
    let mut drawn_again = 0;
    let mut ran = 0;
    for batch in 0..batches {
        ran = batch + 1;
        spent += arena.staged_frames(&groups, BATCH)?;
        let held = arena.read(SCORE as usize, colony as usize)?;
        // **What the card selects on is not always a count of right answers.** `weakest` packs the
        // lowest answer above the total and `dynamic` multiplies each by a weight, so the number to
        // report on is added up again here from the answers written down apart.
        let split = if exam.weigh == 0 {
            Vec::new()
        } else {
            arena.read(SPLIT as usize, (colony * exam.outputs) as usize)?
        };
        let scores: Vec<u32> = if exam.weigh == 0 {
            held.iter().map(|&it| unpack(it, exam.budget()).0).collect()
        } else {
            split.chunks(exam.outputs as usize).map(|it| it.iter().sum()).collect()
        };
        // **The weight is how far behind the colony is.** An answer nobody has yet is worth the most
        // and one everybody has is worth one, so the score keeps moving towards what is missing.
        if exam.weigh == 2 {
            let weights: Vec<u32> = (0..exam.outputs as usize)
                .map(|at| {
                    let far = split
                        .iter()
                        .skip(at)
                        .step_by(exam.outputs as usize)
                        .copied()
                        .max()
                        .unwrap_or(0);
                    cases + 1 - far.min(cases)
                })
                .collect();
            arena.write_at(PARAMS as usize, WEIGHTS as usize, &weights)?;
        }
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
        // **Or when it has stopped moving.** Waiting for mastery leaves the run on a rung that is
        // itself a wall — rung two of `count` never came, so the whole run was spent on two thirds of
        // the exam and then judged on all of it.
        if best > high {
            high = best;
            stuck = 0;
            waited = 0;
        } else {
            stuck += 1;
            waited += 1;
        }
        if (best == asked_at(rung) || stuck >= PATIENCE) && rung < exam.outputs {
            // **Everyone is re-scored when the rung moves.** A chip that did not change is carrying a
            // score measured against the easier exam, and comparing that against a score measured
            // against the harder one hands the rung to whoever was asked less. Setting the tick back
            // to nought puts the whole colony on the changed list next tick.
            rung += 1;
            high = 0;
            stuck = 0;
            arena.write(KEEP as usize, &masked(rung))?;
            arena.write_at(PARAMS as usize, 0, &[0])?;
            println!("  {:>10}  {:>5}  rung {rung} of {}: {} bits asked", (batch + 1) * BATCH, best, exam.outputs, asked_at(rung));
        }
        // **A run is one draw, so draw again.** Everything but the best chip is replaced with fresh
        // grids, which costs one write of the genome and keeps whatever the run has found.
        if exam.renew > 0 && waited >= exam.renew && best < perfect {
            let genome = arena.read(GENE as usize, (colony * CELLS) as usize)?;
            let at = scores.iter().enumerate().max_by_key(|&(_, it)| *it).map_or(0, |(at, _)| at);
            let mut fresh = draw(&mut seed);
            let span = CELLS as usize;
            // **With `apiece` the best bands are in different chips.** Each is kept from whichever
            // chip held it, so what a redraw carries over is a grid nothing in the colony was.
            let mut best = genome
                .get(at * span..(at + 1) * span)
                .map(<[u32]>::to_vec)
                .unwrap_or_else(|| vec![0; span]);
            let apiece = arena.read(SPLIT as usize, (colony * exam.outputs) as usize)?;
            let mut theirs = vec![0_u32; exam.outputs as usize];
            if exam.apiece {
                for which in 0..exam.outputs as usize {
                    let from = (0..colony as usize)
                        .max_by_key(|&chip| {
                            apiece.get(chip * exam.outputs as usize + which).copied().unwrap_or(0)
                        })
                        .unwrap_or(0);
                    if let Some(slot) = theirs.get_mut(which) {
                        *slot = apiece
                            .get(from * exam.outputs as usize + which)
                            .copied()
                            .unwrap_or(0);
                    }
                    for cell in 0..span {
                        if banded(&exam.band, cell as u32 % ROWS).0 != which {
                            continue;
                        }
                        if let (Some(now), Some(was)) =
                            (best.get_mut(cell), genome.get(from * span + cell))
                        {
                            *now = *was;
                        }
                    }
                }
            }
            // **One copy of the best does not survive a generation**, because every chip is a
            // mutant of itself or its neighbour and the best is no exception. Spreading `KEPT`
            // copies across the colony is what makes the redraw keep anything at all.
            let mut blank = vec![0_u32; (colony + 1) as usize];
            let mut apart = vec![0_u32; ((colony + 1) * exam.outputs) as usize];
            let step = (colony / KEPT).max(1) as usize;
            for slot in (0..colony as usize).step_by(step) {
                if let Some(now) = fresh.get_mut(slot * span..(slot + 1) * span) {
                    now.copy_from_slice(&best);
                }
                if let (Some(held), Some(now)) = (held.get(at), blank.get_mut(slot)) {
                    *now = if exam.apiece { theirs.iter().sum() } else { *held };
                }
                for (which, &was) in theirs.iter().enumerate() {
                    if let Some(now) = apart.get_mut(slot * exam.outputs as usize + which) {
                        *now = was;
                    }
                }
            }
            arena.write(GENE as usize, &fresh)?;
            // **The scores have to go with the genomes.** `select` runs at the top of a frame and
            // reads the scores the last one left, so a fresh grid carrying the number its
            // predecessor earned would be copied over the one chip that was worth keeping.
            arena.write(SCORE as usize, &blank)?;
            if exam.apiece {
                arena.write(SPLIT as usize, &apart)?;
            }
            arena.write_at(PARAMS as usize, 0, &[0])?;
            waited = 0;
            drawn_again += 1;
        }
        // With `lean` on, being right is where the work starts rather than where it stops.
        if best == perfect && rung == exam.outputs && exam.lean == 0 {
            break;
        }
    }

    let raw = arena.read(SCORE as usize, colony as usize)?;
    let genes = arena.read(GENE as usize, (colony * CELLS) as usize)?;
    // **`dynamic`'s last score was taken in the last batch's weights**, which are nobody's idea of
    // a ranking, so the winner is picked by working the exam out again on the host.
    let winner = if exam.weigh == 2 {
        (0..colony as usize)
            .max_by_key(|&chip| {
                let held = genes.get(chip * CELLS as usize..(chip + 1) * CELLS as usize);
                held.map_or(0, |it| scored(it, &exam))
            })
            .unwrap_or(0)
    } else {
        raw.iter().enumerate().max_by_key(|&(_, s)| *s).map_or(0, |(at, _)| at)
    };
    let genome = genes
        .get(winner * CELLS as usize..(winner + 1) * CELLS as usize)
        .unwrap_or(&[]);
    let alive = live(genome, &exam);
    println!("\n{}", drawn(genome, exam.bits, &exam, &alive));
    println!(
        "    - wire  & and  v or  ^ xor  ! not  # maj  ? mux  + sum  | over  · dead  (<- an answer)"
    );
    if exam.outputs > 1 {
        let apiece = each(genome, &exam);
        let (trades, gains, losses, flat, weakest, lifts, refused) = pulls(genome, &exam);
        println!("  what each answer is worth on its own: {apiece:?}");
        println!(
            "  of every single-gate change: {trades} pull two answers apart, {gains} help at \
             no cost, {losses} only hurt, {flat} change nothing"
        );
        println!(
            "  answer {weakest} is the short one: {lifts} changes raise it, {refused} of \
             those the total score refuses"
        );

        let cones: Vec<Vec<bool>> = (0..exam.outputs).map(|it| cone(genome, &exam, it)).collect();
        println!("  what each answer is built from:");
        for (which, held) in cones.iter().enumerate() {
            let own = held.iter().filter(|&&it| it).count();
            let shared = held
                .iter()
                .enumerate()
                .filter(|&(cell, &it)| {
                    it && cones.iter().enumerate().any(|(other, that)| {
                        other != which && that.get(cell).copied().unwrap_or(false)
                    })
                })
                .count();
            println!("    answer {which}: {own} gates, {shared} of them shared with another answer");
        }
        println!();
    }
    let (live_gates, survived, whole) = faults(genome, &exam);
    let across = alive
        .iter()
        .enumerate()
        .filter(|&(cell, &it)| it && genome.get(cell).copied() == Some(8))
        .count();
    let wide = alive
        .iter()
        .enumerate()
        .filter(|&(cell, &it)| it && price(genome.get(cell).copied().unwrap_or(0)) == price(5))
        .count();
    let wires: u32 = alive
        .iter()
        .enumerate()
        .filter(|&(_, &it)| it)
        .map(|(cell, _)| price(genome.get(cell).copied().unwrap_or(0)))
        .sum();
    println!(
        "\n  {live_gates} of {CELLS} gates carry the answer and read {wires} wires, {wide} of \
         them three at a time; the rest \
         are scenery."
    );
    if exam.band.len() > 1 {
        println!("  {across} of them cross a band, which is the only thing that does");
    }
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
                let seen = remembers(genome, case, &exam);
                let held: Vec<String> = seen.iter().map(|it| format!("{}", it.0)).collect();
                let says: Vec<String> = seen.iter().map(|it| format!("{}", it.1)).collect();
                println!("    {pattern}   state {}", held.join(" -> "));
                println!("           saying {}", says.join(" -> "));
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
    if let Some(path) = exam.keep.as_deref() {
        let held: Vec<String> = genome.iter().map(u32::to_string).collect();
        std::fs::write(path, held.join(" "))?;
        println!("  kept in {path}");
    }
    let again = scored(genome, &exam);
    let (whole, worst) = exactly(genome, &exam);
    // **Is the hold a perturbation or part of the exam?** The same genome, worked out on the host
    // under a hold it was never selected under, is what says which.
    if exam.hold > 0 {
        let elsewhere: Vec<String> = [0, 1, 2, 3, 4, exam.hold * 2]
            .into_iter()
            .filter(|it| *it != exam.hold)
            .map(|hold| {
                let asked = Exam { hold, ..exam.clone() };
                format!("{} at hold {hold}", scored(genome, &asked))
            })
            .collect();
        println!("  the same chip scores {}", elsewhere.join(", "));
    }
    if exam.state > 0 {
        let of = exam.steps + exam.hold;
        for (what, answers) in [("state", false), ("answer", true)] {
            let (latest, never) = settles(genome, &exam, answers);
            let but = if never == 0 {
                "on every input".to_string()
            } else {
                format!("but never on {never} inputs")
            };
            println!("  the {what} stops changing after step {latest} of {of}, {but}");
        }
    }
    // The card's own count, from the plain total under `sum` and from the answers written down
    // apart under the other two, because there the score is not a count of right answers.
    let card = if exam.weigh == 0 {
        unpack(raw.get(winner).copied().unwrap_or(0), exam.budget()).0
    } else {
        let split = arena.read(SPLIT as usize, (colony * exam.outputs) as usize)?;
        split
            .chunks(exam.outputs as usize)
            .nth(winner)
            .map_or(0, |it| it.iter().sum())
    };
    println!(
        "\n  worked out on the host it gets {again} of {perfect} — the card said {card}{}",
        if again == perfect { ", and that is every case the exam asked about" } else { "" }
    );
    println!(
        "  entirely right on {whole} of {} inputs, and never more than {worst} out",
        exam.cases()
    );
    println!(
        "  the colony was drawn again {drawn_again} times",
    );
    println!(
        "  {:.3} us a generation of host clock",
        spent.as_secs_f64() * 1e6 / f64::from(ran.max(1))
    );
    Ok(())
}
