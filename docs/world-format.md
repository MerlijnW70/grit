# World file, version 1

A saved world: `grit::branch::World::save` writes it, `World::load` reads it. Every number is a little-endian `u32` unless the table says otherwise. `P` is the number of parameters, `k` the number of planes (channels), `x`, `y`, `z` the world's size in cells.

## Header

| Offset | Bytes | Field | Value |
|---|---|---|---|
| 0 | 8 | magic | `GRITWLD1` |
| 8 | 4 | version | 1 |
| 12 | 32 | law hash | sha256 of the law section's text |
| 44 | 4 | params | `P` |
| 48 | 4·P | parameter words | in order |
| 48 + 4P | 4 | frame | frames stepped so far; the next frame is this one |
| 52 + 4P | 12 | size | x, y, z in cells, each a power of two, at least 2 |
| 64 + 4P | 4 | channels | `k`, from 1 to 4: 1 for a built-in rule, the law's channels for a law |
| 68 + 4P | 4 | sparse | 0 dense, 1 sparse |
| 72 + 4P | 4 | capacity | sparse: pool slots, at least 1 and at least the held bricks, at most all bricks; dense: 0 |
| 76 + 4P | 12 | brick sides | sparse: x, y, z, powers of two from 2 to the world's side; dense: 0, 0, 0 |
| 88 + 4P | | sections | |

## Sections

Each section is a 4-byte ASCII tag, a `u64` count of payload words, then the payload. They come in this order and no other:

| World | Sections |
|---|---|
| dense, built-in rule | `RULE`, `DENS`, `SUM ` |
| sparse, built-in rule | `RULE`, `TABL`, `POOL`, `SUM ` |
| dense, law | `LAW `, `DENS`, `SUM ` |
| sparse, law | `LAW `, `TABL`, `POOL`, `SUM ` |

| Tag | Payload words | Payload |
|---|---|---|
| `RULE` | 1 + ⌈n/4⌉ | `n`, the text's byte count, then the text, zero-padded to a word: the canonical JSON `{"params":P,"rule":"<name>"}` (sorted keys, no spaces) |
| `LAW ` | 1 + ⌈n/4⌉ | `n`, then the text, zero-padded to a word: the law's canonical JSON |
| `DENS` | k·x·y·z | plane by plane, each plane every cell, x fastest, then y, then z |
| `TABL` | (k+1)·bricks | first per brick its slot and its plane 0 value, then for each plane from 1 to k−1 one value per brick |
| `POOL` | k·held·c | plane by plane, each plane the held bricks' cells, held brick `h` at `h`·c |
| `SUM ` | 8 | 32 bytes: sha256 of every byte before the `SUM ` tag |

`c` is the cells per brick, `bricks` is x·y·z / c, `held` the held bricks.

A brick is uniform when each of its planes holds one word in all its cells: its slot is `0xFFFFFFFF`, and its value in each plane is that word. Otherwise it is held: its slot is `h` when it is the `h`-th held brick, counted from 0, and its value in every plane is 0. A held brick that is uniform in every plane is not canonical and is refused, so one world has one file.

Bricks tile the world: brick `(i, j, l)` holds the cells from `(i·sx, j·sy, l·sz)` on. Bricks are numbered x fastest, then y, then z; so are the cells inside a brick.

## Reading

A reader refuses, each by name: a wrong magic (`bad_magic`), another version (`unknown_version`), a file that ends early (`truncated`), bytes after the checksum (`trailing`), a checksum that does not match (`bad_checksum`), a tag it does not know (`unknown_section`), a law hash that is not the sha256 of the law text (`law_hash`), section sizes, channels or a capacity that disagree with the header (`sizes`), and anything else out of form (`malformed`). A world whose cells, words, table or pool would count past 64 bits is `malformed`. Opening the world refuses an unknown rule (`unknown_rule`), a wrong parameter count (`params`) and a world the device cannot hold (`unfit`).

Lists, awake marks and the tally are not stored. Opening a world seeds it: every held brick awake, every oct listed. A sparse world opens on any device: when the device's bricks differ from the file's, the cells are laid into the device's bricks.

## Laws

`RULE` names one of the built-in rules. `LAW ` holds a law in the law language as its canonical JSON; the header's law hash covers that text, and the header's channels are the law's.
