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

# Unlike the other two thresholds in this tool, this one is a real cut and not a
# preference. `bench.py --suite route` stresses the boundary from both sides -
# windows worded like accumulators ("at least two of the last three", "the last
# four add up to more than two") and accumulators worded like windows ("three 1s
# have arrived in a row at some point, and it stays 1 from then on") - and the
# two populations land at opposite ends with nothing in between. Almost any
# floor separates them, so this number is not delicate.
#
# `windowed` carries the decision. `accumulates` is corroboration and is read
# literally: a latch that remembers whether something ever happened is a single
# remembered bit rather than a running tally, so it scores low on a question
# about tallies while still being no kind of window. Taking the pair together -
# min(windowed, 1 - accumulates) - routes it correctly where either noul alone
# would not.
WINDOW_FLOOR = 0.50

SPANS = {"1": "only what arrives at this step matters, with no earlier step needed",
         "2": "this step and the one before it",
         "3": "this step and the two before it",
         "4": "this step and the three before it",
         # Without this, a deeper rule is answered with the deepest option on
         # offer, and answered confidently - a forced choice has no way to say
         # none of these, so it says the nearest one and sounds certain.
         "more": "it needs to look further back than the three steps before this one"}

# A weak guard, and weaker than it used to be. `bench.py --suite depth` asks
# rules of known depth at their own depth and at one shallower, where provably
# no table can carry them, and the two populations now overlap almost entirely:
# an impossible window scores about what a correct one does.
#
# That got worse on purpose. Naming each step by how far back it is, rather
# than leaving the order to be recovered from a list, fixed a two-input rule
# that had been reading every value as belonging to the current step - and it
# also made the model decisive about windows it cannot possibly decide. Clearer
# questions bought better labels and cost most of this signal.
#
# So it is kept as a catastrophe detector rather than a depth check: at this
# floor it rejects nothing that was adequate and still catches the worst of the
# inadequate. The depth question itself is the real guard, and the routing
# suite shows it answering with confidence and answering right. Read the table.
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
            instructions=("How many steps of history does the rule in `rule` need to "
                          "decide a step's output, counting that step itself?"),
            criteria=SPANS),
    })
    span = answers.choices["span"]
    return {"windowed": answers.nouls["windowed"].noul,
            "accumulates": answers.nouls["accumulates"].noul,
            "span": None if span.choice == "more" else int(span.choice),
            "span_choice": span.choice,
            "span_confidence": span.confidence}


def window_table(client: TypeSafeClient, rule: str, k: int, names: list[str],
                 outputs: list[str], model: str | None = None
                 ) -> tuple[dict[int, dict], float, float]:
    """Every window the run can present, judged once, keyed by how long it is.

    Not just the 2^k full windows. The first k-1 steps of a run have less
    history than the window wants, and what the rule says there is a question,
    not an assumption. Padding those with zeros - the obvious shortcut - labels
    "the last three inputs were all the same" as true at step 0, because three
    imaginary zeros are indeed all the same. The rule means no such thing.

    So a short prefix is asked about as a short prefix: this is the start, only
    this many steps have happened. With n named inputs a window of k steps is
    k*n bits, so the count is sum over lengths of 2^(length*n), in one request
    and still flat in how many runs get labelled afterwards. It grows fast in
    both k and n, which is the real limit on how wide a machine this can spec.
    """
    questions, shapes = {}, {}
    for length in range(1, k + 1):
        for r in range(len(names) and 2 ** (length * len(names))):
            # A window is `length` words, newest first; each word packs one bit
            # per named input, lowest bit first.
            window = tuple((r >> (step * len(names))) & (2 ** len(names) - 1)
                           for step in range(length))
            # Each step is named by how far back it is, rather than left to be
            # recovered from an ordering. Said as a bare sequence, a two-input
            # rule came back reading every input as belonging to the current
            # step, and the table fired on the newest word alone.
            said = [", ".join(f"{name}={(word >> bit) & 1}"
                              for bit, name in enumerate(names))
                    for word in window]
            def when(ago: int) -> str:
                if ago == 0:
                    return "at this step"
                if ago == 1:
                    return "at the step immediately before this one"
                return f"{ago} steps before this one"
            told = "; ".join(f"{when(ago)}, {said[ago]}"
                             for ago in reversed(range(length)))
            if len(names) == 1:
                # A rule written as a sequence ("1 then 0 then 1") is answered
                # far more sharply when the window is shown as one too. The
                # per-step naming alone cost a 101 detector most of its margin.
                run = " then ".join(str((word >> 0) & 1) for word in reversed(window))
                told = f"in order, oldest first, the inputs were {run} - that is, {told}"
            if length == k:
                where = f"the inputs were: {told}" if len(names) > 1 else told
            else:
                many = "step has" if length == 1 else f"{length} steps have"
                where = (f"this is the very start of the run, so only {many} happened "
                         f"at all, and the inputs were: {told}")
            for out in outputs:
                key = f"w{length}_{r}_{out}"
                shapes[key] = (length, window, out)
                questions[key] = Noul(instructions=(
                    f"Applying the rule in `rule`: {where}. "
                    f"The output bit `{out}` at this step is 1."
                ))
    answers = client.system_one(state={"rule": rule}, questions=questions, model=model)
    tables: dict[int, dict] = {length: {} for length in range(1, k + 1)}
    # Kept apart on purpose. The full windows answer "is this depth enough",
    # which is what TABLE_FLOOR guards. The run-in answers "does the rule say
    # what happens before it has its history", a harder and different question
    # whose uncertainty says nothing about the depth.
    full, run_in = 1.0, 1.0
    for key, (length, window, out) in shapes.items():
        p = answers.nouls[key].noul
        word = tables[length].setdefault(window, 0)
        if p > 0.5:
            tables[length][window] = word | (1 << outputs.index(out))
        if length == k:
            full = min(full, abs(p - 0.5))
        else:
            run_in = min(run_in, abs(p - 0.5))
    return tables, full, run_in


