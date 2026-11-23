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
| `halo_sparse` | (world) a sparse world, at create (`$.sparse`) or open (`$.name`); make it dense |

A host write to a world whose law looks wakes the 27 octs around the oct of each phase that holds the cell, so every oct whose 4x4x4 box holds the cell steps next frame. The quiet class is decided as below, with the looked cells part of the oct's input: a law that reads neither `frame` nor `at` gives the same output for the same box, so it keeps the box contract and steps listed; any other law steps whole.

`tests/halo3/` holds three laws that look: a 2D Sobel edge detector, a 3D 3x3x3 Sobel and a blur over two planes.

## Iterating a law on the device

A law that reads plane `a` and writes plane `b` gives one generation a frame. `alternate` `[a, b]` makes it iterate with no host round trip: frames of even parity (the world's frame counter, bit 0) run the law as written, looking at `a` and writing `b`; frames of odd parity run it with `a` and `b` swapped in its inputs and outputs. After `k` frames from frame 0 generation `k` is on the current plane: `a` at an even frame, `b` at an odd one.

`grit_world_create` takes `alternate` next to `law`; for a `grit_law_learn` answer it is `[the read's plane, the write's plane]`, and the law text is passed unchanged. A law may also carry the key itself. The world's law is then the iterating law, with its own hash; the answer adds `alternate`. The alternation is part of the law's text and its parity is the frame's, so both come back after a journal replay, `grit_world_fork`, `grit_world_restore`, `grit_world_save` and `grit_world_open` (also at an odd frame), `grit_world_params` and a lost device.

1. `grit_law_learn` Life: `stencil` `3x3`, `reads` `[{"plane":0,"bit":0}]`, `writes` `[{"plane":1,"bit":0}]`, `rule` `outer`.
2. `grit_world_create` with `wide` 64, `deep` 64, `tall` 2, its `law` and `alternate` `[0,1]`.
3. `grit_world_write` the soup with `current` true.
4. `grit_world_step` 64 frames: `current` 0.
5. `grit_world_read` with `current` true: generation 64.

`grit_world_read`, `grit_world_slice` and `grit_world_write` take `current` true for the current plane, and their answer names `plane`; `grit_world_step` answers `current`. A world made without `alternate` refuses `current` with `not_iterating` at `$.current`; `current` with a `plane` other than 0 is `malformed` at `$.plane`.

Within a frame the looked-at plane is frozen, as `halo_written` requires. Refused as `law_refused` at `$.alternate`, with the law's own code:

| Code | When |
|---|---|
| `alternate_plane` | `a` equals `b`, or either is past `channels` |
| `alternate_written` | the law writes `a` |
| `alternate_unwritten` | the law leaves `b` unchanged |
| `alternate_other` | the law writes a plane besides `b` |
| `alternate_unread` | the law never reads `a` |
| `alternate_reversible` | the law is reversible; `grit_world_back` on an iterating world is `not_reversible` |

`alternate` with `rule`, or with a law that carries another `alternate`, is `malformed` at `$.alternate`. A law that looks runs only dense (`halo_sparse`), with or without `alternate`; one that does not look iterates on sparse worlds too.

The quiet class is decided on the law as written, with every uniform value checked at both parities. A listed world steps an oct while it or an oct around it changed in the two frames before; an oct that changed its written plane wakes the octs whose box reads it, so moving patterns are never skipped, and a world that stops changing, a period-2 oscillator included, goes quiet. Its frames are priced as every frame: cells times frames, calibrated under the iterating law's own text.

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
| `rule` | `minimal`, `symmetric`, `totalistic` or `outer`, below |
| `examples` | inline `{size, input, output}`, with one word list per read and per write, x fastest, at most 4096 words a list; or `{world, output_world, from, to}` for larger ones, up to 2^20 cells over all examples |
| `gates`, `crowd`, `rounds`, `seed` | the search's effort, as `grit_least` |
| `start` | under `minimal` and `symmetric`, a circuit to begin from; see [A start](#a-start) |
| `noise` | under `totalistic` and `outer`: `refuse` (the default) or `vote`; see [Noisy examples](#noisy-examples) |
| `places`, `place_list` | under `totalistic` and `outer`: the places counted; see [Counted places](#counted-places) |
| `min_support` | under `noise` `vote`: lists the counts shown by few cells in `thin`; see [Noisy examples](#noisy-examples) |

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
| `deviating_cells` | per write, at most 64 of those cells in example order, then z, y, x: `example`, `at`, `count`, `centre` (under `outer`), `seen` and `law` |
| `deviating_share` | deviating cells over voted cells (example cells times writes), as an exact `[num, den]` |

`min_support`, from 1 (if left out) up to the stencil's places times 64, marks the counts that rest on little evidence. The answer then adds `thin`: per write, in the shape of `ties`, each count shown by at least one and fewer than `min_support` example cells, as `{count, tally}` with `tally` its `[zeros, ones]`. Such a count is still decided by the vote, so the law does not change. `min_support` without `noise` `vote` is refused `support_noise` at `$.min_support`; 0 is refused `malformed` there, and past the stencil's limit (576 for 3x3) `support_limit`.

Where `refuse` answers a law, `vote` answers the same bytes with only these fields added, so `deviating` is 0. `refuse` answers exactly as a request without `noise`. The same request gives the same bytes.

Life from 16 32x32 torus pairs, 5% of the cells flipped in the input and the output: `vote` answers `[[[0,0,0,1,0,0,0,0,0],[0,0,1,1,0,0,0,0,0]]]`, and `deviating` names the cells to distrust. A crop of a larger world with `border` `zero` reads zeros past its edge, so its deviating cells lie on the crop's outer ring.
