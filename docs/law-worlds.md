# Worlds from laws

A world can run a law, the JSON language of `tests/law/*.law.json`, instead of a built-in rule.

## Make one

`grit_world_create` takes exactly one of `rule` (a name `grit_world_rules` lists) and `law` (the law's JSON text as a string). Size, `params`, `sparse` and `slots` work as for a rule; `params` holds exactly the law's `params` words.

```json
{"wide":32,"deep":32,"tall":32,"law":"{\"law\":1,\"name\":\"keep\",...}","params":[],"sparse":true}
```

The answer adds:

| Field | Meaning |
|---|---|
| `law` | sha256 of the law's canonical text (`Law::hex`) |
| `quiet` | `proved` or `whole` |
| `quiet_why` | for `whole`, the reason |
| `tallied` | which octs the tally sums: `listed octs` (proved) or `every oct` (whole); also in every step answer |

Refusals:

| Code | When | `field` |
|---|---|---|
| `rule_or_law` | both or neither of `rule` and `law` | `$` |
| `law_refused` | the law does not compile; `law` in the answer holds its own `code`, `path`, `expected`, `found` | `$.law` |
| `law_not_built` | the device's driver failed to compile a kernel made from a law (shader module or pipeline creation, which the message names): a create with `law`, and an open, fork, walk back or rebuild of a law world; `device` and `vulkan` in the answer name the device and its result. No world is made or changed; a kept or rebuilding world stays as it was and opens on a device that compiles it. A kernel made from no law (a rule world) is `kernel_not_built` at `$`, with the same promises | `$.law` |
| `params` | a params count other than the law's | `$.params` |
| `quiet_refused` | a sparse world under a `whole` law | `$.sparse` |

`grit_world_params` gives a world new params between frames. The world steps every oct on its next frame, and a law world's class is decided again: a sparse world whose law turns `whole` is refused with `quiet_refused` at `$.params` and keeps its params, cells and state hash. A dense world that changes class switches between listed and whole stepping in place, with no second copy of the world: the switch is one word of `PARAMS` its decay pass reads.

`grit_world_restore` gives a world the cells, frame, params and stepping of another world of the same law, size and layout, so a dense fork whose params changed its class restores with that class.

The law, its hash and its params come back after `grit_world_save` and `grit_world_open`, `grit_world_fork`, `grit_world_restore`, a restart of the server (the journal's `LAW` and `PARAMS` records) and a lost device. `grit_world_list` names each law world's `law` hash, and for an open world its `quiet`; `grit_describe` lists the open law worlds under `worlds.laws`.

## Looking at neighbours

`["look", plane, dx, dy, dz, corner]` is the word of `plane` at the corner's cell plus `(dx, dy, dz)`, wrapped on the torus. Each offset is -1, 0 or 1; `plane`, the offsets and `corner` are ints known when the law compiles. A look inside the oct is the corner's own word.

```json
["map", "c", ["range", 8], ["vec", ["at", ["at", "cells", "c"], 0], ["look", 0, -1, 0, 0, "c"]]]
```

Rules, each refused by its own code at the path of the look or of `halo`:

| Code | When |
|---|---|
| `halo_declared` | `halo` is not 1 for a law that looks, or not 0 for one that does not |
| `halo_radius` | an offset outside -1..1 |
| `halo_constant` | a plane, offset or corner known only when the law runs |
| `halo_plane` | a plane past `channels` |
| `halo_written` | a look at a plane the law writes: only a frozen plane, whose 8 out words are its input words unchanged, may be looked at |

A host write to a world whose law looks wakes the 27 octs around the oct of each phase that holds the cell, so every oct whose 4x4x4 box holds the cell steps next frame. The quiet class is decided as below, with the looked cells part of the oct's input: a law that reads neither `frame` nor `at` gives the same output for the same box, so it keeps the box contract and steps listed; any other law steps whole.

`tests/halo3/` holds three laws that look: a 2D Sobel edge detector, a 3D 3x3x3 Sobel and a blur over two planes.

### On sparse worlds

A law that looks runs on dense and sparse worlds alike, with the same cells after every frame. A sparse world reads an oct's 4x4x4 box through its brick table: a cell of a held brick is its word, a cell of a brick held as one word is that word, on each plane, across brick borders and the torus seam.

An oct that changes wakes the 27 octs around it, so every oct whose box reads a changed cell steps the next two frames. A woken oct in a one-word brick steps the whole brick; a brick whose cells change takes a slot first. A brick left all one word, and untouched for a frame, returns its slot. So memory follows activity: a glider flying through a large world holds only the few bricks around it, and each step answer names `pool_use`.

A frame visits only the bricks near activity, so its cost follows what changes, not the size of the world.

A host write to a sparse world wakes the 27 octs around each written cell's octs, as on dense. A frame is priced as on any sparse world: held bricks times their cells times frames, calibrated under the law's own text.

## Iterating a law on the device

A law that reads plane `a` and writes plane `b` gives one generation a frame. `alternate` `[a, b]` makes it iterate with no host round trip: frames of even parity (the world's frame counter, bit 0) run the law as written, looking at `a` and writing `b`; frames of odd parity run it with `a` and `b` swapped in its inputs and outputs. After `k` frames from frame 0 generation `k` is on the current plane: `a` at an even frame, `b` at an odd one.

`grit_world_create` takes `alternate` next to `law`; for a `grit_law_learn` answer it is `[the read's plane, the write's plane]`, and the law text is passed unchanged. A law may also carry the key itself. The world's law is then the iterating law, with its own hash; the answer adds `alternate`. The alternation is part of the law's text and its parity is the frame's, so both come back after a journal replay, `grit_world_fork`, `grit_world_restore`, `grit_world_save` and `grit_world_open` (also at an odd frame), `grit_world_params` and a lost device.

1. `grit_law_learn` Life: `stencil` `3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `rule` `outer`.
2. `grit_world_create` with `wide` 64, `deep` 64, `tall` 2, its `law` and `alternate` `[0,1]`.
3. `grit_world_write` the soup with `current` true.
4. `grit_world_step` 64 frames: `current` 0.
5. `grit_world_read` with `current` true: generation 64.

`grit_world_read`, `grit_world_slice` and `grit_world_write` take `current` true for the current plane, and their answer names `plane`; `grit_world_step` answers `current`. A world made without `alternate` refuses `current` with `not_iterating` at `$.current`; `current` with a `plane` other than 0 is `plane_or_current` at `$.plane`.

Within a frame the looked-at plane is frozen, as `halo_written` requires. Refused as `law_refused` at `$.alternate`, with the law's own code:

| Code | When |
|---|---|
| `alternate_plane` | `a` equals `b`, or either is past `channels` |
| `alternate_written` | the law writes `a` |
| `alternate_unwritten` | the law leaves `b` unchanged |
| `alternate_other` | the law writes a plane besides `b` |
| `alternate_unread` | the law never reads `a` |
| `alternate_reversible` | the law is reversible; `grit_world_back` on an iterating world is `not_reversible` |
| `alternate_twice` | the law carries an `alternate` other than the argument |

`alternate` with `rule` is `rule_or_law` at `$.alternate`. A law iterates on dense and sparse worlds, whether it looks or not.

A law may carry two pairs, `"alternate": [[a, b], [c, d]]` (at most channels / 2), each swapped on odd frames as one pair is: every looked plane is frozen and every target written, no plane named twice (`alternate_plane`), and the law writes no other plane (`alternate_other`); refusals in a list name the pair, `$.alternate[i]`. `current` is the first pair's. A one-pair law keeps the form `[a, b]` and the same kernel words. `grit_world_create`'s `alternate` argument takes one pair; a law with two carries them in its text, as a `grit_law_learn` answer under `steps` or `hidden` does.

The quiet class is decided on the law as written, with every uniform value checked at both parities. A listed world steps an oct while it or an oct around it changed in the two frames before; an oct that changed its written plane wakes the octs whose box reads it, so moving patterns are never skipped, and a world that stops changing, a period-2 oscillator included, goes quiet. Its frames are priced as every frame: cells times frames, calibrated under the iterating law's own text.

## Reading only the set or changed cells

`grit_world_read` takes `form`, for any world:

| `form` | Lists | Box |
|---|---|---|
| `dense` (default) | every cell's word, as before | at most 4096 cells |
| `set` | `[x, y, z, word]` of each cell whose word is not 0 | any size |
| `changed` | `[x, y, z, word]` of each cell whose word differs from the mark `since` names; a cleared cell is listed with 0 | the mark's |

Cells come x fastest, then y, then z; `count` is how many. `plane` and `current` work as for a dense read. One call lists at most 16384 cells (`limits.set_cells`); past that it is refused `too_many` at `$.to`, with `limit` and `count`, the cells it would list. Nothing is cut short. A box that full reads cheaper with `form` `dense`, at most 4096 cells a call: a listed cell carries its place as well as its word, so dense reads cost less once about a quarter of a box is set. The refusal's `hint` says so.

`mark` true on a `set` or `changed` read records the box as the read sees it and answers its id as `mark`. A `changed` read compares the device's words now with those recorded words, so it is exact across steps, writes, params, a restore and a fork (a fork starts with its source's marks). Marks live with the open world in the server's memory, at most 16 (`limits.marks_held`), the oldest dropped first. They are not journaled: a world opened from a file, after a restart or rebuilt after a lost device has none, and a `changed` read of an earlier mark is refused `mark_unknown`; take a new mark. `mark` false is the same as leaving it out, on any form.

1. `grit_world_read` the whole current plane with `form` `set`, `current` true and `mark` true.
2. `grit_world_step` 1 frame.
3. `grit_world_read` the same box with `form` `changed`, `since` the mark and `mark` true: the cells generation 1 changed, and the next mark.

The read scans the box on the device in one pass and moves 12 bytes plus 8 a listed cell back, whatever the box. Like the dense read it is not priced.

Refusals:

| Code | When | `field` |
|---|---|---|
| `read_form` | `since` without `form` `changed`, `changed` without `since`, or `mark` true with `form` `dense` | `$.since` or `$.mark` |
| `mark_unknown` | a mark this world does not hold: never taken, dropped past 16, or lost with a save, an open, a restart or a lost device | `$.since` |
| `mark_unlike` | a box or plane other than the mark's | `$.from` or `$.to`; for the plane, `$.current` when the read gives `current` true, else `$.plane` |
| `too_many` | more than 16384 cells to list, or a mark of more than 16384 set cells | `$.to` or `$.mark` |

## Counting the bits of a box

`grit_world_read` with `form` `tally` answers, for each bit `bits` names, how many cells of the box have that bit set and the least and greatest corner of those cells. A world that holds one pattern per bit, as 32 soups in one world do, learns which patterns died, which grew and where each one lives, without listing a cell.

| Field | Meaning |
|---|---|
| `bits` (argument) | 1 to 32 bit numbers, each from 0 to 31 and named once; all 32 if left out |
| `bits` (answer) | one record a named bit, in ascending order: `bit`, `count`, and, when `count` is above 0, `min` and `max`, each `[x, y, z]` in the world |

`from`, `to`, `plane` and `current` work as for every read, the box may be any size, and it works on every world, rule or law, dense or sparse; a cell of a sparse brick held as one word counts as that word. The answer adds `form` and, with `current`, `plane`.

```json
{"world":0,"from":[0,0,0],"to":[31,31,31],"current":true,"form":"tally","bits":[0,31]}
```

The device counts the box in one pass and moves 960 bytes back, whatever the box, plane or bits (`limits.tally_read`). Like the other reads it is not priced.

Refusals:

| Code | When | `field` |
|---|---|---|
| `bit_past` | a bit above 31 | `$.bits[k]` |
| `malformed` | a bit named twice (at its second place), or an empty list | `$.bits[k]` or `$.bits` |
| `too_many` | more than 32 bits | `$.bits` |
| `read_form` | `bits` with another form, or `since` or `mark` true with `tally` | `$.bits`, `$.since` or `$.mark` |

## Absorbing borders

`grit_world_create` takes `absorb`, `[x, y, z]`: after every frame, on the device, every plane of every cell within that many cells of the world's edge on that axis is zeroed. An axis given 0 wraps as before; a 2D world gives `[n, n, 0]`. Rule and law worlds, dense and sparse, `alternate` and `parallel` all take it, and the answer names it. All 0, or left out, is a world without a border.

A world with `absorb` holds, after every frame, the cells of the same world without it whose border is zeroed by host writes after every frame. A cell the border zeroes wakes the octs around it, as a write does. What flies into the border never comes back across the seam. A Life glider stops at the border as a 2 by 2 block, and a Bays 5766 glider as a 2 by 2 by 2 cube: the zeroed cells are a dead wall, and that is what such ships leave on one. On a sparse world the bricks a ship crossed are released by the quiet path; only the bricks of what stays are held.

A frame of a dense world with `absorb` is priced as its cells plus the border's cells; one of a sparse world as twice its held bricks' cells. The border is journaled, saved (section `ABSB` of the world file), forked and opened with the world. `grit_world_restore` refuses `unlike` between worlds with different borders, and `grit_world_back` refuses `not_reversible`: a border loses what it zeroes.

| Code | When | `field` |
|---|---|---|
| `absorb_wide` | twice an axis's border reaches its side, so no inside is left; `limit` is the widest that fits | `$.absorb[k]` |

## Clearing bits of a box

`grit_world_write` with `form` `clear` zeroes, on the device, the bits `bits` names (all 32 if left out) of every cell from `from` to `to`, corners included, on the planes `planes` names (every plane if left out; with `current` true the current plane). A box whose `from` is past its `to` on an axis wraps across the seam there. It wakes what changed, as a write does, and the answer names `cleared`, the words it changed.

```json
{"world":0,"form":"clear","from":[0,0,0],"to":[255,255,255],"bits":[7]}
```

Copy 7 of a 32-copy `parallel` world is then empty; the other 31 step on as if nothing happened. On a sparse world a cleared brick that is left one word is released by the quiet path. A brick held as one word that the box covers takes its new word; one it covers in part takes a slot. The device work follows the box: one lane per oct of the box on a dense world, one per brick on a sparse one, in passes of at most 16384. A clear is priced by its box's cells and journaled; replay, fork, save, open and a restart give the same bits.

| Code | When | `field` |
|---|---|---|
| `outside` | a corner past the world, or a plane past its planes | `$.from`, `$.to` or `$.planes[k]` |
| `bit_past` | a bit above 31 | `$.bits[k]` |
| `malformed` | a bit or plane named twice | `$.bits[k]` or `$.planes[k]` |
| `write_form` | `cells` with form `clear`, or neither `cells` nor form `clear` (a write without `cells` was `malformed` before `cells` became optional); `from`, `to`, `planes` or `bits` with form `cells`; `from` or `to` left out; `planes` with `current` true; `plane` with form `clear` | the field named |

## Reversible laws

A law with `"reversible": true` must be a sequence of lifting steps over its words, 8 a plane: each step adds, subtracts or xors to one word, wrapping, a value made only of other words, params, `frame`, `at`, constants and looks at frozen planes. The compiler checks the form, derives the inverse (the steps undone in reverse order) as an ordinary law named with `_back` and its own hash, and refuses anything else with `not_lifting` at the offending step's path: a step that reads its own word, one node written to two words, a word made by another op (`b = f(a)`, `b | f`, `b * 3`).

```json
{"law":1,"name":"shear","params":1,"channels":1,"halo":0,"reversible":true,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},
"step":["let",[["a",["at","cells",0]],["b",["at","cells",1]],["b2",["add","b",["mul","a",["param",0]]]],["a2",["xor","a",["shr","b2",3]]]],
["vec",["vec","a2","b2",["at","cells",2],["at","cells",3],["at","cells",4],["at","cells",5],["at","cells",6],["at","cells",7]],["vec",0]]]}
```

`grit_world_back` walks a world of such a law back `frames` (1 to 65536) frames, never past frame 0: frame `f` is undone with the inverse at frame `f`'s phase and the world's params. It does not undo writes or params. With `home` true a copy of the walked-back world steps forward again; if it does not come back bit for bit the answer is `strayed`, with the first differing cell, and the world is left as it was.

1. `grit_world_create` with the law above, `params` `[3]`.
2. `grit_world_write` some cells, `grit_world_step` 100 frames.
3. `grit_world_back` with `frames` 100 and `home` true: `frame` 0, `home` true, and the cells you wrote.

Refusals: `not_reversible` at `$.world` for a world whose law is not reversible, or a rule; `past_start` at `$.frames` for more frames than the world's frame. Walked-back worlds save, open, fork, restore, replay their journal and rebuild after a lost device at their frame.

## Learning a law's params by gradient

`grit_law_gradient` answers, for a reversible law, how the objective after `frames` frames moves with each param. The objective is the sum over the `aim` box, on plane 0, of `|cell - target|`; `aim` is `from` and `to` (corners included) with `target` (the box's words, x fastest) or `world` (take them from that world's box). The start is a world (`world`: its cells on every plane at its frame, its params unless `params` is given) or an inline start (`law`, `size` with sides powers of two, `cells` plane by plane, `first`). `gradient[i]` is d objective / d `params[i]` times 2^64, a decimal string; `home` is true. It is the derivative through every `add`, `sub`, `mul`, `div`, shift and `sel` arm from the param to a cell; a compare, and so a `sel`'s choice, counts as constant. A law in which `not`, `and`, `or`, `xor`, `dot_s` or `dot_u` takes a value that moves with a param (also through a cell an earlier frame moved) and passes it on to a cell other than through a compare is refused `no_gradient`, with that node's path as `law.path`.

A law that looks (`halo` 1) has the same exact gradient. A look reads a frozen plane, so its value moves with no param; the walk back reads the same neighbour at the same frame, and the look's share of the derivative goes back to that cell. A law that looks at a plane it writes is refused when it compiles (`halo_written`, as `law_refused`), so no look's value is lost on the way back.

The answers equal the library's `Reversible::walk` and `Reversible::descend` exactly, on the CPU.

`grit_law_descend` takes the same and `steps`, the most trial points after the start. It walks the start, then one trial point per step, and keeps a trial only when its objective is lower than the kept point's. A trial is the secant step through the gradients of its last walks; after a trial that is not lower it retries once at most as far, then at most half as far, then moves one param by 1 against its gradient. So its answer is never worse than its start and its kept objectives fall strictly. It finds a local optimum, not the global one.

It answers the last kept `params` and their `objective` (also the earliest lowest on its `trail`), `steps`, `trail` (every point walked, with `kept`) and `stopped`: `converged` (objective 0), `local` (no trial it can make is lower), `steps`, `budget` or `cancelled` (the best so far).

1. `grit_world_create` with `law` mix (`tests/law/mix.law.json`) and the hidden `params` `[3000,1700]`, `grit_world_write` a calm start (every cell between 2^21 and 2^23), `grit_world_step` 8 frames: the target world.
2. `grit_world_create` the same law with a guess, `params` `[2100,2250]`, and `grit_world_write` the same start: the learner.
3. `grit_law_descend` with `world` the learner, `frames` 8, `aim` `{"from":[0,0,0],"to":[15,15,15],"world":<target>}`, `steps` 16: `params` `[3000,1700]`, `objective` 0, `stopped` `converged`, after 3 steps.
4. `grit_world_params` the learner with them and `grit_world_step` 8 frames: its cells equal the target's.

The gradient only sees the optimum where the objective is smooth around it. A `mul` that wraps makes it ripple: with mix, 256 cells spread over all 32 bits and the same guess, the objective rises only within 12 of `[3000,1700]` along each param, and the descent stops `local` at `[232,0]`, objective 553884330929 against 588373928375 at its start. On such starts `grit_learn_params` often ends lower for the same number of runs; on calm starts the descent does.

A law that iterates (`alternate`) is never reversible, so it is walked by keeping every frame's cells: the frames are run forward and kept, and the adjoint is carried back through each kept frame with the same derivative rules, the planes of each pair swapped on odd frames as the law runs them. Such a walk always comes home; its cost counts the law's nodes twice in place of the law and its inverse. Other laws that are not reversible are refused as before.

Neither tool writes a world or its journal. Refused before any walk: `not_reversible` (with the lifting refusal as `law`), `no_gradient` (above), `params`, `outside`, `reversed`, `target_size`, `aim`, `start`, `walk_limit` (a start past `walk_cells` words, a walk past `walk_work`), `budget`. A walk costs octs times frames times (the nodes of the law and its inverse plus 128, plus 192 for each plane the law looks at). A walk that does not come home answers `strayed` with the cell; one past 128 bits answers `overflow`. `grit_describe` names the limits under `law_gradient`.

## Searching a law's params

`grit_law_score` and `grit_learn_params` work for any law: not reversible, with `not`, `and`, `xor`, `dot_s`, thresholds or looks. They take the start, `params`, `frames` (1 to 4096) and `aim` as `grit_law_gradient` does, and run the law forward on the CPU with `sand3::reference`, the CPU twin the device kernels are tested equal to, so the answer is the same on every device. The objective is the same sum of `|cell - target|`.

`grit_law_score` answers `objective` for `params`.

`grit_learn_params` also takes `bounds` (`[least, greatest]` per param), `seed`, `population` (1 to 256) and `generations` (1 to 1024). It is a (1+`population`) evolution strategy over whole params: each generation runs `population` children of the parent, each param moved by about its spread and clamped to its bounds; the earliest lowest child becomes the parent when it is no worse. A spread starts at a quarter of its bounds' width, grows when more than a fifth of the children were no worse, and shrinks when fewer were. After 8 generations without a lower parent, with every spread at 1, the parent restarts at a random point inside bounds. All draws come from `seed`.

It answers `params` and `objective` (the earliest lowest of every run), `stopped` (`found` at objective 0, `generations`, `budget` or `cancelled`), `generations`, `scored` (runs made) and `trail` (each finished generation's earliest lowest `{params, objective}`; generation 0 is `params`). The same arguments give the same answer byte for byte; a stopped search's `trail` is a prefix of the whole search's.

1. `grit_world_create` with `law` heat (`tests/law/heat.law.json`), 8 by 8 by 8, the hidden `params` `[300,7,900]`, `grit_world_write` the start, `grit_world_step` 8 frames: the target world.
2. `grit_world_create` the same law with a guess, `params` `[700,12,3000]`, and `grit_world_write` the same start: the learner.
3. `grit_learn_params` with `world` the learner, `frames` 8, `aim` `{"from":[0,0,0],"to":[7,7,7],"world":<target>}`, `bounds` `[[0,1023],[0,15],[0,4095]]`, `seed` 7, `population` 8, `generations` 200: `objective` 0, `stopped` `found`, after 533 runs.
4. `grit_world_params` the learner with the answer's `params` and `grit_world_step` 8 frames: its cells equal the target's.

Many params can give objective 0 (heat's `chance` only matters through a 4-bit draw), so the answer need not equal the hidden params. Heat has local optima: from this guess, 10 of seeds 0 to 19 reach 0 within 200 generations at population 8, and 15 at population 16; search again with another seed or a larger population when one stops on `generations`.

A run costs octs times frames times (law nodes + 128), at most `run_work` (2^26), on a start of at most `run_cells` (32768) words; it cannot stop part way. Refused before any run: `law_refused`, `start` (also a world under a named rule), `params` (also at `$.bounds` for the wrong count), `bounds` (a param outside its bounds, or no param free to move), `reversed`, `outside`, `target_size`, `aim`, `run_limit`, `budget`, and `too_many` for `population` or `generations` past their limit. A device lost during a search does not change its answer, which says `restored`. Neither tool writes a world or its journal. `grit_describe` names the limits under `law_search`.

## Checking a law on many starts

`grit_law_check` runs a law on N starts for `frames` frames on the device and scores each start against its targets, in one call. It replaces create, write, step, read and close for every start.

| Field | Meaning |
|---|---|
| `law` or `rule`, `alternate`, `params` | as `grit_world_create` takes them; `params` none if left out |
| `size` | every start's `[wide, deep, tall]`, a shape `grit_world_create` answers |
| `sparse`, `absorb` | as `grit_world_create` takes them, for each start's own world |
| `border` | `wrapped` (default): a start is a torus of its own size. `zero`: every cell outside the start's box reads 0 at every frame |
| `frames` | k, 1 to 4096 |
| `reads`, `writes` | the plane a start is written to, and the plane scored and answered; 0 if left out |
| `current` | true: both are the current plane, at frame 0 and at each target's frame. The default for a law that iterates |
| `values`, `frac` | `binary` (default) or `real`; `frac`, 16 if left out, the fractional bits of a `real` word |
| `starts` | each one of `cells` (`[x, y, z, word]`, every other cell 0), `dense` (every word of the box, x fastest) or `world` and `from` (a box of an open world, on its `plane`, its current plane if it iterates, else 0), with `targets` |
| `targets` | each a box in the same forms, at `frame`, 0 to k; k if left out |
| `return`, `page` | true: also answer the end states, a page at a time; `page` only with `return` true |

Each start equals, bit for bit, a world of its own: `grit_world_create` with the same law, params, size, `sparse`, `alternate` and `absorb`, the start written, stepped k frames and read. Under `border` `zero` that world is twice the start's size on every axis above 2, absorbs half the start's side there (plus `absorb`), and holds the start in its middle; an axis of side 2, the z of a 2D world, wraps.

The answer names `law` (its hash) or `rule`, the arguments it ran with, `run` and, per start, its `fingerprint` and one score per target, in the order given:

| Field | Meaning |
|---|---|
| `frame` | the target's frame |
| `cells`, `right`, `wrong` | the box's cells, and those equal and not equal to the target |
| `first_wrong` | `at`, `want`, `got` of the first wrong cell, x fastest, then y, then z; left out when none is wrong |
| `max`, `sum_squares`, `rms` | under `real`: the largest and the summed squared difference of the signed words, and its root mean square, in words |
| `max_real`, `rms_real` | under `real`: the same decoded, over 2^`frac` |

`totals` sums them over every target; its `max` is the largest. `fingerprint` is the sha256 of the start's end state: the box's words on every plane, plane 0 first, x fastest, each word 4 bytes little-endian, so two runs compare without reading them. `return` answers `[start, x, y, z, word]` of each end-state cell not 0 on the scored plane, 16384 a page, with `page`, `pages` and `count`; each page runs the check again.

Starts share one dense world when the border wraps, no `absorb` is given, the world is dense and the law reads no `at` (and so no `noise`). Each start is then a tile along one axis with a margin of its own torus copies, (1 + `halo`) times k cells wide on each side, rounded up to even: a frame moves what a cell reads by at most 1 + `halo` cells, so in k frames no tile reaches another's start, and the oct grid and the frame's parity are the start's own. A shared world holds at most 2^22 cells and at most 8 times the starts' cells. Every other start runs in a world of its own. `run` names `layout` (`tiled` or `apart`), `worlds`, and for `tiled` its `axis`, `margin` and `starts_per_world`.

A check is priced as the worlds it builds (their bytes), the cells it writes (margins included) and the frames it steps (world cells times k, held bricks on sparse worlds), in the units of `grit_world_create`, `grit_world_write` and `grit_world_step`. Its reads are not priced, as no read is. It stops on budget or cancel only before a build or a batch of frames, and then answers nothing. It writes no journal and leaves no world open.

X1's held check of p1, four 64 by 64 starts 8 frames on, is one call; so are its 12 training pairs, each a start with a target at frame 1.

### Host memory

A check holds every buffer it needs on the host, and the sum is refused past 2^30 bytes (`HOST_MOST`) before any device work. With a box of B cells, N starts, P planes and 4-byte words, it counts:

| Buffer | Bytes |
|---|---|
| starts | 4B each, whatever their form |
| a `dense` or `world` target | 4B |
| a `cells` target | 12 a listed cell: it is held as its listed cells (index and word, 8 bytes), not as a box, plus a sort's scratch |
| end states, with `return` | 4BN |
| reading boxes of worlds | 512 a cell of one piece of 2^15 cells: a box is read a piece at a time |
| running a world | its cells times 4P for the read back, 8B for one box and its fingerprint, and 48 a written cell for the cells the world is given (its whole size when tiled, B when apart) |
| the worlds it builds | 2^28, plus 16 a cell and plane of the largest world, plus 8 a cell and plane of every other world: what the engine and the driver hold for a world, measured on the RTX 4080 with headroom, and what they keep of a world after it is dropped |
| answering | 1024 a start and a target, and with `return` one page of 16384 cells |

The sum is the starts, the targets, the end states and the worlds it builds, plus the largest of reading, running and answering, as they are held one after another. Under `border` `zero` a world is 8 times the box of a 3D start, and counted so. The refusal names `$.starts[k].targets` of the start with the most target bytes when targets are the largest part, else `$.starts`. `cells` targets are held sparse because a box of 2^22 cells is 16 MiB a target from a request of a few bytes; a `dense` target already costs its box in the request.

Refusals, before any device work except `outside` at `$.page`:

| Code | When | `field` |
|---|---|---|
| `too_many` | more than 256 starts, more than 2^22 start cells (`limit` the most starts of this size), more than 1024 targets, more host memory than 2^30 bytes (`limit` the bytes) | `$.starts`, `$.starts[k].targets` |
| `frames` | k past 4096, or start cells times k past 2^32 (`limit` the most frames) | `$.frames` |
| `check_start` | a `size` no world has (`limit` the shape it would be), no start, a start with other than one form, a cell outside the box, a `dense` of the wrong length (`limit` the box's words) | `$.size`, `$.starts`, `$.starts[k]...` |
| `check_target` | a target with other than one form, a cell outside the box, a `dense` of the wrong length, a `frame` past k (`limit` k) | `$.starts[k].targets[j]...` |
| `outside` | `reads` or `writes` past the law's planes (`limit` the last), a `world` box past that world, `page` past the pages | the field |
| `not_iterating`, `plane_or_current` | `current` without `alternate`; `current` with `reads` or `writes` | `$.current`, `$.reads`, `$.writes` |
| `params`, `quiet_refused`, `absorb_wide`, `rule_or_law`, `law_refused` | as `grit_world_create` | as there |
| `malformed` | `frac` without `values` `real`; `page` without `return` true | `$.frac`, `$.page` |

Nothing is cut short: a request past a limit runs nothing.

## The quiet class

A listed world steps an oct only while it, or an oct around it, changed in the two frames before; a sparse world also keeps an all-one-value brick as a single word and steps it only while it is hot. That gives the cells of stepping every oct only when:

1. **the box contract**: an oct that a step leaves unchanged stays unchanged when stepped again with the same eight words. A law whose cell outputs read neither `frame` nor `at` (and so not `noise`, which reads both) is a function of the oct's words and the params alone, so it keeps it.
2. **fixed uniform values**: an oct whose eight words all hold a value a brick may be kept at steps to itself. Otherwise every such brick needs a slot when it is next stepped, and a pool smaller than the world overflows: the world stops and its reads answer `pool_full`.

| Class | Decided | Meaning | Stepping |
|---|---|---|---|
| `proved` | no cell output depends on `frame` or `at` in the law's node graph, and the background (0 on every plane) and every uniform brick value the world holds step to themselves under its params, by evaluating the law | both conditions hold | listed, dense or sparse |
| `whole` | anything else | | dense only, every oct every frame |

The class is decided at create from the background, and at every `grit_world_params` from the background and every uniform brick value in the sparse world's table. Opening a file decides it as a create does; a fork, a restore, a kept world and a rebuild keep the class last decided. A uniform brick value that is not a fixed point of a `proved` law can only be in a world's table when the world steps every brick on its next frame (after a create, an open or a params write), so such a brick takes a slot then and is never kept cold. The check reads the graph, not the text: a `let` the result never uses does not count, and an `["and","frame",0]` the result does use does.

## The tally

The class promises cells, not tallies. A frame's tally is the sum of the law's counts over the octs the frame steps. A `whole` world steps every oct of the frame's phase. A `proved` world steps only the octs it lists, and an unlisted oct adds nothing, so its tally equals the sum over every oct only for counts that are 0 on an oct the step leaves unchanged (a count of changed cells, say). A law that counts 1 for every oct tallies the whole phase on the frames after a seed and 0 once the world is quiet; stepped whole it tallies the whole phase every frame. `grit_describe` says this under `limits.law_tally`.

Laws in `tests/law/`: `code` is `proved`; `sand` and `embed` read `frame`, and `heat`, `nature` and `evo` read `noise`, so they are `whole` and run dense.

## From a learned chip to a world

`grit_law_from_chip` turns a chip that `grit_machine` or `grit_synthesise` found into a law. It takes the chip as those tools do (`inputs`, `outputs`, `state`, and `gates` as their answers list them) and a binding for each of its bits:

| Field | Each item | Meaning |
|---|---|---|
| `bind_inputs` | `{plane, dx, dy, dz, bit}` | chip input `j` is `bit` of `plane` at the cell plus `(dx, dy, dz)`, each -1, 0 or 1, wrapped on the torus |
| `bind_outputs` | `{plane, bit}` | chip output `j` is written to `bit` of `plane` at the cell |
| `bind_state` | `{plane, bit}` | state bit `j` is read from and written back to `bit` of `plane` at the cell |
| `channels` | | the planes of the world |
| `rest` | | `kept` or `cleared`: the other bits of a word an output or state bit is on; planes no binding writes are left as they are |

Chip source `i` is state bit `i` for `i` below `state`, then input `i - state`, then the gates, as in `grit_machine`. State bits start from what the world holds, 0 in a new world, as `grit_machine` starts from 0.

A gate costs up to 24 nodes (3 in each of the 8 cells of an oct) and the bindings up to 1600, so a chip of at most 600 gates (`limits.law_from_chip_gates`) always fits a law's 16384 nodes; copies and constants cost less, and the answer's `nodes` says how much is used.

A look wraps at the world's size as `grit_world_create` answers it: each side is rounded up to a power of two, so a world asked for at 8 by 6 by 10 is 8 by 8 by 16 and wraps there. The answer holds the law's text (`law`, for `grit_world_create`), its `hash`, `halo` (1 exactly when an input leaves the cell), `quiet` and `stepping` as a dense world decides them, and `nodes`. Each gate becomes at most two word operations per cell.

Refusals, each at the path of the binding or gate:

| Code | When |
|---|---|
| `unbound` | fewer bindings than chip inputs, outputs or state bits |
| `overbound` | more bindings than the chip has |
| `bit_past` | a bit past 31 |
| `plane_past` | a plane past `channels` |
| `offset_past` | an offset outside -1..1 |
| `bit_shared` | two outputs or state bits on one bit |
| `halo_written` | an input at a non-zero offset on a plane an output or state bit is on |
| `chip_refused` | the chip does not parse |
| `chip_too_big` | at `$.gates`: the chip compiles to more than the 16384 nodes a law holds; search with fewer gates |
| `law_refused` | the law it makes passes another law limit |

`grit_world_read` and `grit_world_slice` take `plane` to read a law world's other planes, and `grit_world_write` takes `plane` to write one: 0 if left out, as before. A plane past the world's planes is refused `outside` at `$.plane` and nothing is written. A write to plane 1 or above names `plane` in its answer, is journaled with its plane and comes back after a restart; a plane 0 write answers and is journaled exactly as before.

The X0 edge task, a 24-gate chip that `grit_machine` learned from 3x3 neighbourhoods (input `k` is row `k / 3`, column `k % 3`, from the top left):

1. `grit_law_from_chip` with `inputs` 9, `outputs` 1, the chip's `gates`, `bind_inputs` `{"plane":0,"dx":k%3-1,"dy":k/3-1,"dz":0,"bit":0}` for `k` 0 to 8, `bind_outputs` `[{"plane":1,"bit":0}]`, `channels` 2: `halo` 1, `quiet` `proved`.
2. `grit_world_create` with `wide` 32, `deep` 32, `tall` 2 and `law` from step 1.
3. `grit_world_write` the image to plane 0, `[x, y, 0, pixel]`.
4. `grit_world_step` one frame.
5. `grit_world_read` from `[0,0,0]` to `[31,31,0]` with `plane` 1: each cell is the chip's answer on its 3x3 neighbourhood.

The world is a torus, so a 32-wide world reads row 0 next to row 31: on X0's held images 24 of 8192 cells then differ from the chip on X0's zero border. For a border that reads 0, make the world at least twice the image on each side and write the image at the origin: a 64 by 64 world gives X0's answers on all 8192 cells.

## Row order

A table row, a chip input and a learned law's stencil all use one order:

- Row bit `j` is input `j`: the row for inputs `x` is the sum of `x_j` times `2^j`, and output `j` is bit `j` of the row's word. `grit_synthesise`, `grit_least`, `grit_check` and `grit_credit` take and answer tables in it. Their answers name it as `row_order`.
- `grit_machine` takes feed bit `j` as input `j` and want bit `j` as output `j`.
- In `grit_law_learn`, input `j` reads read `j / positions` at stencil place `j % positions`. Place `k` is the offset `(k % 3 - 1, k / 3 % 3 - 1, k / 9 - 1)` as `(dx, dy, dz)`, with `dz` 0 for 3x3. So 3x3 place `k` is row `k / 3`, column `k % 3`, from the top left.

## Learning a law from examples

`grit_law_learn` takes example pairs of input and wanted output and answers a law for `grit_world_create`, in the form `grit_law_from_chip` answers.

| Field | Meaning |
|---|---|
| `stencil` | `3x3` (9 places, `dz` 0) or `3x3x3` (27 places) |
| `reads` | `{plane, bit}`, each read at every place; places times reads at most 27 |
| `writes` | `{plane, bit}`, written at the cell itself |
| `border` | `zero`: a neighbour past an example's edge reads 0. `wrapped`: an inline example wraps on itself, and a world example's box reads its neighbours from that world |
| `rule` | `minimal`, `symmetric`, `totalistic`, `outer` or `linear`, below |
| `examples` | inline `{size, input, output}`, with one word list per read and per write, x fastest, at most 4096 words a list; or `{world, output_world, from, to}` for larger ones, up to 2^20 cells over all examples |
| `gates`, `crowd`, `rounds`, `seed` | the search's effort, as `grit_least` |
| `start` | under `minimal` and `symmetric`, a circuit to begin from; see [A start](#a-start) |
| `noise` | under `totalistic` and `outer`: `refuse` (the default) or `vote`; see [Noisy examples](#noisy-examples) |
| `places`, `place_list` | under `totalistic` and `outer`: the places counted; see [Counted places](#counted-places) |
| `min_support` | under `noise` `vote`: lists the counts shown by few cells in `thin`; see [Noisy examples](#noisy-examples) |
| `steps` | under `totalistic` and `outer`: k, the law's applications between input and output, 1 if left out; see [Examples k steps apart](#examples-k-steps-apart) |
| `parallel` | `true`: the law in bit-parallel form, 32 independent copies in one word; see [32 copies in one word](#32-copies-in-one-word) |

The server builds every example cell's row, counts the rows, and refuses `conflict` at `$.examples` when one row is seen with two outputs. The refusal's `rows` lists each such row with every output and where it was first seen. The search is `grit_machine`'s, over the seen rows only, so 27-input stencils work. It walks a gate ladder down from `gates` as `grit_least` does and keeps the smallest circuit that answers every seen row.

Rows the examples never showed are decided by the rule:

- `minimal`: by the smallest circuit found.
- `symmetric`: the seen rows are first checked under the stencil's 8 (3x3) or 48 (3x3x3) rotations and reflections. Seen rows that are images of each other with different outputs are refused `asymmetric` at `$.rule`, and `orbits` lists them. The ladder then searches over every image of every seen row. The law answers output `j` 1 exactly when that circuit answers 1 on some image of the neighbourhood, so it turns with its input.

The answer adds `stencil`, `row_order`, `border` and `runs_as` (how to run it so it reads as the examples did). It also adds:

- `examples`: `cells`, `rows` (two to the inputs), `seen` and `conflicts`.
- `found`: the ladder's smallest solved budget, its live gates and its circuit.
- `chip`: the law's own gates.
- `ladder`: every rung tried.
- `unseen`: `rows`, `rule`, `means`, and per output the `zeros` and `ones` decided; for 9 inputs or fewer also the full `table` and the `unseen_rows`.
- `stopped`: `ladder`, `budget` or `cancelled`.
- `crowd_capped` and `gates_capped`: `{asked, used, why}`, present only when the crowd or the ladder's start was lowered.

A call stops on its deadline or cancel only at a brake: before each build that measures the device, before each build of a search and before each device submission of a look. No measured time is compared with the time left. Once the deadline has passed no further submission starts, so an answer can come late by the one submission in flight: a build, or one submission of a look, sized by the device's measured pace. A build's kernel loops over the rows searched, every image of every seen row under `symmetric`, so it does not grow with them. Before the ladder the call builds the kernel of its smallest rung, then larger ones while each build's first submission (where a driver that compiles at its first dispatch, such as lavapipe, compiles), grown with the square of the kernel's words from the closest one measured, stays within half the device's patience (`GRIT_PATIENCE_SECS`). A top rung past that is refused `learn_limit` at `$.gates` with the most gates that fit. That limit is measured on this device at this moment, so it is outside the same-bytes rule below. An answer with `stopped` `ladder` is the same bytes for the same request on the same device; a stop on `budget` or `cancelled` depends on when the deadline passed or the cancel came, and its ladder is a prefix of that answer's ladder. A stopped ladder answers the smallest circuit solved so far, and refuses `budget` or `cancelled` when there is none. A ladder that solves nothing is refused `unsolved` at `$.gates`. The seen rows, and under `symmetric` their images, are refused `learn_limit` before any search when they pass the rows one search kernel holds (WORK / (gates + inputs + outputs) times 32); the expansion stops as soon as it passes them. The expansion checks budget and cancel every 4096 rows. The count of decided rows (at most 2^(inputs - 6) blocks of 64 rows times the inputs plus the law's gates) checks only cancel, every 4096 blocks, and none after a ladder that stopped; otherwise it runs to its end, so a found law is never lost to the deadline. A rung's crowd starts at most at what one device binding holds and doubles up to 65536 or that limit, whichever is less; a lowered crowd is named in `crowd_capped`. Under `symmetric` the ladder starts at most at the gates whose 8 or 48 copies and their ors fit the 600 gates a law always holds (74 for 3x3, 11 for 3x3x3 with one output); a lowered start is named in `gates_capped`. Other refusals: `example`, `outside`, `reversed`, `plane_past`, `halo_written` (a read on a plane a write writes), `bit_shared`, `learn_limit` (inputs past 27, cells past 2^20, rows past one kernel, gates below writes, not even a crowd of 64 fitting one device binding, a top rung whose build the device's evidence puts past half its patience), `no_world`, `chip_too_big` and `no_compute_device`.

### A start

Under `minimal` and `symmetric`, `start` is a circuit to begin from: `inputs`, `outputs`, `state` (0 if left out; a learned law holds none) and `gates`, as `grit_law_from_chip` takes a chip. The `chip` of an earlier `grit_law_learn` answer passes straight in. Under `symmetric` the law is the or of the start's images, so pass the `found` chip of a symmetric answer; its 8 or 48 copies must fit as `gates` does.

Refused before any device work:

| Code | When | `field` |
|---|---|---|
| `start_shape` | `inputs` other than the stencil's places times the reads, `outputs` other than the writes, or `state` other than 0; `inputs` and `outputs` in the refusal are the counts wanted | `$.start` |
| `chip_refused` | a gate reads a source not before it, or its kind is past 15; the gates hold fewer than `outputs` | `$.start.gates[k]` or `$.start.gates` |
| `learn_limit` | more live gates than a rung takes: 256, or under `symmetric` the gates whose copies and ors fit a law (74 for 3x3 with one write); `limit` names it | `$.start` |
| `start_rule` | `start` under `totalistic` or `outer` | `$.start` |
| `noise_rule` | `noise` `vote` under `minimal` or `symmetric` | `$.noise` |
| `places_rule` | `places` or `place_list` under `minimal` or `symmetric` | `$.places` or `$.place_list` |

Only live gates count: gates no output reads are dropped first. The server answers the start's law (its live gates under `minimal`, the or of their images under `symmetric`) on the host on every seen row and, under `symmetric`, on every image of one. Under `symmetric` "answers every seen row" is about that or of images, not the circuit alone, so a 3-gate circuit (the centre and not all of north, south and east) holds for the X0b pairs: its images together ask for all four.

- A start that answers every seen row is a solved rung before any search. It is the first entry of `ladder`, `{"gates", "solved": true, "start": true}`; the ladder then walks down from its live gates less one. The answer is never larger than the start, and a stop on `budget` or `cancelled` before a smaller circuit is solved answers the start itself, with that `stopped`. When the expansion of the images is the step that stops, `unseen` has no `by_symmetry`.
- A start that misses seen rows is no rung. It is one member of the first rung's crowd, padded to the rung's gates with constant gates in front, and the search keeps it unless a candidate scores at least as well: its score is counted on the host and seeded with it. The rest of the crowd is drawn as without a start; the search's neighbour choice spreads the start's offspring, and the random rest keeps the crowd varied. The ladder starts at `gates`, or at the start's live gates when that is larger, and only the first rung is seeded.

The answer adds `start`:

| Field | Meaning |
|---|---|
| `gates` | the start's live gates |
| `solved` | whether it answers every seen row |
| `checked` | what was checked, in words |
| `wrong`, `wrong_rows` | the seen rows it answers wrongly; at most 64 listed, each with `row`, `outputs` (wanted), `answered` and where it was first seen (`example`, `at`) |
| `images_wrong` | under `symmetric`: the images of seen rows it answers wrongly |
| `changed`, `changed_seen`, `changed_unseen`, `changed_rows` | the rows on which the answer's law decides other than the start's law; at most 64 listed, seen rows first, each with `row`, `seen`, `start` and `law` |
| `changed_over` | `every row` for 9 inputs or fewer (3x3 with one read), else `the seen rows` |

An answer with `stopped` `ladder`, and the start itself after a stop, are the same bytes for the same request. A request without `start` answers as before.

### Counting laws

`totalistic` and `outer` take exactly one read and decide every row by a count, with no search:

- `totalistic`: output `j` is `T_j[n]`, where `n` counts the stencil's places, centre included, whose read bit is 1: 0 to 9 for 3x3, 0 to 27 for 3x3x3.
- `outer`: output `j` is `T_j[c][n]`, where `c` is the centre's read bit and `n` counts the other places: 0 to 8 or 0 to 26.

The rows are built and refused (`conflict`) as above. Each table is then read from the seen rows in one pass. Two seen rows with the same `n` (and, for `outer`, the same `c`) but different outputs for one write are refused `not_counting` at `$.rule`. Its `counts` lists, for each such count and write, `count` (and `centre` for `outer`), `write` and `seen`: one row with output 0 and one with output 1, each where it was first seen (`example`, `at`). `not_counting` is how many there are. More than one read is refused `count_reads` at `$.reads`. A count no example showed is decided 0.

The law counts with a gate circuit through the same chip wiring as the other rules: an adder tree sums the counted places, a decoder turns the sum into one line a count, and each output ors the lines whose entry is 1 (or the negated or of those that are 0, when fewer). For 3x3x3 with 8 writes under `outer` it stays under the 600 gates a law always holds, so it never meets `chip_too_big`. A law built this way turns with its input under every move of the stencil, since a count does not see the order of the places.

The answer has the fields above except `found`, and:

| Field | Meaning |
|---|---|
| `tables` | per write: `T[n]` for `totalistic`; `[T[0][n], T[1][n]]` for `outer` |
| `unseen_counts` | per write, in the same shape: the counts no example showed, each decided 0 |
| `unseen` | as above, with `rule` and `means` naming the counting rule |
| `ladder` | `[]` |
| `stopped` | `no_search` |

The call builds no search kernel and has no brake: it reads the rows, then builds the circuit. With inline examples it needs no compute device. The same request gives the same bytes. `gates`, `crowd`, `rounds` and `seed` are not used.

### Counted places

By default a counting law counts every place of the stencil. `places` names a smaller neighbourhood by preset, and `place_list` names one by place indices in the [row order](#row-order). Give one of them, not both.

| `places` | Stencil | Place indices |
|---|---|---|
| `moore` | both | every place: the default |
| `von_neumann` | 3x3 | 1, 3, 4, 5, 7: the centre and its 4 face neighbours |
| `von_neumann` | 3x3x3 | 4, 10, 12, 13, 14, 16, 22: the centre and its 6 face neighbours |
| `hex` | 3x3 | 1, 2, 3, 4, 5, 6, 7: the centre and the axial hex neighbours (+1,0) (-1,0) (0,+1) (0,-1) (+1,-1) (-1,+1) as (dx, dy) |

`hex` runs a hex world stored as a 2D grid in axial coordinates, the two diagonal places 0 and 8 left out. Under `totalistic` the count runs over the named places, the centre included. Under `outer` it runs over them less the centre, and the centre stays the separate bit. The law's circuit reads no other place, so the counts, `tables`, `unseen_counts`, `not_counting`, `unseen` and the vote all shrink to the named places. The answer adds `places`: `list`, the sorted indices, and `preset` when one was given. A `place_list` equal to a preset answers the same law.

Refused before any example is read:

| Code | When | `field` |
|---|---|---|
| `places` | a preset the stencil lacks (`hex` on 3x3x3) | `$.places` |
| `places` | an index outside the stencil, or one named twice | `$.place_list[k]` |
| `places` | a list without the centre (4 for 3x3, 13 for 3x3x3), or both `places` and `place_list` | `$.place_list` |

A request without `places` or `place_list` answers the same bytes as before.

3x3 majority (1 when at least 5 of the 9 places are 1) from 12 random 32x32 examples: `rule` `totalistic` answers `tables` `[[0,0,0,0,0,1,1,1,1,1]]`. Life is `outer` with `[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]`.

The X0b edge task: 24 inline 32x32 pairs, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `border` `zero`. `minimal` and `symmetric` see 125 rows of 512, refuse nothing, and find the same 4 gates: the centre is 1 and not all four of its orthogonal neighbours are. Under `minimal` that is the law; under `symmetric` it is 8 copies and 7 ors, 39 gates, with the same table. Run it as `runs_as` says: each image at the origin of a 64x64x2 world, plane 0, one step, plane 1 read. `totalistic` and `outer` refuse it `not_counting`: a centre with 4 neighbours set is an edge or not by which 4 they are.

### Noisy examples

Real examples are rarely clean: one flipped cell in 16384 is enough for `conflict`. Under `totalistic` and `outer`, `noise` `vote` decides each count by the example cells instead:

- Each example cell is one vote, for its write's output, in its count's class: `n`, and `c` too under `outer`.
- Each table entry is the majority of its votes. A tie is 0 and listed in `ties`. A count no cell showed is 0 and listed in `unseen_counts`, not in `ties`.
- No row is refused `conflict` and no count `not_counting`; `examples.conflicts` stays 0. Every other refusal stands.

The answer adds:

| Field | Meaning |
|---|---|
| `tallies` | per write, `[zeros, ones]` for each count, in the shape of `tables`; a write's tallies sum to the example cells |
| `ties` | per write, the tied counts, in the shape of `unseen_counts` |
| `deviating` | per write, the example cells whose output disagrees with the law |
| `deviating_cells` | per write, at most 64 of those cells in example order, then z, y, x: `example`, `at`, `count`, `centre` (under `outer`), `seen` and `law`; `at` is the cell's place in the example's box, so for a world example it is counted from the example's `from`, not from the world's origin |
| `deviating_share` | deviating cells over voted cells (example cells times writes), as an exact `[num, den]` |

`min_support`, from 1 (if left out) up to the stencil's places times 64, marks the counts that rest on little evidence. The answer then adds `thin`: per write, in the shape of `ties`, each count shown by at least one and fewer than `min_support` example cells, as `{count, tally}` with `tally` its `[zeros, ones]`. Such a count is still decided by the vote, so the law does not change. `min_support` without `noise` `vote` is refused `support_noise` at `$.min_support`; 0 is refused `malformed` there, and past the stencil's limit (576 for 3x3) `support_limit`.

Where `refuse` answers a law, `vote` answers the same bytes with only these fields added, so `deviating` is 0. `refuse` answers exactly as a request without `noise`. The same request gives the same bytes.

Life from 16 32x32 torus pairs, 5% of the cells flipped in the input and the output: `vote` answers `[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]`, and `deviating` names the cells to distrust. A crop of a larger world with `border` `zero` reads zeros past its edge, so its deviating cells lie on the crop's outer ring.

### Examples k steps apart

Measurements are often taken every few steps. With `steps` k from 2 to 8 (`limits.law_steps_most`), under `totalistic` or `outer`, each example's output is the read bit after k applications of one table, with the example's border at every step:

- `zero`: a neighbour past the example's box reads 0 at every step.
- `wrapped`: an inline example wraps on itself at every step; a world example's box is read with k cells around it from that world, so it equals k steps of the law on that world.

The law reads its input around each cell and writes its output, so it takes exactly one read and one write, and the write is on the read's bit but on another plane: `reads` `[{"plane":0,"bit":0}]` with `writes` `[{"plane":1,"bit":0}]`. A write on the read's plane is refused `halo_written` (a law may look only at a plane it does not write). The examples' outputs are the read bit after k steps; the answer's `iterate` runs the law with `alternate` on those two planes, so each frame reads the plane the last one wrote.

The answer is one step of the rule. The rows of one step are not seen, so there is no `conflict` or `not_counting`. Up to 18 classes every table of the rule and its counted places is searched, 2 to the classes of them; past that the search is local (see [Large rules and noise](#large-rules-and-noise)). A table is numbered by the sum of `T[class]` times 2^class, class `n` under `totalistic` and `c` times the counts plus `n` under `outer`; Life is 6152. Tables are taken in ascending number.

The search runs on the host, needs no device and reads no clock. It takes 64 tables at a time: each is first run on the cells a few probe cells depend on, then on every example cell. A table is dropped only once it is wrong on more cells than a table already seen, so the count of consistent tables and the closest table are exact.

The answer has the fields of a counting law except `unseen`, `unseen_counts` and `ladder`, with `examples` holding `cells`, and adds:

| Field | Meaning |
|---|---|
| `steps` | k |
| `consistent` | the tables whose k-fold application gives every example cell |
| `consistent_tables` | the first 16 of them, in the shape of `tables`; the law is the first |
| `checked` | the tables tried or excluded |
| `space` | the tables of the rule, 2^classes |
| `stopped` | `complete`, `budget` or `cancelled` |
| `complete` | whether every table was checked |
| `iterate` | `alternate` (the read's and the write's planes) and `frames` (k), to run it on a world |

`consistent` above 1 means the data does not pin the rule down. In `tests/law_steps.rs`, Life seen every 4th frame in four seeded 32x32 soups (fill 0.35, two of them after 10 warm-up frames) never shows a dead cell with 8 live neighbours, so `T[0][8]` is free and `consistent` is 2; eight such soups name Life alone.

A brake before each 64 tables stops on budget or cancel; an answer can come late by the check of 64 tables. A stopped search answers what it found, a prefix of the whole search's order, with `complete` false, and refuses `budget` or `cancelled` when it found none. A complete search answers the same bytes for the same request. The found law is run k steps on the host on every example before it is answered.

1. `grit_law_learn` with `stencil` `3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `border` `wrapped`, `rule` `outer`, `steps` 4 and eight such pairs 4 frames apart: `tables` Life, `consistent` 1.
2. `grit_world_create` with the example's size, its `law` and `alternate` `[0,1]`.
3. `grit_world_write` the input, `grit_world_step` 4 frames, `grit_world_read` with `current` true: the example's output.

`steps` 1 answers exactly as a request without `steps`. Refused before any example is read:

| Code | When | `field` |
|---|---|---|
| `malformed` | `steps` 0 | `$.steps` |
| `steps_limit` | `steps` past 8 | `$.steps` |
| `steps_rule` | `steps` above 1 under `minimal` or `symmetric` | `$.steps` |
| `steps_noise` | `steps` above 1, or `search`, with `noise` `vote` | `$.noise` |
| `steps_writes` | not exactly one write, on the read's bit: the law reads back what it wrote | `$.writes` |
| `steps_space` | `search` `exhaustive` with more than 18 classes (`limits.law_steps_classes`, 262144 tables); `space` and `classes` name the size | `$.steps` |
| `search_rule` | `search` under `minimal`, `symmetric` or `linear` | `$.search` |
| `noise_rule` | `noise` `least_wrong` or `likely` under `minimal`, `symmetric` or `linear` | `$.noise` |

A complete search that finds no table is refused `no_rule_k` at `$.examples`. `closest` names the first table with the fewest wrong example cells as `tables` and its `wrong` cells, with `cells`, `checked`, `space` and `steps`.


### Large rules and noise

`search` picks how the tables are searched. `noise` `least_wrong` or `likely` takes noisy examples. Each goes with `totalistic` or `outer` and any `steps` from 1 to 8, and each sends a request with `steps` 1 to this search too.

| `search` | What runs |
|---|---|
| `exhaustive` | every table, at most 18 classes; the default up to 18 classes |
| `local` | the default past 18 classes, such as 3x3x3 Moore under `outer` (54 classes) or `totalistic` (28) |

The local search:

1. Scores every table whose births and survivals (under `outer`) are each one interval of counts or none, or whose live counts (under `totalistic`) are one interval or none. It scores them on a window of the example cells: 143 641 tables for 3x3x3 `outer`.
2. Scores the 16 best of them on every example cell and starts from the best.
3. Flips single entries. When no single flip lowers the score, it flips pairs of entries, at least one of them reached by the table's own run. It keeps a flip only when it lowers the score, and stops when neither kind does.

Every table is scored on the host, 64 at a time, by running it k steps on the cells the example cells depend on. The search reads no clock and answers the same bytes for the same request. Under `noise` `refuse` a search that ends with wrong cells is refused `no_rule_k` with `closest` and `search`; a local search is no proof that no table fits.

#### The table with the fewest wrong cells

Under `noise` `least_wrong` the score is the wrong example cells after k steps. Both kinds of search add:

| Field | Meaning |
|---|---|
| `search`, `noise` | what ran |
| `wrong`, `share` | the wrong example cells, and `[wrong, cells]` |
| `margins` | in the shape of `tables`: how much flipping that entry alone would raise the score. 0 means the data does not pin the entry |
| `pinned` | in the shape of `tables`: the entry where the data pins it, `null` elsewhere |
| `unpinned` | entries the answer's own k-step run uses but the data does not pin, each `count` and, under `outer`, `centre` |
| `unreached` | entries the answer's own run never uses |
| `work` | `units`: cell steps scored, each step of a cell scoring up to 64 tables |

#### The noise model and pinned entries

The noise model flips each cell of both frames at a rate p. The input frame is seen flipped, so the cells the law ran on are drawn from what the seen input allows. With d the example's own density less the flips, d = (seen density - p) / (1 - 2p), a seen live cell was dead with chance (1 - d)p / ((1 - d)p + d(1 - p)), and a seen dead cell was live with chance dp / (dp + (1 - d)(1 - p)). On sparse inputs most flips make live cells, so a large share of the seen live cells is noise. Flips at rate p leave at least a share p of every seen frame live and at least p dead.

Pinning is judged by surprise under the noise model (see "The most likely table"), whatever the search's own score. An entry is pinned when all of these hold:

- its margin is above 0;
- the answer's entry makes the examples more likely than the flipped entry by at least 20 nats, again by at least 20 nats under 16 other seeded draws, and by at least 4 standard errors of its sum over blocks. Each example is cut into blocks by halving each axis it spans (up to 8), and the standard error comes from how much the gap varies between blocks: cells of one block share inputs, so their nats are not independent. The draws are a sample, so a gap they make alone does not pin;
- the answer's own entry fits the cells the flip changes: the answer is wrong on at most the expected count plus four times its square root plus 4, where each cell is wrong with chance p plus (1 - 2p) times the share of 64 draws of the noisy input on which the cell's output moves;
- the best table found with the other value is less likely than the best table found with the answer's value by the same three tests. The tables come from single-flip descents by surprise: one from the answer, one from the answer with that entry flipped and held, one for each other entry that passed the tests above, plus the other family starts of a local search and, under `likely`, the `least_wrong` answer. Every table a descent scores counts.

The last test is the profile of the entry: seen 2 steps apart on sparse soups, a quite different table can explain the examples as well as the rule, and the entries where they differ stay `null`. The descents are local, so a far table that explains the examples better can still be missed.

The rate is the `noise_estimate` under `least_wrong` and the assumed rate under `likely`; with an estimate of 0 and wrong cells, it is the share of wrong cells. Without noise (no wrong cells under `least_wrong`) the score is wrong cells: an entry with a margin is pinned unless a descent finds a table with no wrong cells and the other value. When `far_above` is true no entry is pinned. The fewest wrong cells can differ from the rule even one step apart: on a 5% soup with 1% flips the flips make births the rule never made. Such entries have a margin but are not pinned.

`exhaustive` adds `tied` (the tables with the least score), `consistent` (the tables with no wrong cells), `checked` and `space`. `local` adds `family` (the best interval table and its score), `moves` (each kept flip: its entries and the score after it) and `checked` (the tables scored).

`noise_estimate` holds:

| Field | Meaning |
|---|---|
| `flips_per_million` | the cell flip rate on each frame that explains the wrong cells of the half of the example cells with the fewest. Cells are grouped by their input count and the law's output, and each half of the examples picks the groups for the other half. Flips of the input frame are run through the law k steps on 64 seeded draws, so the rate counts what a flipped input cell spreads to |
| `expected_wrong` | the wrong cells that rate explains over every example cell |
| `basis_cells` | the cells the rate was fitted on |
| `far_above` | true when the wrong cells pass twice `expected_wrong` plus four times its square root plus 16, or when explaining the half with the fewest needs a rate above what the seen frames allow: the least share of live or dead cells of any seen frame plus four standard errors. `warning` then says that the data is not this rule with noise |

The estimate assumes the same flip rate on both frames: examples with a clean input frame read as a lower rate. `far_above` false is no proof that the examples come from a counting rule or that the table is the rule. From 2 steps on, a rule of another family, or a quieter table of the same family, can read as a counting rule with a high flip rate, because the input count of a cell says little about its output two steps on.

#### The most likely table

The fewest wrong cells is the wrong aim when an input flip spreads. Seen 4 steps apart with 0.5% flips on both frames, a 3x3x3 rule's own table is wrong on more cells than quieter tables: every output cell's past holds hundreds of input cells, so a few of them are flipped. `noise` `likely` aims at the table under which the examples are most likely:

1. It runs the `least_wrong` search and starts from a sixteenth of its `flips_per_million`, at least 1.
2. It searches again, by the same steps, for the table with the least surprise at that rate. Surprise sums, over the example cells, minus the log of the chance the table gives the cell's output, in millionths of a nat. The input is drawn from the seen input on 16 seeded draws as the noise model says and run k steps; the chance of a live output is p plus (1 - 2p) times the share of draws that end live, at least one in a million.
3. It fits the rate that gives that table the least surprise, by 12 golden-section steps over an eighth to eight times the rate, at most the rate the seen frames allow. When the fit is more than a fifth off, it searches again from that table at the fitted rate, at most three searches in all.

The answer adds `assumed_flips_per_million` (the rate of the last search) and `surprise`; `margins`, `family` and `moves` count surprise. A request whose `least_wrong` noise estimate is 0 answers the `least_wrong` table with `assumed_flips_per_million` 0, whatever its wrong cells.

#### Stopping

A brake before each round of at most 64 batches of 64 tables, each pass of single or pair flips, the margins, each step of the estimate, each round of the pin descents, each step of the rate fit and each search of `likely` stops the search on budget or cancel. A stop at any brake answers the best table found so far with `stopped` `budget` or `cancelled`, `complete` false, `noise` as asked, and no `margins`, `pinned`, `unpinned`, `unreached`, `noise_estimate` or `warning`, so it claims no entry. Under `likely` a stop before the likelihood search scored a table answers the `least_wrong` table that way. It is refused `budget` or `cancelled` when it scored no table on every example.

1. `grit_law_learn` with `stencil` `3x3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `border` `wrapped`, `rule` `outer`, `steps` 2, `noise` `least_wrong` and ten noisy 32x32x32 pairs 2 steps apart: `search` `local`, `tables` B5,6/S4,5,6, `noise_estimate` near the data's flip rate, `far_above` false.
2. Read `pinned`: entries with `null` are not learned. Counts no example reaches, such as 24 to 26 live neighbours, stay `null`.
3. With the same pairs 4 steps apart, give `noise` `likely`.
4. Run the law as in the steps above with `iterate`.

### Fixed-point linear laws

`rule` `linear` learns a law over numbers: heat, concentrations, densities. Each read and written plane holds one signed fixed-point number a cell, the whole 32-bit word in two's complement with `frac` fractional bits (16 if left out, at most 30): the word as a signed integer over 2^`frac`. A read or write names `plane` and `bit` 0.

| Field | Meaning |
|---|---|
| `poly` | 1 (default): the read words at the counted places. 2 adds the product of the centres of each two reads and each centre times the sum of a read's other counted places. 3 adds the product of each three centres, so Gray-Scott's u·v² is a term |
| `frac` | the fractional bits of every read and written word |
| `places`, `place_list` | the counted places, as for the counting rules |
| `clamp` | `[least, greatest]` signed words each written word is clamped to; it must meet the words the law can write, else `clamp_range` |
| `range` | `[least, greatest]` signed words of every read the law never wraps on; it must hold every example input word, else `plane_format` at `$.range`. The examples' own least and greatest if left out |

The fit is exact least squares over every example cell. The normal equations are summed in integers, centred and scaled, and solved in f64 by Cholesky; the answer's `condition` is the ratio of the largest to the smallest eigenvalue of the centred, scaled normal matrix. Past 1e10, or with a term the same on every cell, the request is refused `ill_conditioned` at `$.examples`, with `condition`.

The law computes, with every add and multiply wrapping on 32 bits:

- `rnd_s(v) = floor(v / 2^s + 1/2)`: round to nearest, ties toward +infinity, as `((v + 2^(s-1) + 2^31) >> s) - 2^(31-s)` with a logical shift. A product term is `rnd_frac` of the 32-bit product; a cube rounds after each product.
- `y = ((D + sum_j C_j * T_j) >> k) - 2^(31-k)`, with `D = B + 2^(k-1) + 2^31`, then `clamp`.

`C_j` and `B` are the fit's coefficients and bias rounded to `k` fractional bits; `k` is the largest from 30 down whose sum of coefficients times terms over `range` fits one word, so inside `range` no step wraps. On a device the law equals this formula bit for bit. When no `k` fits, or a product passes 31 bits, the request is refused `fixed_range` at `$.frac`, and `limit` names the largest `frac` that fits.

At `k` fractional bits every written word is in `[-2^(31-k), 2^(31-k) - 1]`, so `clamp` is applied in that interval. A `clamp` that meets it holds: every written word is in `clamp`. Each write has its own `k`; a `clamp` wholly outside the narrowest of these intervals is refused `clamp_range` at `$.clamp`, and `limit` names that interval as `[least, greatest]`. A wider `range` lowers `k` and widens it.

These refusals need the examples and are checked in this order; the first that applies is the one named: `ill_conditioned`, then `fixed_range`, then `clamp_range`. `fixed_range` comes after `ill_conditioned` because its `limit` is found from the fit's coefficients, which an ill-conditioned fit does not determine; `clamp_range` needs `k`, which `fixed_range` chooses.

The answer has `law`, `hash`, `halo`, `quiet`, `stepping` and `nodes` as above, and:

| Field | Meaning |
|---|---|
| `terms` | in order: `place` (`read`, `place`, `at`), `product` and `cube` (`reads`, the centres multiplied), `centre_times_others` (`reads`: the centre of the first times the sum of the second's other counted places) |
| `coefficients` | per write, per term: `value` (the exact decimal of `fixed` over 2^k), `fixed` (C_j), `fit` (the f64 solution) and `stderr` |
| `bias` | per write: `value` (B over 2^(frac+k)), `fixed` (B), `fit`, `stderr` |
| `fixed` | `frac`, `coefficient_frac` (k per write), `format`, `rounding`, `range` per read, and `clamp` |
| `residual` | per write, of the law as it runs on every example cell: `max` and `rms` in words, `sum_squares` exact, `max_real`, `rms_real`, and `fit_rms_real` of the f64 fit. Noise on the outputs shows here |
| `symmetry` | `group` (`D4` for 3x3, `Oh`, order 48, for 3x3x3) and `invariant`: per write the place coefficients of each read and their images under every move differ by at most max(2^-k, 6 standard errors of the difference); `deviation`, `tolerance`, `read` and `places` name the pair furthest past it. Only place terms are compared: `product`, `cube` and `centre_times_others` terms are left out |
| `iterate` | one read and one write: `alternate`; as many reads as writes: `copy`, the written plane and the read plane to write it back to after each frame |

It needs no device and has no brake. Refused before any example is read: `plane_format` (a bit other than 0, `frac` past 30), `poly_limit` (`poly` past 3), `learn_limit` at `$.reads` (reads times counted places past 27), `linear_rule` (`poly`, `frac`, `clamp` or `range` under another rule, and `law_frac`, `settle`, `params` or `coefficient_frac` under another rule), `steps_rule` (`law_frac`, `settle`, `params` or `coefficient_frac` without `steps` or `hidden`), `reversed` (a `clamp` or `range` whose least is past its greatest), `start_rule`, `noise_rule` and `parallel_rule` (`parallel` true). With `steps` or `hidden` the fit runs through k steps, below. An answer whose coefficients keep fewer than 12 fractional bits adds `warnings` with `coefficient_bits` (X1's `frac` 24 kept 7).

Heat from four 64x64 pairs, `alpha` 0.15: `stencil` `3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `border` `wrapped`, `places` `von_neumann`: the four face coefficients are alpha, the centre 1 - 4 alpha. Run it with `alternate` `[0,1]`. Gray-Scott reads u and v on planes 0 and 1, writes planes 2 and 3, with `poly` 3 and `frac` 14.

### Fitting through k steps, with hidden planes

`steps` or `hidden` turns `rule` `linear` into a fit through the law's own stepping. Each example is a run of shots k law steps apart, and the law is fitted so that k applications of it carry one shot to the next.

| Field | Meaning |
|---|---|
| `steps` | k, from 1 to 64: the law's applications between two shots |
| `hidden` | 0 or 1: planes the examples do not show, run by the law beside the reads |
| `settle` | shots at the start of each example from which no rollout is scored, so the hidden planes settle; 0 if left out |
| `law_frac` | the law's frac: a whole number from 0 to 30, or `auto`; `frac` if left out. The examples stay at `frac` |
| `params`, `coefficient_frac` | an earlier answer's params and `fixed.coefficient_frac`: no fit, only the hidden planes are run (for a start state) |

An example is `{size, input, output}` (two shots) or `{size, sequence}`, where `sequence` lists the shots, each one word list per read. World examples are refused `example`. Each write is the next value of the read at its position, so `reads` and `writes` have the same length, else `steps_writes`. Every written word is clamped to `range` (a hidden plane to minus to plus its largest magnitude), so an iterated law never wraps; `clamp` is refused `linear_rule`.

The terms are those of the one-step rule over the reads. A hidden plane enters as its centre word and, under `poly` 2 or 3, as its product with each read's centre (Aliev–Panfilov's u·w); it has no other places and no square or cube. Each write has its terms and a bias. More than 32 params (the law limit) is refused `learn_limit` at `$.poly`; more than 4 planes (reads, hidden and their writes) at `$.hidden`. Hidden planes take the lowest free planes: first their read planes, then their write planes.

The fit, on the host with no device:

1. The reads' one-shot least squares fit (as the one-step rule), split into k equal sub-steps: each coefficient's distance from the identity divided by k. Each hidden plane starts as a leaky copy of the read centre, fed back into it; the best of 12 such starts on one-shot rollouts is kept.
2. Levenberg–Marquardt on the law in f64 (no rounding). Its residuals are the read planes of rollouts of 1, 2, 4 and up to 16 shots. Rollouts start every horizon shots after `settle`; each start's read words are fitted too (a few L-BFGS rounds between steps, scored against the shot), so noise in a start does not bias the coefficients. The hidden planes of a start are the law's own run over the shots before it, from 0 at the example's first shot, with the reads interpolated linearly across the k sub-steps. The Jacobian is carried forward exactly through every sub-step.
3. Each hidden plane is scaled so its run spans half the range; the coefficients are rounded to k fractional bits, as the one-step rule does.

The answer has the one-step rule's fields and:

| Field | Meaning |
|---|---|
| `law` | carries its `alternate` (one pair per read, `[[read, write], ...]` for two or more) and `params`: coefficient j of write i is param i times (terms + 1) plus j, a write's bias the last; each is its fixed word plus 2^31 |
| `params` | the words to create the world with |
| `iterate` | `alternate` and `frames` k |
| `hidden` | `count`, `planes` (`read`, `write`), `start` (the rule above) and `identifiable` |
| `coefficients`, `bias` | per write: `value`, `fixed`, `fit`, `param`; with hidden planes `scales_as`: the law is unchanged when hidden plane h is multiplied by a and each coefficient by a^scales_as[h]. A coefficient whose `scales_as` is 0 is pinned by the examples; the others only in products whose powers sum to 0 |
| `fixed` | `frac` (the law's), `example_frac`, `frac_from` (`frac`, `given`, `auto` or `params`), `coefficient_frac`, `coefficient_frac_least` (12), `clamp` per write, `param_format` |
| `residual` | per read: `rms_real` and `max_real` of the fixed-point law one shot ahead from every scored shot, `fit_rms_real` of the f64 fit |
| `baselines` | in sample, NRMSE at `shots_ahead` 1 and the longest horizon of the law against `persistence` and `linear` (the reads' one-shot linear fit, clamped); `beats` when the law's is at most `margin` (0.8) times the better one |
| `stability` | per read write, the largest gain of its place coefficients over spatial waves |
| `stages` | per horizon, its LM rounds and rms |
| `work` | cell sub-steps evaluated, times 1 plus params for those that carry a Jacobian |
| `state` | per example, every plane at its last shot: the reads' words and the hidden planes' run; write them to continue on a world |
| `warnings` | `coefficient_bits`, `baselines_not_beaten`, `unstable_linear_part`, each with a message |

`law_frac` `auto` takes, of the fracs from 0 to 30 whose coefficients over `range` keep at least 12 fractional bits in every write, the one with the least rounding a step (half a unit of the frac, plus the term bounds times half a coefficient unit); none is refused `coefficient_bits` at `$.law_frac` with `limit` the most bits any frac keeps. A one-step or k-step answer whose coefficients keep fewer than 12 bits adds the warning `coefficient_bits`; other answers are unchanged.

Run it: `grit_world_create` with the answer's `law` and `params`, write each read (and each hidden plane, 0 or `state`) on its read plane, step `iterate.frames` frames: the read planes hold the next shot. To start mid-run, pass the shots before the start with `params` and `coefficient_frac`: the answer's `state` is the start.

Each step of the law's rounding is counted in `work`, EPF1's unit (a cell stepped once); the call has no brake. More than 2^26 cell sub-steps over the examples is refused `learn_limit` at `$.steps`.

### 32 copies in one word

A learned law reads one bit of each read and writes one bit of each write. With `parallel` true, under `minimal`, `symmetric`, `totalistic` and `outer` and with `steps`, `noise` and `start`, the answer is the same circuit in bit-parallel form: each read and each write is the whole word of its plane, and bit j of every word is copy j. Bit j of each written word is the single-bit law applied to bit j of the read words, for every j in 0 to 31, so one world runs 32 independent soups.

- Every read and write names `bit` 0; any other bit is refused `parallel_bit` at `$.reads[k].bit` or `$.writes[k].bit`. Two writes on one plane are then one bit twice, which stays `bit_shared`.
- Every bit of a written word is a copy's output, so `rest` has no bit to act on and the answer has no `rest`.
- `linear` already reads and writes whole words as numbers, so `parallel` true with it is refused `parallel_rule` at `$.parallel`.
- A NOT is an xor with 4294967295 and a constant 1 gate is 4294967295, so no copy leaks into another; the inputs carry no `and ..., 1` mask and the writes no shift.
- Learning does not change: the circuit, `tables`, `ladder` and every other field are the same bytes as without `parallel`. Only `law`, `hash` and `nodes` differ, and the answer adds `parallel`: `copies` (32), `bytes` and `nodes` of its law text, and `single` with the `bytes` and `nodes` of the single-bit form. The gates are the same.
- `parallel` false, or left out, answers the same bytes as before.

It runs with `alternate` as the single-bit law does: `[the read's plane, the write's plane]`, or a `steps` answer's `iterate`.

1. `grit_law_learn` Bays 5766 (B6/S5,6,7): `stencil` `3x3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `rule` `outer`, `parallel` true.
2. `grit_world_create` with `wide`, `deep` and `tall` 32, its `law` and `alternate` `[0,1]`.
3. `grit_world_write` with `current` true each cell's 32 soups as one word, soup j at bit j.
4. `grit_world_step` 64 frames, then `grit_world_read` with `current` true: bit j of each word is soup j at generation 64.

| Law (3x3 Life / 3x3x3 Bays, outer) | Gates | Bytes, single / parallel | Nodes, single / parallel |
|---|---|---|---|
| Life | 39 | 1788 / 1680 | 573 / 405 |
| Bays 5766 | 127 | 5010 / 4731 | 1605 / 1149 |

