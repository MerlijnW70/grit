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


def read_shape(client: TypeSafeClient, rule: str, inputs: list[str],
               outputs: list[str], model: str | None = None) -> dict:
    """Does this rule want the inputs read as a number and computed with?"""
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
        })
    return {"arithmetic": answers.nouls["arithmetic"].noul,
            "counting": answers.nouls["counting"].noul}


def read_relevance(client: TypeSafeClient, rule: str, inputs: list[str],
                   outputs: list[str], model: str | None = None) -> dict[tuple[str, str], float]:
    """Which inputs each output actually reads."""
    questions = {}
    for name in inputs:
        for out in outputs:
            questions[f"rel_{name}_{out}"] = Noul(instructions=(
                f"Under the rule in `rule`, the value of input `{name}` can change what "
                f"output `{out}` should be. That is: there are two cases differing only "
                f"in `{name}`, every other input the same, where the rule calls for a "
                f"different `{out}`."
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
                  model: str | None = None) -> list[tuple[str, str]]:
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
            for r in range(2 ** len(rest)):
                background = {other: (r >> i) & 1 for i, other in enumerate(rest)}
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
                relevance: dict[tuple[str, str], float] | None = None) -> list[Row]:
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
    questions = {}
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
            questions[f"u{row.index}_{out}"] = Noul(instructions=(
                "Read the rule in `rule` under the usual convention: it names the cases "
                f"where `{out}` is 1, and `{out}` is 0 in every case it does not name. "
                f"Even under that convention, the rule fails to settle `{out}` when "
                f"{seen}: the rule's own words leave this case genuinely open, so two "
                "careful readers could apply it here and get different answers. This is "
                "about the rule being incomplete or vague, not about the case being rare "
                f"or about `{out}` simply being 0 here."
            ))
    answers = client.system_one(state=state, questions=questions, model=model)
    for row in rows:
        for out in outputs:
            p = answers.nouls[f"r{row.index}_{out}"].noul
            row.probs[out] = p
            row.outs[out] = 1 if p > 0.5 else 0
            row.undecided[out] = answers.nouls[f"u{row.index}_{out}"].noul
    return rows


def _read_pruned(client, rule, inputs, outputs, rows, relevance, model):
    """Ask once per case an output can tell apart, then project onto every row."""
    kept = {out: relevant_inputs(relevance, out, inputs) for out in outputs}
    questions = {}
    for out in outputs:
        for r in range(2 ** len(kept[out])):
            case = {name: (r >> i) & 1 for i, name in enumerate(kept[out])}
            seen = ", ".join(f"{name}={bit}" for name, bit in case.items())
            questions[f"v{out}_{r}"] = Noul(instructions=(
                f"For the rule in `rule`, with {seen}, the output bit `{out}` is 1. "
                "No other input matters to this output."
            ))
            questions[f"u{out}_{r}"] = Noul(instructions=(
                "Read the rule in `rule` under the usual convention: it names the cases "
                f"where `{out}` is 1, and `{out}` is 0 in every case it does not name. "
                f"Even under that convention, the rule fails to settle `{out}` when "
                f"{seen}: the rule's own words leave this case genuinely open, so two "
                "careful readers could apply it here and get different answers."
            ))
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
    return rows


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
                shape = read_shape(client, args.rule, inputs, outputs, args.model)
                report["shape"] = {k: round(v, 3) for k, v in shape.items()}
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
                relevance = (read_relevance(client, args.rule, inputs, outputs, args.model)
                             if args.prune else None)
                if relevance is not None:
                    for name, out in check_pruning(client, args.rule, inputs, outputs,
                                                   relevance, args.model):
                        relevance[(name, out)] = 1.0
                        if not args.json:
                            print(f"put back: `{name}` decides `{out}` after all")
                    if not args.json:
                        asked = sum(2 ** len(relevant_inputs(relevance, out, inputs))
                                    for out in outputs)
                        print("reads: " + "; ".join(
                            f"{out} <- " + ", ".join(relevant_inputs(relevance, out, inputs))
                            for out in outputs)
                            + f"  ({asked} cases instead of "
                            + f"{2 ** len(inputs) * len(outputs)})")
                rows = read_intent(client, args.rule, inputs, outputs, args.model,
                                   relevance)
        except TypeSafeAPIError as error:
            print(f"TypeSafe error {error.status} (request {error.request_id})",
                  file=sys.stderr)
            return 2
        table = [row.word(outputs) for row in rows]
        report["table_from"] = "jev"

        unclear = [row for row in rows if row.unclear(args.margin)]
        report["unclear"] = [
            {"row": row.index, "bits": row.bits, "bit": name,
             "p": round(row.probs[name], 3),
             "p_undecided": round(row.undecided[name], 3)}
            for row in unclear for name in row.unclear(args.margin)
        ]
        if not args.json:
            print(render(rows, inputs, outputs))
        if unclear and not args.force:
            if args.json:
                print(json.dumps({**report, "table": table, "searched": False}, indent=2))
            else:
                print()
                for row in unclear:
                    for name in row.unclear(args.margin):
                        seen = ", ".join(f"{n}={v}" for n, v in row.bits.items())
                        print(f"  undecided: with {seen}, the rule does not say what "
                              f"`{name}` is  (P={row.undecided[name]:.2f}; "
                              f"it leaned {row.outs[name]} at {row.probs[name]:.2f})")
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
