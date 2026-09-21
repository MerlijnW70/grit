"""Re-measure the numbers the README quotes.

Every claim in front/README.md about how well the model answers comes from
here. They are not asserted in test_front.py on purpose: a threshold pinned in
a test breaks on a model update rather than on a bug. But a number nobody can
reproduce is just a story, so this runs the same cases on demand and prints the
model that answered them.

    python front/bench.py                        # everything
    python front/bench.py --suite spec           # the undecided detector
    python front/bench.py --suite depth          # how good a guard TABLE_FLOOR is
    python front/bench.py --cases mine.json      # your specs instead of these
    python front/bench.py --save run.json        # keep a run
    python front/bench.py --sweep-from run.json  # re-read it; no key, no requests

Every suite also sweeps its own threshold and prints what each floor would have
done to the cases, because one number hides whether a floor was a real cut or
just a preference. In this tool they are preferences: the populations interleave
in both, so a band of floors all get the same count wrong and only move which
ones. Set them from your own specs with --cases; these were invented while
building the thing they calibrate.

Needs TYPESAFE_API_KEY. No device and no grit-mcp: nothing here synthesises.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from typesafe_sdk import TypeSafeAPIError, TypeSafeClient

import machine
import spec

# --- the cases, which are the single source of truth for the README ----------

# Specs that state a condition outright.
STATED = [
    ("at least two of the three inputs are 1", ["a", "b", "c"]),
    ("an odd number of the inputs are 1", ["a", "b", "c"]),
    ("exactly two of the three inputs are 1", ["a", "b", "c"]),
    ("all three inputs are 1", ["a", "b", "c"]),
    ("the first input is 1 and the second is 0", ["a", "b"]),
    ("exactly one of the inputs is 1", ["a", "b", "c"]),
]

# Clear to an engineer, but naming a component states no condition at all.
NAMED = [
    ("carry out of a full adder", ["x", "y", "cin"]),
    ("a three-input majority gate", ["a", "b", "c"]),
    ("the sum bit of a half adder", ["a", "b"]),
]

# Genuinely underdetermined: some row has no answer the words decide.
VAGUE = [
    ("output 1 when the inputs are mostly on", ["a", "b"]),
    ("output 1 when there are a lot of 1s", ["a", "b", "c"]),
    ("output 1 when the inputs are roughly balanced", ["a", "b", "c"]),
    ("output 1 when enough of the inputs are set", ["a", "b", "c"]),
    ("output 1 when the first input matters most", ["a", "b", "c"]),
]

# Specs whose table is computable, so the labels can be marked rather than read.
TABLES = [
    ("at least two of the three inputs are 1", ["a", "b", "c"],
     lambda a, b, c: int(a + b + c >= 2)),
    ("an odd number of the inputs are 1", ["a", "b", "c"],
     lambda a, b, c: (a + b + c) % 2),
    ("exactly two of the three inputs are 1", ["a", "b", "c"],
     lambda a, b, c: int(a + b + c == 2)),
    ("1 when at least two of x, y and cin are 1", ["x", "y", "cin"],
     lambda x, y, c: int(x + y + c >= 2)),
]

# Temporal rules, with ground truth over the whole prefix.
TRACES = [
    ("output 1 at a step exactly when the last three inputs, ending at that step, "
     "were 1 then 0 then 1",
     lambda p: int(len(p) >= 3 and p[-3:] == [1, 0, 1]), "window"),
    ("output 1 at a step exactly when the input at that step is 1 and the input "
     "before it was also 1",
     lambda p: int(len(p) >= 2 and p[-1] == 1 and p[-2] == 1), "window"),
    ("output 1 at a step exactly when an odd number of 1s have been seen so far, "
     "counting that step",
     lambda p: sum(p) % 2, "accumulate"),
    ("output 1 at a step exactly when strictly more than half of all inputs so far "
     "have been 1",
     lambda p: int(sum(p) * 2 > len(p)), "accumulate"),
]


# Window rules with a known true depth. Each is asked at its own depth, where a
# table can carry it, and at one shallower, where no table can: the answer then
# depends on an input the window does not show. That makes a labelled set for
# TABLE_FLOOR without anyone deciding by eye what "coin flips" means.
DEPTHS = [
    ("output 1 at a step exactly when the input at that step is 1",
     1, lambda p: p[-1]),
    ("output 1 at a step exactly when the input at that step is 1 and the input "
     "before it was also 1",
     2, lambda p: int(len(p) >= 2 and p[-1] == 1 and p[-2] == 1)),
    ("output 1 at a step exactly when the input at that step is 0 and the input "
     "before it was also 0",
     2, lambda p: int(p[-1] == 0 and (len(p) < 2 or p[-2] == 0))),
    ("output 1 at a step exactly when the last three inputs, ending at that step, "
     "were 1 then 0 then 1",
     3, lambda p: int(len(p) >= 3 and p[-3:] == [1, 0, 1])),
    ("output 1 at a step exactly when the last three inputs, ending at that step, "
     "were all the same as each other",
     3, lambda p: int(len(p) >= 3 and len(set(p[-3:])) == 1)),
    ("output 1 at a step exactly when the last four inputs, ending at that step, "
     "were all 1",
     4, lambda p: int(len(p) >= 4 and p[-4:] == [1, 1, 1, 1])),
]


class Metered:
    """Duck-types the client so the suites can be billed as well as scored."""

    def __init__(self, client: TypeSafeClient):
        self.client, self.calls, self.input, self.output, self.model = client, 0, 0, 0, None

    def system_one(self, *args, **kwargs):
        answer = self.client.system_one(*args, **kwargs)
        self.calls += 1
        self.input += answer.usage.input_tokens
        self.output += answer.usage.output_tokens
        self.model = answer.model
        return answer


DEFAULT_GROUPS = [("stating a condition", STATED, False),
                  ("naming a component", NAMED, False),
                  ("vague", VAGUE, True)]


def load_cases(path: str) -> list[tuple[str, list, bool]]:
    """Your specs instead of mine. The floor is only as good as the specs it was set on.

    {"cases": [{"rule": "...", "inputs": ["a","b"], "should_stop": false,
                "group": "optional label"}]}
    """
    held = json.loads(Path(path).read_text(encoding="utf-8"))
    grouped: dict[str, list] = {}
    for case in held["cases"]:
        group = case.get("group", "stop" if case["should_stop"] else "search")
        grouped.setdefault((group, bool(case["should_stop"])), []).append(
            (case["rule"], list(case["inputs"])))
    return [(name, specs, stop) for (name, stop), specs in grouped.items()]


def cut(scored: list[tuple[float, bool]], floors: list[float] | None = None,
        reject_above: bool = True) -> dict:
    """What every floor would have done to these (value, should_reject) pairs.

    No requests: a threshold is a cut over numbers already measured. With
    reject_above, a value over the floor is rejected - that is the undecided
    detector, where a high score means stop. With it false, a value UNDER the
    floor is rejected - that is a table margin, where a low score means stop.
    """
    floors = floors or [round(0.05 * n, 2) for n in range(1, 20)]
    rows = []
    for floor in floors:
        def rejected(value: float) -> bool:
            return value > floor if reject_above else value < floor
        alarms = sum(1 for value, stop in scored if rejected(value) and not stop)
        missed = sum(1 for value, stop in scored if not rejected(value) and stop)
        rows.append({"floor": floor, "false_alarms": alarms, "missed": missed,
                     "wrong": alarms + missed})
    fewest = min(row["wrong"] for row in rows)
    return {"rows": rows, "specs": len(scored), "fewest_wrong": fewest,
            "best_floors": [row["floor"] for row in rows if row["wrong"] == fewest],
            "separable": fewest == 0}


def sweep(report: dict, floors: list[float] | None = None) -> dict:
    """What every floor would have done. No requests: it is a cut over the peaks."""
    seen = [(case["peak"], group["should_stop"])
            for group in report["groups"] for case in group["cases"]]
    return cut(seen, floors, reject_above=True)



def bench_spec(client, groups=None) -> dict:
    """Does the undecided question stop the specs that should be stopped?"""
    groups = groups or DEFAULT_GROUPS
    out = {"groups": [], "wrong": 0}
    for name, specs, should_stop in groups:
        rows_out, peaks, stopped_count = [], [], 0
        for rule, names in specs:
            rows = spec.read_intent(client, rule, names, ["out"])
            flagged = [r for r in rows if r.unclear(spec.UNDECIDED_FLOOR)]
            peak = max((max(r.undecided.values()) for r in rows), default=0.0)
            stopped = bool(flagged)
            stopped_count += stopped
            wrong = stopped != should_stop
            out["wrong"] += wrong
            peaks.append(peak)
            rows_out.append({"rule": rule, "stopped": stopped, "peak": round(peak, 2),
                             "as_intended": not wrong})
        out["groups"].append({
            "group": name, "specs": len(specs), "stopped": stopped_count,
            "should_stop": should_stop, "peak_low": round(min(peaks), 2),
            "peak_high": round(max(peaks), 2), "cases": rows_out})
    out["specs"] = sum(len(s) for _, s, _ in groups)
    out["as_intended"] = out["specs"] - out["wrong"]
    out["floor"] = spec.UNDECIDED_FLOOR
    out["sweep"] = sweep(out)
    return out


def bench_table(client) -> dict:
    """Are the tables exactly right where the answer is computable?"""
    cases = []
    exact = 0
    for rule, names, truth in TABLES:
        rows = spec.read_intent(client, rule, names, ["out"])
        got = [row.outs["out"] for row in rows]
        want = [truth(*[(r >> i) & 1 for i in range(len(names))])
                for r in range(2 ** len(names))]
        ok = got == want
        exact += ok
        cases.append({"rule": rule, "exact": ok, "got": got, "want": want,
                      "wrong_rows": [i for i, (a, b) in enumerate(zip(got, want)) if a != b]})
    return {"specs": len(TABLES), "exact": exact, "cases": cases}


def bench_trace(client) -> dict:
    """Does the router route, and does a window table label every step right?"""
    rng = random.Random(7)
    feeds = [[rng.randint(0, 1) for _ in range(12)] for _ in range(3)]
    cases = []
    routed = labelled_right = windows = 0
    for rule, truth, expected in TRACES:
        shape = machine.read_shape(client, rule)
        kind = ("window" if shape["windowed"] > machine.WINDOW_FLOOR
                and shape["accumulates"] < machine.WINDOW_FLOOR else "accumulate")
        ok = kind == expected
        routed += ok
        case = {"rule": rule[:56], "expected": expected, "routed": kind,
                "windowed": round(shape["windowed"], 2),
                "accumulates": round(shape["accumulates"], 2),
                "span": shape["span"], "span_confidence": round(shape["span_confidence"], 2),
                "as_intended": ok}
        if kind == "window" and ok and shape["span"]:
            windows += 1
            table, worst = machine.window_table(client, rule, shape["span"])
            steps = wrong = 0
            for feed in feeds:
                got = machine.label(feed, table, shape["span"])
                want = [truth(feed[:t + 1]) for t in range(len(feed))]
                steps += len(feed)
                wrong += sum(a != b for a, b in zip(got, want))
            labelled_right += steps - wrong
            case.update({"steps": steps, "steps_right": steps - wrong,
                         "questions": 2 ** shape["span"], "worst_margin": round(worst, 2)})
        cases.append(case)
    return {"rules": len(TRACES), "routed": routed, "windows": windows,
            "steps_right": labelled_right, "cases": cases}


def bench_depth(client) -> dict:
    """Does a table's own worst margin know when its window is too shallow?

    A rule of depth d asked at depth d can be labelled exactly; asked at d-1 it
    cannot, whatever the model says, because two different answers share one
    window. So "is this depth adequate" is known in advance, and the margin is
    only a predictor of it.
    """
    cases = []
    for rule, depth, truth in DEPTHS:
        for k in {depth, depth - 1} - {0}:
            adequate = k >= depth
            table, worst = machine.window_table(client, rule, k)
            # At an adequate depth the whole table is checkable against truth.
            wrong = None
            if adequate:
                wrong = sum(1 for pattern, bit in table.items()
                            if truth(list(reversed(pattern))) != bit)
            cases.append({"rule": rule[:52], "true_depth": depth, "asked_at": k,
                          "adequate": adequate, "worst_margin": round(worst, 2),
                          "wrong_patterns": wrong,
                          "accepted": worst >= machine.TABLE_FLOOR})
    scored = [(case["worst_margin"], not case["adequate"]) for case in cases]
    return {"cases": cases, "pairs": len(cases),
            "floor": machine.TABLE_FLOOR,
            "as_intended": sum(1 for case in cases
                               if case["accepted"] == case["adequate"]),
            "sweep": cut(scored, reject_above=False)}


def say_depth(found: dict) -> None:
    print(f"\ntable margin as a depth check (floor {found['floor']}) - "
          f"{found['as_intended']}/{found['pairs']} as intended")
    print(f"  {'depth':>5} {'asked':>5} {'adequate':>9} {'margin':>7} {'verdict':>9}")
    for case in found["cases"]:
        verdict = "accept" if case["accepted"] else "reject"
        mark = " " if case["accepted"] == case["adequate"] else "<"
        extra = "" if case["wrong_patterns"] is None else f"  {case['wrong_patterns']} wrong"
        print(f"  {case['true_depth']:>5} {case['asked_at']:>5} "
              f"{str(case['adequate']):>9} {case['worst_margin']:>7.2f} "
              f"{verdict:>9}{mark}{extra}")
        print(f"        {case['rule']}")
    say_sweep(found["sweep"], found["floor"])


def say_spec(found: dict) -> None:
    print("\nundecided detector "
          f"(floor {spec.UNDECIDED_FLOOR}) - {found['as_intended']}/{found['specs']} as intended")
    print(f"  {'group':<22} {'specs':>5} {'stopped':>8}  peak undecided")
    for group in found["groups"]:
        print(f"  {group['group']:<22} {group['specs']:>5} {group['stopped']:>8}  "
              f"{group['peak_low']:.2f} - {group['peak_high']:.2f}")
    for group in found["groups"]:
        for case in group["cases"]:
            if not case["as_intended"]:
                miss = "FALSE ALARM" if case["stopped"] else "MISSED"
                print(f"  {miss}: {case['rule']}  peak {case['peak']:.2f}")
    say_sweep(found["sweep"], found.get("floor"))


def say_sweep(found: dict, floor: float | None = None) -> None:
    print(f"\nwhat every floor would have done over these {found['specs']} specs:")
    print(f"  {'floor':>6} {'false alarms':>13} {'missed':>7} {'wrong':>6}")
    for row in found["rows"]:
        here = "  <- in use" if floor is not None and abs(row["floor"] - floor) < 1e-9 else ""
        best = " *" if row["wrong"] == found["fewest_wrong"] else "  "
        print(f"  {row['floor']:>6.2f} {row['false_alarms']:>13} {row['missed']:>7} "
              f"{row['wrong']:>6}{best}{here}")
    if found["separable"]:
        print(f"  a clean cut exists at {found['best_floors']}: these specs separate")
    else:
        print(f"  no clean cut: the best any floor manages is {found['fewest_wrong']} wrong, "
              f"at {found['best_floors']}")
        print("  the populations interleave, so pick by which error you would rather have")


def say_table(found: dict) -> None:
    print(f"\ntruth tables - {found['exact']}/{found['specs']} exact")
    for case in found["cases"]:
        mark = "ok " if case["exact"] else "WRONG"
        print(f"  {mark} {case['rule'][:52]}")
        if not case["exact"]:
            print(f"       rows {case['wrong_rows']}: got {case['got']} want {case['want']}")


def say_trace(found: dict) -> None:
    print(f"\ntemporal rules - routed {found['routed']}/{found['rules']}, "
          f"{found['steps_right']} steps labelled right")
    for case in found["cases"]:
        mark = "ok " if case["as_intended"] else "WRONG"
        line = (f"  {mark} {case['routed']:<10} windowed {case['windowed']:.2f} "
                f"accum {case['accumulates']:.2f} span {str(case['span']):>4} "
                f"(conf {case['span_confidence']:.2f})")
        if "steps" in case:
            line += (f"  {case['steps_right']}/{case['steps']} steps "
                     f"from {case['questions']} questions")
        print(line)
        print(f"       {case['rule']}")


# Module level so the CLI choices and the registry cannot drift apart.
SUITES = {"spec": (bench_spec, say_spec), "table": (bench_table, say_table),
          "trace": (bench_trace, say_trace), "depth": (bench_depth, say_depth)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="bench", description=__doc__.split("\n")[0])
    parser.add_argument("--suite", default="all", choices=["all", *SUITES])
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--cases", help="JSON file of your own specs; see load_cases")
    parser.add_argument("--save", help="write the full report here")
    parser.add_argument("--sweep-from", dest="sweep_from",
                        help="re-sweep a saved report; no key and no requests")
    args = parser.parse_args(argv)

    if args.sweep_from:
        held = json.loads(Path(args.sweep_from).read_text(encoding="utf-8"))
        found = held.get("spec", held)
        again = sweep(found)
        if args.json:
            print(json.dumps(again, indent=2))
        else:
            print(f"re-swept {args.sweep_from} (model {held.get('model', 'unrecorded')})")
            say_sweep(again, found.get("floor"))
        return 0

    if not os.environ.get("TYPESAFE_API_KEY"):
        print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
        return 2

    wanted = list(SUITES) if args.suite == "all" else [args.suite]

    report: dict = {}
    try:
        with TypeSafeClient() as raw:
            client = Metered(raw)
            groups = load_cases(args.cases) if args.cases else None
            for name in wanted:
                run, _ = SUITES[name]
                report[name] = run(client, groups) if name == "spec" else run(client)
    except TypeSafeAPIError as error:
        print(f"TypeSafe error {error.status} (request {error.request_id})", file=sys.stderr)
        return 2

    report["model"] = client.model
    report["cost"] = {"requests": client.calls, "input_tokens": client.input,
                      "output_tokens": client.output}

    if args.save:
        Path(args.save).write_text(json.dumps(report, indent=2), encoding="utf-8")

    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(f"model {client.model}")
    for name in wanted:
        SUITES[name][1](report[name])
    print(f"\n{client.calls} requests, {client.input} input tokens, "
          f"{client.output} output tokens")
    print("These are the numbers front/README.md quotes. If they have moved, the "
          "README is what needs editing.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
