"""English rule -> truth table -> grit circuit, with an ambiguity gate in between.

The division of labour is the point:

  code  enumerates every one of the 2^n rows, owns all thresholds, and decides
  Jev   judges ONE output bit per row - the only step that needs common sense
  grit  searches for the gates and marks them exactly on the host

Nothing about the circuit is ever inferred. grit's `wrong_rows` is exact and it
is the only claim of correctness this tool makes. What a model CAN get wrong is
the table, i.e. the intent, and a wrong table synthesises into a confidently
wrong circuit - so each row is also asked, separately, whether the rule decides
it at all, and a row the rule does not cover is refused before a single gate is
searched.

Exit codes: 0 solved, 1 searched and did not solve, 2 TypeSafe or key trouble,
3 refused because a row was undecided.

Usage:
    python front/spec.py "at least two of the three inputs are 1" -i a,b,c
    python front/spec.py "carry out of a full adder" -i x,y,cin --least
    python front/spec.py "sum and carry of two bits" -i a,b -o sum,carry --json
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass, field

try:
    from typesafe_sdk import Noul, TypeSafeAPIError, TypeSafeClient
except ImportError:  # the message is the whole point
    sys.exit("typesafe-sdk is not installed: pip install typesafe-sdk")

DEFAULT_MCP = r"F:\grit\target\release\grit-mcp.exe"

# Ambiguity is asked about directly rather than read off the output probability.
# Distance from 0.5 does not separate the two cases: a confident no and a row
# nobody can decide land in the same place, so any margin threshold both misses
# real ambiguity and fires on rows that were never in doubt. The second question
# below asks what we actually mean, and its answer is the gate.
#
# There is no clean cut here. Clear specs and vague ones interleave, because a
# spec naming a known component states no condition either, so every floor in a
# wide band gets the same number wrong and only moves which ones. This sits
# where a vague spec getting through is treated as worse than an interruption.
# `bench.py --suite spec` prints what every floor would do; set it from your own
# specs with --cases, because this one was set on specs invented while building.
UNDECIDED_FLOOR = 0.45

# Kept only as a secondary display signal, never as the gate.
NARROW_MARGIN = 0.15

# How near the middle the closest answer in a table may sit.
#
# This is a margin, and margins standing in for other questions have lost every
# comparison in this front door - a whole-rule reading beat a row-by-row one, a
# direct depth question beat a table margin. This one is not standing in for
# anything. A bit answered at fifty one hundredths is a bit that nearly came
# back the other way, and that is the whole of what is being said.
#
# Over nine specs asked whole it separates completely: every exact table has
# its closest call at twenty six hundredths from the middle or further, every
# table with a wrong row has one at three hundredths or nearer.
#
# It stood at six for a while, because a staged spec then answered at twelve
# hundredths and was right, and fifteen would have refused a spec that works.
# Asking each output on its own moved those answers out to between thirty two
# and forty four, and took the reason for six away with them.
#
# Where it sits now, everything measured separates. Exact tables, staged or
# whole, have their closest call at twenty six hundredths or further. Wrong
# tables asked whole have one at three or nearer, and the one staged run in
# four that comes back wrong has one at nine - which six let through and this
# does not.
#
# The caveat that stood here - that a staged spec sits inside the gap and is
# right anyway - was true of a staged spec that asked all its outputs at once.
# Asking each one on its own moved the adder from twelve hundredths to thirty
# three, and the exception went away rather than being worked around.
THIN_TABLE = 0.15

# An output that reads three of six inputs needs eight rows asked, not sixty
# four. As in machine.py the two errors differ in size: keeping a spare input
# costs questions, dropping a real one flattens the spec where grit cannot see
# it, since the projected table is consistent by construction and marks clean.
# So the floor is low and every drop is checked afterwards.
PRUNE_FLOOR = 0.25


# Above this, the rule wants arithmetic on a number spelled out in the inputs,
# which is where the answers stop being reliable: measured on specs with known
# tables, the ones that decode bits into a number and test a property of it got
# rows wrong while conditions stated over the bits came back exact. Nothing
# else catches this - the undecided question asks whether the rule decides the
# case, and these rules decide it perfectly well. The model just answers wrong.
#
# It is a partial guard, and `bench.py --suite hard` shows where it stops. It
# catches "these bits are a number, is it prime" and "is it nine or less". It
# does not catch a two-bit adder, which scored low and got six of forty eight
# bits wrong - and answering low there is defensible, because an adder really
# is a per-bit circuit rather than number-level reasoning. Whatever went wrong
# in the adder is not the thing this question asks about, so read the table.
ARITHMETIC_CEILING = 0.50

# Above this, deciding an output means finding something in between first: a
# carry, a count, a value that is neither an input nor the answer. It is the
# failure the arithmetic question rightly misses - a two-bit adder is a circuit
# over bits and not number-level reasoning, and arithmetic reads it at fifteen
# hundredths, while every wrong bit in it is in an output needing the carry.
#
# On its own this does not separate. Over the five specs with known tables the
# exact ones read forty eight and fifty five while the wrong ones read forty
# eight, fifty six and eighty six: an exact spec sits above a wrong one. It is
# set where it catches the adder and alarms on neither exact spec, which leaves
# two wrong specs below it - and those are the two the arithmetic question
# catches at sixty nine and eighty two. The pair covers all five; neither does
# alone, and fifty five against fifty six is not a margin to lean on.
THROUGH_CEILING = 0.60


def read_shape(client: TypeSafeClient, rule: str, inputs: list[str],
               outputs: list[str], model: str | None = None,
               worked: list[str] | None = None) -> dict:
    """Does this rule want the inputs read as a number and computed with?

    `worked` names values that have already been worked out and handed over. The
    in-between question has to know about them or it refuses the very thing it
    asked for: a parity spec staged on its own exclusive-or was told that
    deciding its output needs something found in between, while that something
    was sitting in the inputs under the name the caller gave it.
    """
    worked = worked or []
    given = (f" The values {', '.join(worked)} have already been worked out and "
             "are given to you as inputs." if worked else "")
    answers = client.system_one(
        state={"rule": rule, "inputs": inputs, "outputs": outputs},
        model=model,
        questions={
            "arithmetic": Noul(instructions=(
                "Applying the rule in `rule` means reading the inputs together as the "
                "digits of a number, and then doing arithmetic with that number - "
                "adding it, comparing its size, or testing a property of it such as "
                "being prime or being in a range. It is not enough to look at the "
                "individual inputs and combine them with and, or and not."
            )),
            "counting": Noul(instructions=(
                "Applying the rule in `rule` means counting how many inputs are 1 and "
                "comparing that count against a number."
            )),
            # A different failure from arithmetic, and the one arithmetic misses.
            # A two-bit adder is not number-level reasoning - it is a circuit over
            # bits, and the arithmetic question rightly said so at eighteen
            # hundredths - yet every wrong bit in it was in the one output that
            # needs the carry worked out first. That is a hop, not a sum.
            **{f"through_{out}": Noul(instructions=(
                f"Deciding the output bit `{out}` from the rule in `rule` means first "
                "working out something that is not one of the inputs and is not the "
                "answer either - a carry, a running count, an intermediate value that "
                "has to be found before the answer can be. Reading the inputs and "
                "combining them directly is not enough." + given
            )) for out in outputs},
        })
    return {"arithmetic": answers.nouls["arithmetic"].noul,
            "counting": answers.nouls["counting"].noul,
            "through": {out: answers.nouls[f"through_{out}"].noul for out in outputs}}


def _undecided_question(out: str, seen: str) -> Noul:
    """Does the rule settle this one case. Asked the same way wherever it is asked."""
    return Noul(instructions=(
        "Read the rule in `rule` under the usual convention: it names the cases "
        f"where `{out}` is 1, and `{out}` is 0 in every case it does not name. "
        f"Even under that convention, the rule fails to settle `{out}` when "
        f"{seen}: the rule's own words leave this case genuinely open, so two "
        "careful readers could apply it here and get different answers. This is "
        "about the rule being incomplete or vague, not about the case being rare "
        f"or about `{out}` simply being 0 here."
    ))


def _whole_question() -> Noul:
    """Is the rule vague, asked once of the rule rather than a row at a time.

    Measured over fourteen specs this is right about all fourteen where walking
    the rows is right about twelve, and the two readings agree on twelve with
    every wrong answer living in the two they part on. So this decides.
    What it cannot do is say which case is the vague one, and that is the whole
    use of the answer - so the rows still carry it, and name the ones nearest
    the middle when this stops a spec.
    """
    return Noul(instructions=(
                "Read the rule in `rule` under the usual convention: it names the "
                "cases where an output is 1, and the output is 0 in every case it "
                "does not name. Even so, there is at least one combination of the "
                "inputs where the rule's own words do not settle what the output "
                "should be - two careful readers could apply it there and get "
                "different answers. This is about the rule being incomplete or "
                "vague, not about it being unusual or hard to build."
    ))


def read_relevance(client: TypeSafeClient, rule: str, inputs: list[str],
                   outputs: list[str], model: str | None = None,
                   known: list[str] | None = None) -> dict[tuple[str, str], float]:
    """Which inputs each output actually reads.

    With in-between values worked out, the question has to say so. A parity spec
    staged on p, where p is a exclusive-or b, was keeping a and b for an output
    that reads only p and c - and rightly, because a does change the answer,
    through p. Once p is known it does not, and that is the question worth
    asking. The check that puts values back cannot fix this: it only ever adds.
    """
    worked = [name for name in (known or []) if name not in inputs]
    questions = {}
    for name in inputs:
        for out in outputs:
            questions[f"rel_{name}_{out}"] = Noul(instructions=(
                (f"The values {', '.join(worked)} have already been worked out and are "
                 f"known. Given them, under the rule in `rule`, " if worked
                 else f"Under the rule in `rule`, ")
                + f"the value of input `{name}` can change what "
                f"output `{out}` should be. That is: there are two cases differing only "
                f"in `{name}`, with every other value including "
                + (", ".join(worked) if worked else "every other input")
                + f" the same, where the rule calls for a different `{out}`."
            ))
    answers = client.system_one(state={"rule": rule, "inputs": inputs,
                                       "outputs": outputs},
                                questions=questions, model=model)
    return {(name, out): answers.nouls[f"rel_{name}_{out}"].noul
            for name in inputs for out in outputs}


def relevant_inputs(relevance: dict[tuple[str, str], float], out: str,
                    inputs: list[str], floor: float = PRUNE_FLOOR) -> list[str]:
    """The inputs an output is asked about. Never empty: a constant still needs a row."""
    kept = [name for name in inputs if relevance[(name, out)] > floor]
    return kept or [inputs[0]]


def check_pruning(client: TypeSafeClient, rule: str, inputs: list[str],
                  outputs: list[str], relevance: dict[tuple[str, str], float],
                  model: str | None = None,
                  reachable: list[dict[str, int]] | None = None
                  ) -> list[tuple[str, str]]:
    """Put each dropped input to the test: does flipping it ever change the output?

    The pruned enumeration never asks a pair differing only in a dropped input,
    so the claim that it does not matter goes unchecked. Every background of the
    other inputs is tried, which is exhaustive and still far cheaper than the
    full table whenever the pruning saved anything at all.
    """
    questions, pairs = {}, {}
    for out in outputs:
        kept = relevant_inputs(relevance, out, inputs)
        for name in inputs:
            if name in kept:
                continue
            rest = [other for other in inputs if other != name]
            # Every background, unless some of these values are not free of each
            # other. An in-between value is not: a carry is whatever the bits
            # under it make it, so flipping one of those while holding the carry
            # still asks about a case that cannot happen - and the answer came
            # back putting both bits back as decisive, which is true of the rule
            # and false of the question. When the reachable cases are known, only
            # pairs that both occur are put.
            grounds = ([one for one in reachable] if reachable is not None
                       else [{other: (r >> i) & 1 for i, other in enumerate(rest)}
                             for r in range(2 ** len(rest))])
            for r, background in enumerate(grounds):
                if reachable is not None and not any(
                        all(one[other] == background[other] for other in rest)
                        and one[name] != background[name] for one in reachable):
                    continue
                for value in (0, 1):
                    key = f"chk_{name}_{out}_{r}_{value}"
                    pairs[key] = (name, out, r, value)
                    seen = ", ".join(f"{one}={value if one == name else background[one]}"
                                     for one in inputs)
                    questions[key] = Noul(instructions=(
                        f"For the rule in `rule`, with {seen}, the output bit `{out}` is 1."
                    ))
    if not questions:
        return []
    answers = client.system_one(state={"rule": rule}, questions=questions, model=model)
    said = {key: answers.nouls[key].noul > 0.5 for key in questions}
    return sorted({(name, out) for key, (name, out, r, value) in pairs.items()
                   if value == 0 and said[key] != said[f"chk_{name}_{out}_{r}_1"]})


@dataclass
class Row:
    index: int
    bits: dict[str, int]
    outs: dict[str, int] = field(default_factory=dict)
    probs: dict[str, float] = field(default_factory=dict)
    undecided: dict[str, float] = field(default_factory=dict)

    def word(self, outputs: list[str]) -> int:
        return sum(self.outs[name] << j for j, name in enumerate(outputs))

    def margin(self) -> float:
        return min(abs(p - 0.5) for p in self.probs.values())

    def unclear(self, floor: float) -> list[str]:
        """Bits the rule itself does not decide. Not the same as a close call."""
        return [name for name, p in self.undecided.items() if p > floor]


def read_intent(client: TypeSafeClient, rule: str, inputs: list[str],
                outputs: list[str], model: str | None = None,
                relevance: dict[tuple[str, str], float] | None = None,
                ) -> tuple[list[Row], float]:
    """One request. Every (case, output bit) pair is an independent judgement.

    The rows are always all 2^n of them - they are the deliverable, and code
    enumerates them so none can go missing. What a relevance map changes is
    which *cases* get asked about: an output reading three of six inputs is
    eight questions, and the rows it does not distinguish are filled by
    projecting onto the inputs it reads.
    """
    rows = [
        Row(index=r, bits={n: (r >> i) & 1 for i, n in enumerate(inputs)})
        for r in range(2 ** len(inputs))
    ]
    if relevance is not None:
        return _read_pruned(client, rule, inputs, outputs, rows, relevance, model)
    state = {
        "rule": rule,
        "inputs": inputs,
        "outputs": outputs,
        "bit_order": "inputs[0] is bit 0, the lowest bit of the row index",
    }
    questions = {"whole": _whole_question()}
    for row in rows:
        seen = ", ".join(f"{n}={v}" for n, v in row.bits.items())
        for out in outputs:
            # What the bit is.
            questions[f"r{row.index}_{out}"] = Noul(instructions=(
                f"For the rule in `rule`, with {seen}, the output bit `{out}` is 1."
            ))
            # Whether the rule decides it at all. This is a different question
            # from the one above being a close call, and it is the one that
            # decides whether a human is asked.
            #
            # The convention has to be spelled out. Without it this fired on
            # every row whose output was 0 and no row whose output was 1:
            # a rule saying "output 1 when at least two inputs are 1" does not,
            # read literally, say anything about the rows where it is 0, and
            # the model answers the question as written.
            questions[f"u{row.index}_{out}"] = _undecided_question(out, seen)
    answers = client.system_one(state=state, questions=questions, model=model)
    for row in rows:
        for out in outputs:
            p = answers.nouls[f"r{row.index}_{out}"].noul
            row.probs[out] = p
            row.outs[out] = 1 if p > 0.5 else 0
            row.undecided[out] = answers.nouls[f"u{row.index}_{out}"].noul
    return rows, answers.nouls["whole"].noul


def _read_pruned(client, rule, inputs, outputs, rows, relevance, model):
    """Ask once per case an output can tell apart, then project onto every row."""
    kept = {out: relevant_inputs(relevance, out, inputs) for out in outputs}
    questions = {}
    for out in outputs:
        for r in range(2 ** len(kept[out])):
            case = {name: (r >> i) & 1 for i, name in enumerate(kept[out])}
            seen = ", ".join(f"{name}={bit}" for name, bit in case.items())
            # Only say this when something was actually left out. Said while
            # every input is named it is both false and harmful: measured on a
            # two-bit adder, the two outputs that read all four inputs - where
            # the cases asked are identical to the unpruned ones - went from one
            # and five bits wrong to six and eight. The output that really does
            # read two of four was perfect either way.
            held_back = [name for name in inputs if name not in kept[out]]
            questions[f"v{out}_{r}"] = Noul(instructions=(
                f"For the rule in `rule`, with {seen}, the output bit `{out}` is 1."
                + (f" The other inputs, {', '.join(held_back)}, do not matter to "
                   f"`{out}`." if held_back else "")
            ))
            questions[f"u{out}_{r}"] = Noul(instructions=(
                "Read the rule in `rule` under the usual convention: it names the cases "
                f"where `{out}` is 1, and `{out}` is 0 in every case it does not name. "
                f"Even under that convention, the rule fails to settle `{out}` when "
                f"{seen}: the rule's own words leave this case genuinely open, so two "
                "careful readers could apply it here and get different answers."
            ))
    questions["whole"] = _whole_question()
    answers = client.system_one(state={"rule": rule, "inputs": inputs,
                                       "outputs": outputs},
                                questions=questions, model=model)
    for row in rows:
        for out in outputs:
            r = sum(row.bits[name] << i for i, name in enumerate(kept[out]))
            p = answers.nouls[f"v{out}_{r}"].noul
            row.probs[out] = p
            row.outs[out] = 1 if p > 0.5 else 0
            row.undecided[out] = answers.nouls[f"u{out}_{r}"].noul
    return rows, answers.nouls["whole"].noul


def loosened(kept: list[str], reachable: list[dict[str, int]]) -> list[str]:
    """Drop values that cannot tell two reachable cases apart, by arithmetic.

    A value worked out from the inputs is tied to them: with p the exclusive-or
    of a and b, no two cases differ in a alone, so asking whether a can change
    the answer with p held still asks about a pair that does not exist. The
    model reads it as true - a does change the answer, through p - and it is
    right about the rule and answering a different question. No wording fixes
    an unsatisfiable premise.

    So it is not asked. A value can go when what is left still tells every
    reachable case apart, which is decided here and is exact. It is careful
    rather than clever: it never collapses two cases on the chance that they
    agree, so it drops a or b and not both.
    """
    held = list(kept)
    for name in list(kept):
        rest = [one for one in held if one != name]
        seen = {tuple(case[one] for one in rest) for case in reachable}
        if len(seen) == len(reachable):
            held = rest
    return held


def reachable_cases(client: TypeSafeClient, inputs: list[str],
                    vias: list[tuple[str, str]], model: str | None = None
                    ) -> list[dict[str, int]]:
    """Every case that can actually happen once the in-between values are worked out.

    There are 2^inputs of them and no more, however many in-between values are
    named, because each is a function of what came before it.
    """
    known = list(inputs)
    carried = [{name: (r >> i) & 1 for i, name in enumerate(inputs)}
               for r in range(2 ** len(inputs))]
    for name, told in vias:
        stage_rows, _ = read_intent(client, told, known, [name], model)
        table = {tuple(row.bits[one] for one in known): row.outs[name]
                 for row in stage_rows}
        for row in carried:
            row[name] = table[tuple(row[one] for one in known)]
        known = known + [name]
    return carried


def staged(client: TypeSafeClient, rule: str, inputs: list[str], outputs: list[str],
           vias: list[tuple[str, str]], model: str | None = None,
           relevance: dict[tuple[str, str], float] | None = None
           ) -> tuple[list[Row], float, list[dict]]:
    """Work out the named in-between values first, then the outputs given them.

    The guard refuses a spec whose outputs need something found in between - a
    carry, a count - because that is where the wrong rows are. Naming that value
    is a judgement about the rule and belongs to whoever wrote it; working it out
    and carrying it through is arithmetic and belongs here.

    Measured on a two-bit adder, which the guard refuses and which came back six
    of forty eight bits wrong when asked whole: the carry on its own is exact,
    the middle sum bit given the carry is exact, and the carry out given the
    carry is exact. Every stage passes both guards with room to spare.

    The staging is what does that, not the asking-one-output-at-a-time this does
    alongside it. The adder asked whole, one request an output, is five of forty
    eight bits wrong with its closest call a tenth from the middle - the same as
    asking all three together. Narrowing what is in view makes an answerable
    question easier; it does not make "s1 is the next bit of the sum" into one.

    Each stage is a spec in its own right, asked over the original inputs and
    any in-between values named before it. The last stage is asked once per row
    of the real table, with the in-between values spelled out as facts about
    that row, so the table that comes out is over the inputs the caller named
    and nothing else. grit still marks the circuit against it.
    """
    known: list[str] = list(inputs)
    worked: list[dict] = []
    # Every row of the real table, carrying whatever has been worked out so far.
    carried: list[dict[str, int]] = [
        {name: (r >> i) & 1 for i, name in enumerate(inputs)}
        for r in range(2 ** len(inputs))
    ]

    for name, told in vias:
        # A stage is a spec, so it is guarded like one. Without this the whole
        # arithmetic simply moves into the stage: asked to work out "big is 1
        # when the number is ten or more" the stage comes back wrong on two rows
        # while the step that reads it - out is 1 when big is 0 - is easy and
        # sure, so the finished table is wrong and its closest call is forty four
        # hundredths from the middle. Nothing downstream sees that either.
        stage_shape = read_shape(client, told, known, [name], model)
        stage_rows, stage_whole = read_intent(client, told, known, [name], model)
        stage_thin = min((abs(p - 0.5) for row in stage_rows
                          for p in row.probs.values()), default=0.5)
        table = {tuple(row.bits[one] for one in known): row.outs[name]
                 for row in stage_rows}
        for row in carried:
            row[name] = table[tuple(row[one] for one in known)]
        worked.append({"via": name, "rule": told, "over": list(known),
                       "whole": round(stage_whole, 3),
                       "arithmetic": round(stage_shape["arithmetic"], 3),
                       "through": round(max(stage_shape["through"].values()), 3),
                       "thinnest": round(stage_thin, 3),
                       "table": [stage_rows[r].outs[name] for r in range(len(stage_rows))]})
        known = known + [name]

    # The last stage asks once per distinct case an output can tell apart, not
    # once per row. With an in-between value worked out, several rows present
    # the same case to an output - the carry-out of an adder sees the same three
    # values from four different rows - and asking each of them separately is
    # asking one question four times and letting the answers differ. Measured,
    # that is where the wrong rows came from: the same stage asked once per case
    # in isolation was exact, and asked once per row was one to three rows out.
    rows = [Row(index=r, bits={name: carried[r][name] for name in inputs})
            for r in range(2 ** len(inputs))]
    reads = {out: (relevant_inputs(relevance, out, known) if relevance is not None
                   else known) for out in outputs}
    # Anything the arithmetic can drop for free goes before anything is asked.
    reads = {out: loosened(one, carried) for out, one in reads.items()}
    cases: dict[str, dict[tuple, int]] = {out: {} for out in outputs}
    questions = {"whole": _whole_question()}
    for row in rows:
        held = carried[row.index]
        for out in outputs:
            key = tuple(held[name] for name in reads[out])
            if key in cases[out]:
                continue
            cases[out][key] = len(cases[out])
    # One request an output, naming only that output. The cases it asks are the
    # same either way; what changes is what else is in view. Measured on the
    # middle sum bit of an adder, the same eight cases asked with all three
    # outputs named close to twenty five hundredths of the middle and asked with
    # only their own named close to thirty seven. Thin answers are what a rerun
    # crosses, so the room is the point.
    asked_per: dict[str, dict] = {}
    for out in outputs:
        held_back = [name for name in known if name not in reads[out]]
        for key, n in cases[out].items():
            here = ", ".join(f"{name}={bit}" for name, bit in zip(reads[out], key))
            asked_per.setdefault(out, {})[f"v{out}_{n}"] = Noul(instructions=(
                f"For the rule in `rule`, with {here}, the output bit `{out}` is 1."
                + (f" The other values, {', '.join(held_back)}, do not matter to "
                   f"`{out}`." if held_back else "")
            ))
            asked_per[out][f"u{out}_{n}"] = _undecided_question(out, here)
    held_answers = {}
    whole = 0.0
    for out in outputs:
        asked = dict(asked_per.get(out, {}))
        if not held_answers:
            asked["whole"] = questions["whole"]
        answers = client.system_one(
            state={"rule": rule, "inputs": reads[out], "outputs": [out],
                   "worked_out_first": [one["via"] for one in worked]},
            questions=asked, model=model)
        if "whole" in asked:
            whole = answers.nouls["whole"].noul
        held_answers.update({key: answers.nouls[key].noul for key in asked_per.get(out, {})})
    for row in rows:
        held = carried[row.index]
        for out in outputs:
            n = cases[out][tuple(held[name] for name in reads[out])]
            p = held_answers[f"v{out}_{n}"]
            row.probs[out] = p
            row.outs[out] = 1 if p > 0.5 else 0
            row.undecided[out] = held_answers[f"u{out}_{n}"]
    # What each output was actually asked over, after everything that could be
    # dropped was. Handed back so a caller reports what happened rather than
    # working it out again a slightly different way.
    for one in worked:
        one["reads"] = {out: list(reads[out]) for out in outputs}
    return rows, whole, worked

def grit(tool: str, arguments: dict, binary: str | list[str], timeout: int = 900) -> dict:
    """Speak MCP to grit over stdio. Its answer is the authority on circuits."""
    conversation = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
         "params": {"name": tool, "arguments": arguments}},
    ]
    payload = "".join(json.dumps(message) + "\n" for message in conversation)
    argv = [binary] if isinstance(binary, str) else list(binary)
    try:
        done = subprocess.run(argv, input=payload, capture_output=True,
                              text=True, timeout=timeout)
    except FileNotFoundError:
        raise SystemExit(f"grit-mcp not found at {binary}; build it with "
                         "`cargo build --release --bin grit-mcp`, or set GRIT_MCP")
    except subprocess.TimeoutExpired:
        raise SystemExit(f"{tool} did not answer within {timeout}s")
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("id") != 2:
            continue
        if "error" in message:
            raise SystemExit(f"grit refused {tool}: {message['error']}")
        result = message.get("result", {})
        if result.get("isError"):
            said = (result.get("content") or [{}])[0].get("text", "")
            raise SystemExit(f"grit could not do it: {said}")
        return result.get("structuredContent", {})
    raise SystemExit(f"no answer from grit-mcp: {done.stderr[:400] or 'no stderr'}")


def render(rows: list[Row], inputs: list[str], outputs: list[str]) -> str:
    head = "  ".join(inputs) + "  |  " + "  ".join(outputs)
    lines = [head, "-" * len(head)]
    for row in rows:
        left = "  ".join(str(row.bits[n]).rjust(len(n)) for n in inputs)
        right = "  ".join(str(row.outs[n]).rjust(len(n)) for n in outputs)
        note = f"P={row.margin() + 0.5:.2f}"
        worst = max(row.undecided.values()) if row.undecided else 0.0
        if worst > UNDECIDED_FLOOR:
            note += f"  UNDECIDED {worst:.2f}"
        elif row.margin() < NARROW_MARGIN:
            note += "  close"
        lines.append(f"{left}  |  {right}   {note}")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="spec", description="Turn an English rule into a verified grit circuit.")
    parser.add_argument("rule", help="the rule, in plain English")
    parser.add_argument("-i", "--inputs", default="a,b,c",
                        help="comma-separated input names, lowest bit first")
    parser.add_argument("-o", "--outputs", default="out",
                        help="comma-separated output names, lowest bit first")
    parser.add_argument("-g", "--gates", type=int, default=8, help="gate budget")
    parser.add_argument("--least", action="store_true",
                        help="walk the budget down and report the smallest that solved")
    parser.add_argument("--margin", type=float, default=UNDECIDED_FLOOR,
                        help=f"flag a row when P(undecided) exceeds this (default {UNDECIDED_FLOOR})")
    parser.add_argument("--force", action="store_true",
                        help="search even where intent was unclear (it becomes a gate)")
    parser.add_argument("--via", action="append", default=[], metavar="NAME=RULE",
                        help="an in-between value to work out first, as name=rule; "
                             "repeatable, and each may use the ones before it. The "
                             "guard refuses specs that need one of these, and naming "
                             "it is the judgement it cannot make for you")
    parser.add_argument("--prune", action="store_true",
                        help="ask which inputs each output reads and enumerate only "
                             "those cases; the table is still every row")
    parser.add_argument("--table", help="skip the model and use this comma-separated table")
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    parser.add_argument("--model", help="TypeSafe model (default jev-latest)")
    parser.add_argument("--mcp", default=os.environ.get("GRIT_MCP", DEFAULT_MCP),
                        help="path to grit-mcp")
    args = parser.parse_args(argv)

    inputs = [n.strip() for n in args.inputs.split(",") if n.strip()]
    outputs = [n.strip() for n in args.outputs.split(",") if n.strip()]
    if not inputs or not outputs:
        parser.error("need at least one input and one output")

    vias: list[tuple[str, str]] = []
    for one in args.via:
        name, sep, told = one.partition("=")
        if not sep or not name.strip() or not told.strip():
            parser.error(f"--via wants name=rule, not {one!r}")
        if name.strip() in inputs or name.strip() in outputs:
            parser.error(f"{name.strip()} is already an input or an output")
        vias.append((name.strip(), told.strip()))

    report: dict = {"rule": args.rule, "inputs": inputs, "outputs": outputs}

    if args.table:
        table = [int(value) for value in args.table.split(",")]
        if len(table) != 2 ** len(inputs):
            parser.error(f"table needs {2 ** len(inputs)} rows, got {len(table)}")
        report["table_from"] = "caller"
    else:
        if not os.environ.get("TYPESAFE_API_KEY"):
            print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
            return 2
        try:
            with TypeSafeClient() as client:
                named = inputs + [name for name, _ in vias]
                shape = read_shape(client, args.rule, named, outputs, args.model,
                                   [name for name, _ in vias])
                report["shape"] = {
                    k: ({one: round(held, 3) for one, held in v.items()}
                        if isinstance(v, dict) else round(v, 3))
                    for k, v in shape.items()}
                # Not asked once in-between values have been named. The question
                # is "you need one of these", and the answer is "I gave you one" -
                # a spec staged on its own exclusive-or was refused for needing
                # the very value sitting in its inputs under the name the caller
                # chose. What the naming does not settle is whether each stage is
                # sound, and whether enough of them were named; the first is now
                # guarded stage by stage and the second shows up as a thin table.
                through = [out for out, held in shape["through"].items()
                           if held > THROUGH_CEILING] if not vias else []
                if through and shape["arithmetic"] <= ARITHMETIC_CEILING                         and not args.force:
                    if args.json:
                        print(json.dumps({**report, "searched": False}, indent=2))
                    else:
                        print("Deciding " + ", ".join(f"`{one}`" for one in through)
                              + " means working out something in between first - a "
                              "carry, a count, a value that is neither an input nor "
                              "the answer. Measured on specs with known tables, that "
                              "is where the wrong rows are, and no later check here "
                              "would catch it. Say the rule in terms of the inputs "
                              "themselves, name the in-between value as an input of "
                              "its own, or pass --table. --force searches anyway.")
                    return 3
                if shape["arithmetic"] > ARITHMETIC_CEILING and not args.force:
                    if args.json:
                        print(json.dumps({**report, "searched": False}, indent=2))
                    else:
                        print("This rule reads the inputs as a number and computes "
                              f"with it (P={shape['arithmetic']:.2f}). Measured on "
                              "specs with known tables, that is where the answers stop "
                              "being right, and nothing later here would catch it: grit "
                              "marks the circuit against whatever table it is handed." + 
                              "\n\n"
                              "State the condition over the inputs themselves if you "
                              "can, or pass --table to give the table yourself. "
                              "--force searches anyway.")
                    return 3
                # With stages, relevance is read over the in-between values too,
                # since an output may read one of those and not the inputs under it.
                relevance = (read_relevance(client, args.rule, named, outputs,
                                            args.model, named if vias else None)
                             if args.prune else None)
                if relevance is not None:
                    # With stages the named values are not free of each other,
                    # so the check is given the cases that can actually happen.
                    within = None
                    if vias:
                        within = reachable_cases(client, inputs, vias, args.model)
                    for name, out in check_pruning(client, args.rule, named, outputs,
                                                   relevance, args.model, within):
                        relevance[(name, out)] = 1.0
                        if not args.json:
                            print(f"put back: `{name}` decides `{out}` after all")
                    if not args.json:
                        asked = sum(2 ** len(relevant_inputs(relevance, out, named))
                                    for out in outputs)
                        print("reads: " + "; ".join(
                            f"{out} <- " + ", ".join(relevant_inputs(relevance, out, named))
                            for out in outputs)
                            + f"  ({asked} cases instead of "
                            + f"{2 ** len(inputs) * len(outputs)})")
                if vias:
                    rows, whole, worked = staged(client, args.rule, inputs, outputs,
                                                 vias, args.model, relevance)
                    report["worked_out_first"] = worked
                    amiss = [one for one in worked
                             if one["arithmetic"] > ARITHMETIC_CEILING
                             or one["through"] > THROUGH_CEILING
                             or one["thinnest"] < THIN_TABLE
                             or one["whole"] > 0.5]
                    if amiss and not args.force:
                        if args.json:
                            print(json.dumps({**report, "searched": False}, indent=2))
                        else:
                            for one in amiss:
                                print(f"\nWorking out `{one['via']}` is itself a spec "
                                      "this would refuse: arithmetic "
                                      f"{one['arithmetic']:.2f}, in-between "
                                      f"{one['through']:.2f}, leaves something open "
                                      f"{one['whole']:.2f}, closest call "
                                      f"{one['thinnest']:.2f} from the middle.")
                            print("A stage that is wrong makes the whole table wrong, "
                                  "and the step that reads it can be easy and sure "
                                  "while it is - so neither the closest call in the "
                                  "finished table nor anything after it would catch "
                                  "this. Say the stage over the inputs themselves, or "
                                  "work it out in stages of its own.")
                        return 3
                    if not args.json:
                        for one in worked:
                            print(f"worked out `{one['via']}` over "
                                  + ", ".join(one["over"])
                                  + f": {one['table']}")
                else:
                    rows, whole = read_intent(client, args.rule, inputs, outputs,
                                              args.model, relevance)
        except TypeSafeAPIError as error:
            print(f"TypeSafe error {error.status} (request {error.request_id})",
                  file=sys.stderr)
            return 2
        table = [row.word(outputs) for row in rows]
        report["table_from"] = "jev"

        # The whole-rule reading decides; the rows say where. They are right
        # together twelve times in fourteen and every wrong answer measured sat
        # where they parted, so a parting is worth printing rather than hiding.
        thinnest = min((abs(p - 0.5) for row in rows for p in row.probs.values()),
                       default=0.5)
        report["thinnest"] = round(thinnest, 3)
        stopping = whole > 0.5
        unclear = [row for row in rows if row.unclear(args.margin)]
        report["whole"] = round(whole, 3)
        report["readings_agree"] = stopping == bool(unclear)
        nearest = sorted(rows, key=lambda row: -max(row.undecided.values()))[:3]
        report["unclear"] = [
            {"row": row.index, "bits": row.bits, "bit": name,
             "p": round(row.probs[name], 3),
             "p_undecided": round(row.undecided[name], 3)}
            for row in unclear for name in row.unclear(args.margin)
        ]
        if not args.json:
            print(render(rows, inputs, outputs))
        if thinnest < THIN_TABLE and not args.force:
            if args.json:
                print(json.dumps({**report, "searched": False}, indent=2))
            else:
                near = min(((abs(p - 0.5), row, name)
                            for row in rows for name, p in row.probs.items()),
                           key=lambda one: one[0])
                seen = ", ".join(f"{n}={v}" for n, v in near[1].bits.items())
                print(f"\nThe closest answer in this table sits {thinnest:.2f} from "
                      f"the middle: with {seen}, `{near[2]}` came back at "
                      f"{near[1].probs[near[2]]:.2f}. Over the specs measured here, "
                      "every table with a call that near has a wrong row in it and "
                      "every table without one is exact. Nothing later catches it: "
                      "grit marks the circuit against whatever table it is handed. "
                      "Say the rule more plainly, work an in-between value out first "
                      "with --via, or pass --table. --force searches anyway.")
            return 3
        if stopping and not args.force:
            if args.json:
                print(json.dumps({**report, "table": table, "searched": False}, indent=2))
            else:
                print()
                print(f"  read whole, this rule leaves something open (P={whole:.2f}).")
                if not unclear:
                    print("  No single row reads that way, which is a parting "
                          "worth knowing about; these are the nearest the middle:")
                for row in (unclear or nearest):
                    for name, held in sorted(row.undecided.items(), key=lambda one: -one[1]):
                        seen = ", ".join(f"{n}={v}" for n, v in row.bits.items())
                        print(f"  with {seen}: `{name}` at {held:.2f} undecided "
                              f"(it leaned {row.outs[name]} at {row.probs[name]:.2f})")
                print("\nThe rule does not decide these rows. Say what they should be and\n"
                      "ask again, or pass --table to state the whole table yourself.\n"
                      "Nothing was searched: an unreviewed row becomes a gate that is\n"
                      "wrong on purpose.")
            return 3

    report["table"] = table
    ask = {"inputs": len(inputs), "outputs": len(outputs),
           "gates": args.gates, "table": table}

    found = grit("grit_least" if args.least else "grit_synthesise", ask, args.mcp)
    report["grit"] = found

    if args.json:
        print(json.dumps(report, indent=2))
        return 0 if (found.get("solved") or found.get("smallest_that_solved")) else 1

    if args.least:
        ladder = found.get("ladder", [])
        print("\nladder: " + "  ".join(
            f"{rung.get('gates')}{'ok' if rung.get('solved') else 'X'}"
            for rung in ladder))
        smallest = found.get("smallest_that_solved")
        if smallest is None:
            print("nothing solved on this ladder; start from more gates, or raise crowd")
            return 1
        print(f"smallest budget that solved: {smallest} gates")
        # grit_least nests the winning run under `circuit`, shaped like a
        # grit_synthesise answer rather than like a bare list of gates.
        circuit = found.get("circuit") or {}
        print(f"marked {circuit.get('marks')}/{circuit.get('of')} on the host")
        gates = circuit.get("gates", [])
    else:
        if not found.get("solved"):
            print(f"\nnot solved: {found.get('marks')}/{found.get('of')} rows marked, "
                  f"wrong rows {found.get('wrong_rows')}")
            print("this is evidence about the gate budget, not proof no circuit exists")
            return 1
        print(f"\nsolved {found.get('marks')}/{found.get('of')} rows, "
              f"checked on the host: {found.get('checked_on_the_host')}")
        gates = found.get("gates", [])

    for n, gate in enumerate(gates):
        print(f"  g{n + len(inputs)} = kind {gate.get('kind'):>2}"
              f"  left {gate.get('left')}  right {gate.get('right')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
