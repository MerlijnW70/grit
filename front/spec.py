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
                outputs: list[str], model: str | None = None) -> list[Row]:
    """One request. Every (row, output bit) pair is an independent judgement."""
    rows = [
        Row(index=r, bits={n: (r >> i) & 1 for i, n in enumerate(inputs)})
        for r in range(2 ** len(inputs))
    ]
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
                rows = read_intent(client, args.rule, inputs, outputs, args.model)
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