def label(feed: list[int], tables: dict[int, dict], k: int) -> list[int]:
    """Each step reads the window it actually has, short ones included."""
    out = []
    for t in range(len(feed)):
        length = min(t + 1, k)
        out.append(tables[length][tuple(feed[t - i] for i in range(length))])
    return out


def make_runs(rng: random.Random, many: int, steps: int, table: dict[int, dict],
              k: int, inputs: int = 1) -> list[dict]:
    """A feed is one word a step, not one bit: each word packs every input."""
    runs = []
    for _ in range(many):
        feed = [rng.randrange(2 ** inputs) for _ in range(steps)]
        runs.append({"feed": feed, "want": label(feed, table, k)})
    return runs


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="machine", description="Synthesise a machine from an English rule, then probe it.")
    parser.add_argument("rule", help="the rule, in plain English")
    parser.add_argument("-i", "--inputs", default="x",
                        help="comma-separated input names, lowest bit first")
    parser.add_argument("-o", "--outputs", default="out",
                        help="comma-separated output names, lowest bit first")
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
    names = [n.strip() for n in args.inputs.split(",") if n.strip()]
    outputs = [n.strip() for n in args.outputs.split(",") if n.strip()]
    if not names or not outputs:
        parser.error("need at least one input and one output")

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
            table, worst, run_in = window_table(client, args.rule, k, names,
                                                outputs, args.model)
            if worst < TABLE_FLOOR and not args.force:
                if args.json:
                    print(json.dumps({"rule": args.rule, "shape": shape,
                                      "window": k, "worst_margin": round(worst, 3),
                                      "labelled": False}, indent=2))
                else:
                    print(f"\nThe {2 ** (k * len(names))} full windows close to "
                          f"{worst:.2f} of the middle. The answer depends on "
                          f"something\nthese {k} steps do not show, so nothing was taught: a machine "
                          "fitted to coin flips still solves.")
                return 3
    except TypeSafeAPIError as error:
        print(f"TypeSafe error {error.status} (request {error.request_id})", file=sys.stderr)
        return 2

    if not args.json:
        print(f"\nwindow tables, {sum(2 ** (n * len(names)) for n in range(1, k + 1))} windows "
              f"(full windows close to {worst:.2f}, run-in to {run_in:.2f}), "
              f"newest input first:")
        for length in sorted(table):
            head = "full window" if length == k else f"run-in, {length} so far"
            print(f"  {head}:")
            for window, word in sorted(table[length].items()):
                shown = " ".join(f"{w:0{len(names)}b}" for w in reversed(window))
                print(f"    {shown} -> {word:0{len(outputs)}b}")

    rng = random.Random(args.seed)
    taught = make_runs(rng, args.runs, args.steps, table, k, len(names))
    probes = make_runs(rng, args.probe_runs, args.probe_steps, table, k, len(names))

    ask = {"state": args.state, "inputs": len(names), "outputs": len(outputs),
           "gates": args.gates,
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
