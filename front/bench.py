"""Re-measure the numbers the README quotes.

Every threshold in this front door was set from here, and none of the numbers
are written down anywhere else. They are not asserted in test_front.py on
purpose: a threshold pinned in a test breaks on a model update rather than on a
bug, and a number copied into prose is wrong by the next release. So the cases
live here and the measurement is run on demand, against the model it names.

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


# The routing boundary, stressed from both sides. A rule that says "count" but
# only ever looks at a few inputs is a window; a rule that sounds local but has
# to remember whether something ever happened is not.
ROUTES = [
    # Windows wearing an accumulator's words.
    ("output 1 at a step exactly when at least two of the last three inputs, "
     "ending at that step, were 1", "window"),
    ("output 1 at a step exactly when an odd number of the last two inputs, "
     "ending at that step, were 1", "window"),
    ("output 1 at a step exactly when the last four inputs, ending at that step, "
     "add up to more than two", "window"),
    ("output 1 at a step exactly when none of the last three inputs were 1", "window"),
    # Plain windows.
    ("output 1 at a step exactly when the input at that step is 1 and the one "
     "before it was 0", "window"),
    # Accumulators wearing a window's words.
    ("output 1 at a step exactly when a 1 has arrived at some point, at that step "
     "or any earlier one, and once that has happened it stays 1 forever", "accumulate"),
    ("output 1 at a step exactly when three 1s have arrived in a row at some point "
     "in the run so far, and it stays 1 from then on", "accumulate"),
    ("output 1 at a step exactly when more 1s than 0s have arrived so far", "accumulate"),
    # Plain accumulators.
    ("output 1 at a step exactly when the number of 1s so far is a multiple of "
     "three", "accumulate"),
    ("output 1 at a step exactly when an odd number of 1s have arrived so far",
     "accumulate"),
]


# Rules with a known relevant set: exactly which values in the window decide
# the output. Dropping one of these is the failure that matters; keeping a
# spare only costs questions.
PRUNES = [
    ("output 1 at a step exactly when a is 1 at that step and b was 1 at the step "
     "before it", 2, ["a", "b"], {(0, "a"), (1, "b")}),
    ("output 1 at a step exactly when b is 1 at that step, whatever a does",
     2, ["a", "b"], {(0, "b")}),
    ("output 1 at a step exactly when the last three inputs, ending at that step, "
     "were 1 then 0 then 1", 3, ["x"], {(0, "x"), (1, "x"), (2, "x")}),
    ("output 1 at a step exactly when a was 1 two steps before this one, no matter "
     "what has happened since", 3, ["a", "b"], {(2, "a")}),
    ("output 1 at a step exactly when a and b are both 1 at this step",
     2, ["a", "b"], {(0, "a"), (0, "b")}),
]


# Wide combinational specs where the answer, and which inputs decide it, are
# both known. The table is still every row: what pruning changes is how many
# cases get asked about before the rest are projected.
WIDE = [
    ("output 1 when d and f are both 1, ignoring every other input",
     ["a", "b", "c", "d", "e", "f"], {"d", "f"},
     lambda r: int(((r >> 3) & 1) and ((r >> 5) & 1))),
    ("output 1 when exactly one of a and b is 1; c, d and e are irrelevant",
     ["a", "b", "c", "d", "e"], {"a", "b"},
     lambda r: (r & 1) ^ ((r >> 1) & 1)),
    ("output 1 when at least two of a, b and c are 1, whatever d does",
     ["a", "b", "c", "d"], {"a", "b", "c"},
     lambda r: int((r & 1) + ((r >> 1) & 1) + ((r >> 2) & 1) >= 2)),
    ("output 1 when a is 1", ["a", "b", "c", "d"], {"a"}, lambda r: r & 1),
]


# Specs of the kind someone would actually write, rather than three-input toys.
# Bit 0 of the row index is the first named input. Ground truth is computed
# from the row index so the table can be marked rather than eyeballed.
def _bits(r, n):
    return [(r >> i) & 1 for i in range(n)]


HARD = [
    ("output 1 when the two-bit number a (a1 is its high bit, a0 its low bit) is "
     "strictly greater than the two-bit number b (b1 high, b0 low)",
     ["a0", "a1", "b0", "b1"], ["out"],
     lambda r: [int((((r >> 1) & 1) * 2 + (r & 1)) >
                    (((r >> 3) & 1) * 2 + ((r >> 2) & 1)))]),
    ("the four inputs are the bits of a number from 0 to 15, where n0 is the "
     "lowest bit and n3 the highest; output 1 when that number is 9 or less",
     ["n0", "n1", "n2", "n3"], ["out"],
     lambda r: [int(r <= 9)]),
    ("the three inputs are the bits of a number from 0 to 7, where n0 is the "
     "lowest bit and n2 the highest; output 1 when that number is prime",
     ["n0", "n1", "n2"], ["out"],
     lambda r: [int(r in (2, 3, 5, 7))]),
    ("output 1 when at least three of the five inputs are 1",
     ["a", "b", "c", "d", "e"], ["out"],
     lambda r: [int(sum(_bits(r, 5)) >= 3)]),
    ("add the two-bit number a (a1 high, a0 low) to the two-bit number b (b1 "
     "high, b0 low); s0 is the low bit of the sum, s1 the next bit, s2 the carry out",
     ["a0", "a1", "b0", "b1"], ["s0", "s1", "s2"],
     lambda r: [(((((r >> 1) & 1) * 2 + (r & 1)) + ((((r >> 3) & 1) * 2 + ((r >> 2) & 1)))) >> i) & 1
                for i in range(3)]),
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
        kind = "window" if shape["route_window"] > machine.WINDOW_FLOOR else "accumulate"
        ok = kind == expected
        routed += ok
        case = {"rule": rule[:56], "expected": expected, "routed": kind,
                "windowed": round(shape["windowed"], 2),
                "accumulates": round(shape["accumulates"], 2),
                "span": shape["span"], "span_confidence": round(shape["span_confidence"], 2),
                "as_intended": ok}
        if kind == "window" and ok and shape["span"]:
            windows += 1
            tables, worst, _ = machine.window_table(client, rule, shape["span"], ["x"], ["out"])
            steps = wrong = 0
            for feed in feeds:
                got = machine.label(feed, tables, shape["span"])
                want = [truth(feed[:t + 1]) for t in range(len(feed))]
                steps += len(feed)
                wrong += sum(a != b for a, b in zip(got, want))
            labelled_right += steps - wrong
            case.update({"steps": steps, "steps_right": steps - wrong,
                         "questions": 2 ** (shape["span"] + 1) - 2,
                         "worst_margin": round(worst, 2)})
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
            table, worst, run_in = machine.window_table(client, rule, k, ["x"], ["out"])
            # At an adequate depth the whole table is checkable against truth.
            wrong = None
            if adequate:
                # Every window the run can present, run-in included: the short
                # ones are where zero-padding used to quietly invent history.
                wrong = sum(1 for length in table
                            for pattern, bit in table[length].items()
                            if truth(list(reversed(pattern))) != bit)
            cases.append({"rule": rule[:52], "true_depth": depth, "asked_at": k,
                          "adequate": adequate, "worst_margin": round(worst, 2),
                          "run_in_margin": round(run_in, 2),
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


def bench_route(client) -> dict:
    """Does the window/accumulate routing hold at the boundary, and on what?

    Two predictors are scored on the same answers. `both` is what the tool
    uses: a window needs `windowed` over the floor AND `accumulates` under its
    mirror, which is min(windowed, 1 - accumulates) against one number, and at
    0.50 that is exactly the current rule. `gap` is the cheaper idea of just
    taking whichever noul is larger, scored as (windowed - accumulates + 1) / 2.
    """
    cases = []
    for rule, expected in ROUTES:
        shape = machine.read_shape(client, rule)
        both = min(shape["windowed"], 1 - shape["accumulates"])
        # The same decision asked as one comparison instead of two separate
        # yes-or-no questions. A route is a choice between two accounts of the
        # rule, and a Choice is the primitive that weighs options against each
        # other rather than each on its own.
        asked = shape["route_window"]
        gap = (shape["windowed"] - shape["accumulates"] + 1) / 2
        routed = ("window" if both > machine.WINDOW_FLOOR else "accumulate")
        cases.append({"rule": rule[:58], "expected": expected, "routed": routed,
                      "windowed": round(shape["windowed"], 2),
                      "accumulates": round(shape["accumulates"], 2),
                      "both": round(both, 2), "gap": round(gap, 2),
                      "asked": round(asked, 2), "route": shape["route"],
                      "route_right": (shape["route"] == "window") == (expected == "window"),
                      "span_choice": shape["span_choice"],
                      "as_intended": routed == expected})
    accumulating = [case["expected"] == "accumulate" for case in cases]
    return {"rules": len(cases), "cases": cases, "floor": machine.WINDOW_FLOOR,
            "as_intended": sum(1 for case in cases if case["as_intended"]),
            "sweep": cut([(case["both"], stop)
                          for case, stop in zip(cases, accumulating)],
                         reject_above=False),
            "sweep_gap": cut([(case["gap"], stop)
                              for case, stop in zip(cases, accumulating)],
                             reject_above=False),
            "route_right": sum(1 for case in cases if case["route_right"]),
            "sweep_asked": cut([(case["asked"], stop)
                                for case, stop in zip(cases, accumulating)],
                               reject_above=False)}


def say_route(found: dict) -> None:
    print(f"\nrouting at the boundary (floor {found['floor']}) - "
          f"{found['as_intended']}/{found['rules']} as intended")
    print(f"  {'want':>10} {'got':>10} {'wndw':>5} {'accum':>6} {'both':>5} {'span':>5}")
    for case in found["cases"]:
        mark = " " if case["as_intended"] else "<"
        print(f"  {case['expected']:>10} {case['routed']:>10} {case['windowed']:>5.2f} "
              f"{case['accumulates']:>6.2f} {case['both']:>5.2f} "
              f"{str(case['span_choice']):>5}{mark}")
        print(f"        {case['rule']}")
    print("\npredictor the tool uses - min(windowed, 1 - accumulates):")
    say_sweep(found["sweep"], found["floor"])
    print("\ncheaper predictor - whichever noul is larger:")
    say_sweep(found["sweep_gap"])


def bench_prune(client) -> dict:
    """Does asking what the rule reads cut the enumeration without losing a value?

    The two errors are not the same size, so they are counted apart. A dropped
    value flattens the rule and nothing downstream sees it; a kept spare costs
    questions. Savings are reported against the full enumeration.
    """
    cases = []
    for rule, k, names, want in PRUNES:
        relevance = machine.read_relevance(client, rule, k, names)
        kept = set(machine.relevant_slots(relevance, k, names))
        dropped = sorted(want - kept)
        spare = sorted(kept - want)
        full = sum(2 ** (n * len(names)) for n in range(1, k + 1))
        pruned = sum(2 ** len(machine.relevant_slots(relevance, n, names))
                     for n in range(1, k + 1))
        # Does the check pass an honest prune, and catch a sabotaged one? Each
        # rule is asked again with one value it really needs marked irrelevant.
        clean = machine.check_pruning(client, rule, k, names, ["out"], relevance)
        sabotage = sorted(want)[0]
        lied = dict(relevance)
        lied[sabotage] = 0.0
        caught = machine.check_pruning(client, rule, k, names, ["out"], lied)
        cases.append({"rule": rule[:54], "kept": sorted(kept), "want": sorted(want),
                      "dropped": dropped, "spare": spare,
                      "full_windows": full, "pruned_windows": pruned,
                      "saved": round(1 - pruned / full, 2),
                      "safe": not dropped,
                      "check_false_alarm": bool(clean),
                      "sabotage": list(sabotage),
                      "sabotage_caught": sabotage in caught})
    return {"rules": len(cases), "cases": cases,
            "checked_clean": sum(1 for case in cases if not case["check_false_alarm"]),
            "sabotage_caught": sum(1 for case in cases if case["sabotage_caught"]),
            "safe": sum(1 for case in cases if case["safe"]),
            "exact": sum(1 for case in cases if case["safe"] and not case["spare"]),
            "windows_full": sum(case["full_windows"] for case in cases),
            "windows_pruned": sum(case["pruned_windows"] for case in cases)}


def say_prune(found: dict) -> None:
    saved = 1 - found["windows_pruned"] / found["windows_full"]
    print(f"\nrelevance pruning - {found['safe']}/{found['rules']} lost nothing, "
          f"{found['exact']}/{found['rules']} exactly right, "
          f"{found['windows_full']} windows down to {found['windows_pruned']} "
          f"({saved:.0%} fewer)")
    print(f"  the check: passes {found['checked_clean']}/{found['rules']} honest "
          f"prunes, catches {found['sabotage_caught']}/{found['rules']} sabotaged ones")
    for case in found["cases"]:
        mark = "ok  " if case["safe"] else "LOST"
        note = ""
        if case["check_false_alarm"]:
            note += "  check flagged an honest prune"
        if not case["sabotage_caught"]:
            note += "  check MISSED a sabotaged " + f"{case['sabotage'][1]}@-{case['sabotage'][0]}"
        if case["dropped"]:
            note += "  dropped " + ", ".join(f"{n}@-{l}" for l, n in case["dropped"])
        if case["spare"]:
            note += "  kept spare " + ", ".join(f"{n}@-{l}" for l, n in case["spare"])
        print(f"  {mark} {case['full_windows']:>4} -> {case['pruned_windows']:<4}"
              f"{note}")
        print(f"        {case['rule']}")


def bench_wide(client) -> dict:
    """Pruning a wide truth table: does it stay exact, and how much is saved?"""
    cases = []
    for rule, inputs, want, truth in WIDE:
        relevance = spec.read_relevance(client, rule, inputs, ["out"])
        put_back = spec.check_pruning(client, rule, inputs, ["out"], relevance)
        for slot in put_back:
            relevance[slot] = 1.0
        kept = set(spec.relevant_inputs(relevance, "out", inputs))
        rows = spec.read_intent(client, rule, inputs, ["out"], relevance=relevance)
        got = [row.outs["out"] for row in rows]
        wanted = [truth(r) for r in range(2 ** len(inputs))]
        cases.append({"rule": rule[:52], "inputs": len(inputs),
                      "kept": sorted(kept), "want": sorted(want),
                      "dropped": sorted(want - kept),
                      "spare": sorted(kept - want),
                      "put_back": [name for name, _ in put_back],
                      "full_cases": 2 ** len(inputs), "asked": 2 ** len(kept),
                      "exact": got == wanted,
                      "wrong_rows": [i for i, (a, b) in enumerate(zip(got, wanted))
                                     if a != b]})
    return {"specs": len(cases), "cases": cases,
            "exact": sum(1 for case in cases if case["exact"]),
            "safe": sum(1 for case in cases if not case["dropped"]),
            "full": sum(case["full_cases"] for case in cases),
            "asked": sum(case["asked"] for case in cases)}


def say_wide(found: dict) -> None:
    saved = 1 - found["asked"] / found["full"]
    print(f"\nwide specs pruned - {found['exact']}/{found['specs']} tables exact, "
          f"{found['safe']}/{found['specs']} lost nothing, "
          f"{found['full']} cases down to {found['asked']} ({saved:.0%} fewer)")
    for case in found["cases"]:
        mark = "ok  " if case["exact"] and not case["dropped"] else "WRONG"
        note = ""
        if case["dropped"]:
            note += "  dropped " + ", ".join(case["dropped"])
        if case["spare"]:
            note += "  kept spare " + ", ".join(case["spare"])
        if case["put_back"]:
            note += "  check put back " + ", ".join(case["put_back"])
        if case["wrong_rows"]:
            note += f"  {len(case['wrong_rows'])} rows wrong"
        print(f"  {mark} {case['full_cases']:>3} -> {case['asked']:<3}{note}")
        print(f"        {case['rule']}")


def bench_hard(client) -> dict:
    """Specs past the toy range, marked against ground truth."""
    cases = []
    for rule, inputs, outputs, truth in HARD:
        shape = spec.read_shape(client, rule, inputs, outputs)
        rows = spec.read_intent(client, rule, inputs, outputs)
        wrong, stopped = [], []
        for row in rows:
            want = truth(row.index)
            for j, out in enumerate(outputs):
                if row.outs[out] != want[j]:
                    wrong.append((row.index, out))
            stopped += [(row.index, name) for name in row.unclear(spec.UNDECIDED_FLOOR)]
        stopped_by_arithmetic = shape["arithmetic"] > spec.ARITHMETIC_CEILING
        cases.append({"rule": rule[:56], "inputs": len(inputs),
                      "outputs": len(outputs),
                      "bits": 2 ** len(inputs) * len(outputs),
                      "wrong": len(wrong), "first_wrong": wrong[:4],
                      "stopped": len(stopped), "exact": not wrong,
                      "arithmetic": round(shape["arithmetic"], 2),
                      "counting": round(shape["counting"], 2),
                      "guarded": stopped_by_arithmetic,
                      # The guard earns its place by stopping what came back
                      # wrong and letting through what came back exact.
                      "guard_right": stopped_by_arithmetic == bool(wrong)})
    return {"specs": len(cases), "cases": cases,
            "exact": sum(1 for case in cases if case["exact"]),
            "guard_right": sum(1 for case in cases if case["guard_right"]),
            "guard_sweep": cut([(case["arithmetic"], bool(case["wrong"]))
                                for case in cases], reject_above=True),
            "bits": sum(case["bits"] for case in cases),
            "wrong": sum(case["wrong"] for case in cases)}


def say_hard(found: dict) -> None:
    print(f"\nspecs past the toy range - {found['exact']}/{found['specs']} exact, "
          f"{found['bits'] - found['wrong']}/{found['bits']} bits right")
    for case in found["cases"]:
        mark = "ok   " if case["exact"] else "WRONG"
        note = f"  {case['wrong']}/{case['bits']} bits wrong" if case["wrong"] else ""
        if case["stopped"]:
            note += f"  ({case['stopped']} rows called undecided)"
        flag = "STOPPED" if case["guarded"] else "let through"
        missed = "" if case["guard_right"] else "  <- guard wrong"
        print(f"  {mark} {case['inputs']}in {case['outputs']}out{note}")
        print(f"        guard: arithmetic {case['arithmetic']:.2f} -> {flag}{missed}")
        print(f"        {case['rule']}")
        if case["first_wrong"]:
            print("        first wrong: " + ", ".join(f"row {r} `{o}`"
                                                      for r, o in case["first_wrong"]))
    say_sweep(found["guard_sweep"], spec.ARITHMETIC_CEILING)


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
          "trace": (bench_trace, say_trace), "depth": (bench_depth, say_depth),
          "route": (bench_route, say_route), "prune": (bench_prune, say_prune),
          "wide": (bench_wide, say_wide), "hard": (bench_hard, say_hard)}


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
    print("Each suite sweeps its own floor above. Where a sweep says the "
          "populations interleave, the floor in use is a preference rather "
          "than a cut: set it from your own cases with --cases, and re-read "
          "a saved run with --sweep-from.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
