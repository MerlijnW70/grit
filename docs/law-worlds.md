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
