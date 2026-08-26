---
id: DR-0006
title: Mutation testing on the card attacks the wrong half, and the numbers to say so were already written down
status: prose-only
---

## The Measurement

`hyperia`'s `DR-0132` and `DR-0136` put a mutant at 7.88 s of build, 0.72 s for one focused test and
14.75 s for the whole suite, with the test-impact matrix answering 3 in 4 — a mean of 12.11 s of
which the build is 65%, and 400 mutants a file is 81 minutes against the 90 that record reports for
`src/card.rs`. Of the 78 files in that crate's `noha.yaml` surface, `src/lib.rs`, `src/source/mod.rs`
and `src/sys.rs` call `extern "system"`, and the arithmetic `hyperia`'s `DR-0010` moved down is
274 lines in `src/spread.rs` and `src/pattern.rs`.

## The Decision

Nothing was built.

## The Rejected Route

Executing mutants on the card was rejected at a ceiling of 1.54× — 12.11 s to 7.88 s if test
execution went to nought — against 2.86× for removing the build.

## The Limit

These figures are `hyperia`'s gate and say nothing about a codebase whose test execution is
arithmetic rather than process startup, and the SPIR-V mutator that `simdr`'s `src/decode.rs` makes
reachable is NOT MEASURED.
