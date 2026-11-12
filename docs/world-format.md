# World file, version 1

A saved world: `grit::branch::World::save` writes it, `World::load` reads it. Every number is a little-endian `u32` unless the table says otherwise. `P` is the number of parameters.

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
| 64 + 4P | 4 | channels | 1 |
| 68 + 4P | 4 | sparse | 0 dense, 1 sparse |
| 72 + 4P | 4 | capacity | sparse: pool slots, from the held bricks to all bricks; dense: 0 |
| 76 + 4P | 12 | brick sides | sparse: x, y, z, powers of two from 2 to the world's side; dense: 0, 0, 0 |
| 88 + 4P | | sections | |

## Sections

Each section is a 4-byte ASCII tag, a `u64` count of payload words, then the payload. They come in this order and no other:

| World | Sections |
|---|---|
| dense | `RULE`, `DENS`, `SUM ` |
| sparse | `RULE`, `TABL`, `POOL`, `SUM ` |

| Tag | Payload words | Payload |
|---|---|---|
| `RULE` | 1 + ⌈n/4⌉ | `n`, the text's byte count, then the text, zero-padded to a word: the canonical JSON `{"params":P,"rule":"<name>"}` (sorted keys, no spaces) |
| `DENS` | x·y·z | every cell, x fastest, then y, then z |
| `TABL` | 2 · bricks | per brick: slot and value. A uniform brick is `0xFFFFFFFF`, its word. A held brick is `k`, 0, where it is the `k`-th held brick, counted from 0 |
| `POOL` | held · cells per brick | the held bricks' cells, brick `k` at `k` · cells per brick |
| `SUM ` | 8 | 32 bytes: sha256 of every byte before the `SUM ` tag |

Bricks tile the world: brick `(i, j, l)` holds the cells from `(i·sx, j·sy, l·sz)` on. Bricks are numbered x fastest, then y, then z; so are the cells inside a brick.

## Reading

A reader refuses, each by name: a wrong magic (`bad_magic`), another version (`unknown_version`), a file that ends early (`truncated`), bytes after the checksum (`trailing`), a checksum that does not match (`bad_checksum`), a tag it does not know (`unknown_section`), a law hash that is not the sha256 of the law text (`law_hash`), section sizes or a capacity that disagree with the header (`sizes`), and anything else out of form (`malformed`). Opening the world refuses an unknown rule (`unknown_rule`), a wrong parameter count (`params`) and a world the device cannot hold (`unfit`).

Lists, awake marks and the tally are not stored. Opening a world seeds it: every held brick awake, every oct listed. A sparse world opens on any device: when the device's bricks differ from the file's, the cells are laid into the device's bricks.

## Laws

`RULE` names one of the built-in rules. A law in the law language is stored as a `LAW ` section in the same form, its text the law's canonical JSON, and the header's law hash covers that text. A version 1 reader that does not know `LAW ` refuses it as `unknown_section`.
