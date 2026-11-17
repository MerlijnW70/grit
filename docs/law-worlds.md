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
| `law_not_built` | the device's driver failed to compile a kernel made from a law: a create with `law`, and an open, fork, walk back or rebuild of a law world; `device` and `vulkan` in the answer name the device and its result; no world is made and nothing else changes. A kernel made from no law (a rule world) is `kernel_not_built` at `$` | `$.law` |
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

`grit_law_gradient` answers, for a reversible law, how the objective after `frames` frames moves with each param. The objective is the sum over the `aim` box, on plane 0, of `|cell - target|`; `aim` is `from` and `to` (corners included) with `target` (the box's words, x fastest) or `world` (take them from that world's box). The start is a world (`world`: its cells on every plane at its frame, its params unless `params` is given) or an inline start (`law`, `size` with sides powers of two, `cells` plane by plane, `first`). `gradient[i]` is d objective / d `params[i]` times 2^64, a decimal string; `home` is true. It is the derivative through every `add`, `sub`, `mul`, `div`, shift and `sel` arm from the param to a cell; a compare, and so a `sel`'s choice, counts as constant. A law in which `not`, `and`, `or`, `xor`, `dot_s` or `dot_u` takes a value that moves with a param (also through a cell an earlier frame moved) and passes it on to a cell other than through a compare is refused `no_gradient`, with that node's path as `law.path`; so is a law that looks.

The answers equal the library's `Reversible::walk` and `Reversible::descend` exactly, on the CPU.

`grit_law_descend` takes the same and `steps`; it answers the best `params` and their `objective` (the earliest lowest on its `trail`), `steps` and `stopped`: `converged` (objective 0, or no step moves), `steps`, `budget` or `cancelled` (the best so far).

1. `grit_world_create` with `law` mix (`tests/law/mix.law.json`) and the hidden `params` `[3000,1700]`, `grit_world_write` the start, `grit_world_step` 8 frames: the target world.
2. `grit_world_create` the same law with a guess, `params` `[2100,2250]`, and `grit_world_write` the same start: the learner.
3. `grit_law_descend` with `world` the learner, `frames` 8, `aim` `{"from":[0,0,0],"to":[15,15,15],"world":<target>}`, `steps` 16: `params` `[3000,1700]`, `objective` 0, `stopped` `converged`, after 3 steps.
4. `grit_world_params` the learner with them and `grit_world_step` 8 frames: its cells equal the target's.

Neither tool writes a world or its journal. Refused before any walk: `not_reversible` (with the lifting refusal as `law`), `no_gradient` (above), `params`, `outside`, `reversed`, `target_size`, `aim`, `start`, `walk_limit` (a start past `walk_cells` words, a walk past `walk_work`), `budget`. A walk that does not come home answers `strayed` with the cell; one past 128 bits answers `overflow`. `grit_describe` names the limits under `law_gradient`.

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

`grit_world_read` and `grit_world_slice` take `plane` to read a law world's other planes.

The X0 edge task, a 24-gate chip that `grit_machine` learned from 3x3 neighbourhoods (input `k` is row `k / 3`, column `k % 3`, from the top left):

1. `grit_law_from_chip` with `inputs` 9, `outputs` 1, the chip's `gates`, `bind_inputs` `{"plane":0,"dx":k%3-1,"dy":k/3-1,"dz":0,"bit":0}` for `k` 0 to 8, `bind_outputs` `[{"plane":1,"bit":0}]`, `channels` 2: `halo` 1, `quiet` `proved`.
2. `grit_world_create` with `wide` 32, `deep` 32, `tall` 2 and `law` from step 1.
3. `grit_world_write` the image to plane 0, `[x, y, 0, pixel]`.
4. `grit_world_step` one frame.
5. `grit_world_read` from `[0,0,0]` to `[31,31,0]` with `plane` 1: each cell is the chip's answer on its 3x3 neighbourhood.

The world is a torus, so a 32-wide world reads row 0 next to row 31: on X0's held images 24 of 8192 cells then differ from the chip on X0's zero border. For a border that reads 0, make the world at least twice the image on each side and write the image at the origin: a 64 by 64 world gives X0's answers on all 8192 cells.
