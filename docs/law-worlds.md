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

Refusals:

| Code | When | `field` |
|---|---|---|
| `rule_or_law` | both or neither of `rule` and `law` | `$` |
| `law_refused` | the law does not compile; `law` in the answer holds its own `code`, `path`, `expected`, `found` | `$.law` |
| `params` | a params count other than the law's | `$.params` |
| `quiet_refused` | a sparse world under a `whole` law | `$.sparse` |

`grit_world_params` gives a world new params between frames. The world steps every oct on its next frame, and a law world's class is decided again: a sparse world whose law turns `whole` is refused with `quiet_refused` at `$.params` and keeps its params, cells and state hash.

The law, its hash and its params come back after `grit_world_save` and `grit_world_open`, `grit_world_fork`, `grit_world_restore`, a restart of the server (the journal's `LAW` and `PARAMS` records) and a lost device. `grit_world_list` names each law world's `law` hash, and for an open world its `quiet`; `grit_describe` lists the open law worlds under `worlds.laws`.

## The quiet class

A listed world steps an oct only while it, or an oct around it, changed in the two frames before; a sparse world also keeps an all-one-value brick as a single word and steps it only while it is hot. That gives the cells of stepping every oct only when:

1. **the box contract**: an oct that a step leaves unchanged stays unchanged when stepped again with the same eight words. A law whose cell outputs read neither `frame` nor `at` (and so not `noise`, which reads both) is a function of the oct's words and the params alone, so it keeps it.
2. **fixed uniform values**: an oct whose eight words all hold a value a brick may be kept at steps to itself. Otherwise every such brick needs a slot when it is next stepped, and a pool smaller than the world overflows: the world stops and its reads answer `pool_full`.

| Class | Decided | Meaning | Stepping |
|---|---|---|---|
| `proved` | no cell output depends on `frame` or `at` in the law's node graph, and the background (0 on every plane) and every uniform brick value the world holds step to themselves under its params, by evaluating the law | both conditions hold | listed, dense or sparse |
| `whole` | anything else | | dense only, every oct every frame |

The class is decided at create from the background, and at every `grit_world_params` from the background and every uniform brick value in the sparse world's table. Opening a file decides it as a create does; a fork, a restore, a kept world and a rebuild keep the class last decided. A uniform brick value that is not a fixed point of a `proved` law can only be in a world's table when the world steps every brick on its next frame (after a create, an open or a params write), so such a brick takes a slot then and is never kept cold. The check reads the graph, not the text: a `let` the result never uses does not count, and an `["and","frame",0]` the result does use does.

Laws in `tests/law/`: `code` is `proved`; `sand` and `embed` read `frame`, and `heat`, `nature` and `evo` read `noise`, so they are `whole` and run dense.
