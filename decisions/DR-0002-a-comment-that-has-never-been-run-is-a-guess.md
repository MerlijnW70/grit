---
id: DR-0002
title: A comment that has never been run is a guess, and five of them here were wrong
status: prose-only
---

## The Measurement

Five claims in `src/` and `tests/` asserted a cost and carried no number: naming `DRAW_INDIRECT` in
every barrier cost 44 µs against 35; `tests/waiting.rs` in a binary of its own failed two runs in
three; `tests/wedged.rs`'s zero-timeout fence failed about one run in eight; command buffer recording
was 7 µs of a 35 µs frame; and "about a hundred microseconds" stood in thirteen places, counted at
`9eb4577`, for a submission `tests/floor.rs` puts at 35–42 µs. A sweep of `src/` for the same word
shapes then found one arithmetic error in `sand::PATIENCE` — seven bits stated where the word uses
eight — and two claims that overreached, in `crate::gate` and in `Resident`'s `INDIRECT_BUFFER` line.

## The Decision

Each of the eight now carries the number that produced it or states that it has none, and
`tests/floor.rs`, `tests/dispatch.rs` and `tests/hot.rs` were added so that four of them have a test
to come from.

## The Rejected Route

Measuring `crate::gate`'s vote and `live::tally`'s atomic was rejected because `tests/floor.rs`
resolves a whole dispatch at 224 ns and both sit under it.

## The Limit

These eight do not prove that no ninth exists: the sweep matched a fixed list of word shapes,
recorded in the commit that ran it, and a claim asserting a cost in words outside that list was not
searched for.
