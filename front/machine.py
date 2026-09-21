"""English rule -> labelled runs -> grit_machine -> the gap it left behind.

`grit_machine` says plainly that solving the runs it was handed is not the same
as holding the rule behind them. Closing that needs runs it has never seen, and
those need wanted outputs, which is exactly what you do not have if you are
synthesising a machine instead of writing one.

So the labels come from the rule, through Jev, and never from a simulator.

Per-step labelling is the obvious way and the wrong one. Two moments sharing
the same recent inputs can come back labelled differently, and then no machine
fits at all; and a rule that tallies over all history runs into a documented
jagged edge, where counting error grows with the length of the count. Measured
against a running-parity rule, per-step labelling drops a third of its steps
and does it quietly - the labels still look like labels.

This asks about the rule instead:

  window rule       one truth table over the 2^k recent-input patterns, asked
                    once. Every step is then a lookup, so identical windows
                    cannot disagree and the labels are always fittable.
  accumulating rule refused here. The fold belongs in code, and the tally is
                    the part the model is documented to get wrong.

A window table costs 2^k questions no matter how many runs are labelled from
it, and against rules whose answers are computable it has labelled every step
correctly. `bench.py --suite trace` re-measures that.

Getting the DEPTH wrong is the one mistake nothing downstream can see. The
labels come from a lookup, so they are self-consistent at any depth: grit
always solves them, and the unseen-run probe is built from the same table, so
it agrees. Both checks are blind here. TABLE_FLOOR guards it and guards it
weakly - `bench.py --suite depth` measures how weakly. Read the printed table.

Exit codes: 0 held on everything unseen, 1 taught but left a gap, 2 TypeSafe or
key trouble, 3 refused - an accumulating rule, a window deeper than the options
offer, or a table too close to coin flips.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import sys

from typesafe_sdk import Choice, Noul, TypeSafeAPIError, TypeSafeClient

from spec import DEFAULT_MCP, grit

# Below this on `windowed`, or above it on `accumulates`, the rule is not one
# a recent-input table can carry.
WINDOW_FLOOR = 0.50

SPANS = {"1": "only the input at this step matters",
         "2": "this step and the one before it",
         "3": "this step and the two before it",
         "4": "this step and the three before it",
         # Without this, a deeper rule is answered with the deepest option on
         # offer, and answered confidently - a forced choice has no way to say
         # none of these, so it says the nearest one and sounds certain.
         "more": "it needs to look further back than the three before this step"}

# A weak guard, and known to be weak. `bench.py --suite depth` asks rules of
# known depth at their own depth and at one shallower, where provably no table
# can carry them, and the two populations interleave: a correct table over a
# short window scores about what an impossible one does, because the margin
# sags at both ends - few patterns, and many. It catches the extreme cases and
# nothing subtler. The floor sits where nothing inadequate got through, since a
# rejection costs a --force and an undersized window costs a machine that is
# wrong wherever the rule actually mattered.
TABLE_FLOOR = 0.30


def read_shape(client: TypeSafeClient, rule: str, model: str | None = None) -> dict:
    """One request. What kind of rule is this, and how far back does it look?"""
    answers = client.system_one(state={"rule": rule}, model=model, questions={
        "windowed": Noul(instructions=(
            "The rule in `rule` decides each step's output from a bounded number of the "
            "most recent inputs only. Two different moments that share the same recent "
            "inputs always get the same output under this rule, no matter what came "
            "before them."
        )),
        "accumulates": Noul(instructions=(
            "Applying the rule in `rule` requires keeping a running tally, count, sum or "
            "other value accumulated over every input seen so far. Remembering only the "
            "last few inputs would not be enough to apply it."
        )),
        "span": Choice(
            instructions="How many of the most recent inputs does the rule in `rule` need?",
            criteria=SPANS),
    })
    span = answers.choices["span"]
    return {"windowed": answers.nouls["windowed"].noul,
            "accumulates": answers.nouls["accumulates"].noul,
            "span": None if span.choice == "more" else int(span.choice),
            "span_choice": span.choice,
            "span_confidence": span.confidence}


def window_table(client: TypeSafeClient, rule: str, k: int,
                 model: str | None = None) -> tuple[dict[tuple[int, ...], int], float]:
    """The 2^k recent-input patterns, judged once. Returns the table and its worst margin."""
    patterns = [tuple((r >> i) & 1 for i in range(k)) for r in range(2 ** k)]
    questions = {}
    for n, pattern in enumerate(patterns):
        oldest_first = ", ".join(str(b) for b in reversed(pattern))
        questions[f"p{n}"] = Noul(instructions=(
            f"Applying the rule in `rule`: the last {k} inputs, oldest first, were "
            f"{oldest_first}, so the input at this step is {pattern[0]}. "
            "The output at this step is 1."
        ))
    answers = client.system_one(state={"rule": rule}, questions=questions, model=model)
    probabilities = {pattern: answers.nouls[f"p{n}"].noul
                     for n, pattern in enumerate(patterns)}
    table = {pattern: (1 if p > 0.5 else 0) for pattern, p in probabilities.items()}
    return table, min(abs(p - 0.5) for p in probabilities.values())


def label(feed: list[int], table: dict[tuple[int, ...], int], k: int) -> list[int]:
    """A machine starts from a state of all zeros, so the run-in pads with zeros."""
    return [table[tuple(feed[t - i] if t - i >= 0 else 0 for i in range(k))]
            for t in range(len(feed))]


def make_runs(rng: random.Random, many: int, steps: int,
              table: dict[tuple[int, ...], int], k: int) -> list[dict]:
    runs = []
    for _ in range(many):
        feed = [rng.randint(0, 1) for _ in range(steps)]
        runs.append({"feed": feed, "want": label(feed, table, k)})
    return runs


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="machine", description="Synthesise a machine from an English rule, then probe it.")
    parser.add_argument("rule", help="the rule, in plain English")
    parser.add_argument("--state", type=int, default=2, help="state bits")
    parser.add_argument("-g", "--gates", type=int, default=6, help="gate budget")
    parser.add_argument("--runs", type=int, default=3, help="how many runs to teach on")
    parser.add_argument("--steps", type=int, default=12, help="steps per taught run")
    parser.add_argument("--probe-runs", type=int, default=8, help="how many unseen runs")
    parser.add_argument("--probe-steps", type=int, default=32, help="steps per unseen run")
    parser.add_argument("--crowd", type=int, default=4096)
    parser.add_argument("--rounds", type=int, default=2000)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--force", action="store_true",
                        help="label even a rule that needs a running tally")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--model", help="TypeSafe model (default jev-latest)")
    parser.add_argument("--mcp", default=os.environ.get("GRIT_MCP", DEFAULT_MCP))
    args = parser.parse_args(argv)

    if not os.environ.get("TYPESAFE_API_KEY"):
        print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
        return 2

    try:
        with TypeSafeClient() as client:
            shape = read_shape(client, args.rule, args.model)
            windowed = (shape["windowed"] > WINDOW_FLOOR
                        and shape["accumulates"] < WINDOW_FLOOR)
            if not args.json:
                print(f"rule shape: windowed={shape['windowed']:.2f} "
                      f"accumulates={shape['accumulates']:.2f} "
                      f"span={shape['span']} (confidence {shape['span_confidence']:.2f})")
            if not windowed and not args.force:
                if args.json:
                    print(json.dumps({"rule": args.rule, "shape": shape,
                                      "labelled": False}, indent=2))
                else:
                    print("\nThis rule needs a running tally rather than a recent window.\n"
                          "Labelling it step by step is the counting failure mode, so it is\n"
                          "refused: fold the tally in code and hand grit the runs directly,\n"
                          "or pass --force and read the marks knowing the labels may be wrong.")
                return 3
            if shape["span"] is None and not args.force:
                if args.json:
                    print(json.dumps({"rule": args.rule, "shape": shape,
                                      "labelled": False}, indent=2))
                else:
                    deepest = max(int(n) for n in SPANS if n.isdigit())
                    print(f"\nThis rule looks further back than {deepest} inputs.\n"
                          "A table that deep is not asked for here: say the rule in terms\n"
                          "of a shallower window if you can, or hand grit the runs yourself.")
                return 3
            k = shape["span"] or 4
            table, worst = window_table(client, args.rule, k, args.model)
            if worst < TABLE_FLOOR and not args.force:
                if args.json:
                    print(json.dumps({"rule": args.rule, "shape": shape,
                                      "window": k, "worst_margin": round(worst, 3),
                                      "labelled": False}, indent=2))
                else:
                    print(f"\nThe {2 ** k}-pattern table's closest call is {worst:.2f} from "
                          f"the middle.\nPatterns this undecided mean the window does not "
                          "carry the rule:\nthe answer depends on something these "
                          f"{k} inputs do not show.\nNothing was taught, because a machine "
                          "fitted to coin flips still solves.")
                return 3
    except TypeSafeAPIError as error:
        print(f"TypeSafe error {error.status} (request {error.request_id})", file=sys.stderr)
        return 2

    if not args.json:
        print(f"\nwindow table over {2 ** k} patterns (worst margin {worst:.2f}), "
              f"newest input first:")
        for pattern, bit in sorted(table.items()):
            print(f"  {''.join(str(b) for b in pattern)} -> {bit}")

    rng = random.Random(args.seed)
    taught = make_runs(rng, args.runs, args.steps, table, k)
    probes = make_runs(rng, args.probe_runs, args.probe_steps, table, k)

    ask = {"state": args.state, "inputs": 1, "outputs": 1, "gates": args.gates,
           "crowd": args.crowd, "rounds": args.rounds, "traces": taught}
    found = grit("grit_machine", ask, args.mcp)
    solved = bool(found.get("solved"))

    held = grit("grit_machine", {**ask, "chip": found.get("gates", []),
                                 "traces": probes}, args.mcp)
    marks, of = held.get("marks", 0), held.get("of", 0)

    report = {"rule": args.rule, "shape": shape, "window": k,
              "taught": {"runs": args.runs, "steps": args.steps, "solved": solved},
              "unseen": {"runs": args.probe_runs, "steps": args.probe_steps,
                         "marks": marks, "of": of}}
    if args.json:
        print(json.dumps(report, indent=2))
        return 0 if solved and marks == of else 1

    print(f"\ntaught on {args.runs} runs of {args.steps} steps: "
          f"solved={solved} ({found.get('marks')}/{found.get('of')})")
    print(f"unseen {args.probe_runs} runs of {args.probe_steps} steps: {marks}/{of}")
    if not solved:
        print("it did not even answer the runs it was given; raise gates, crowd or rounds")
        return 1
    if marks == of:
        print("it held on everything it never saw, which is evidence for the rule "
              "behind the runs rather than proof of it")
        return 0
    print(f"THE GAP: it answers every run it was taught and misses {of - marks} "
          f"of {of} it never saw.\nTeach it longer runs rather than more of them.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
