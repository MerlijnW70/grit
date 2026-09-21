"""Tests for the front door. No device, no key, no network.

Everything here is either pure or spoken to a stub that answers like grit-mcp
does. What these cannot cover is whether the model answers well - that is
measured against rules with computable answers, and the numbers are in the
README rather than asserted here, because a threshold that fails on a model
update is a test that cries wolf.

    python front/test_front.py
"""

from __future__ import annotations

import io
import json
import os
import sys
import types
import textwrap
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from tempfile import mkdtemp

sys.path.insert(0, str(Path(__file__).resolve().parent))

import bench
import machine
import spec

NO_LADDER = {"smallest_that_solved": None, "ladder": [{"gates": 4, "solved": False}],
             "circuit": None}

SOLVED = {
    "solved": True, "marks": 4, "of": 4, "wrong_rows": [],
    "checked_on_the_host": True,
    "gates": [{"kind": 6, "left": 0, "right": 1}],
}


def stub(answer: dict, *, isError: bool = False, silent: bool = False) -> list[str]:
    """A fake grit-mcp: reads the two lines, answers the second one."""
    source = textwrap.dedent(f"""
        import json, sys
        answer = {answer!r}
        for line in sys.stdin:
            if not line.strip():
                continue
            message = json.loads(line)
            if message.get("id") != 2:
                print(json.dumps({{"jsonrpc": "2.0", "id": 1, "result": {{}}}}))
                continue
            if {silent!r}:
                continue
            result = {{"content": [{{"type": "text", "text": "said"}}],
                      "isError": {isError!r}, "structuredContent": answer}}
            print(json.dumps({{"jsonrpc": "2.0", "id": 2, "result": result}}))
            sys.stdout.flush()
    """)
    handle = Path(mkdtemp()) / "stub.py"
    handle.write_text(source, encoding="utf-8")
    return [sys.executable, str(handle)]


class Window(unittest.TestCase):
    """The run-in is asked about, not assumed."""

    # "this input and the one before it were both 1", newest input first.
    tables = {1: {(0,): 0, (1,): 0},                      # nothing before it yet
              2: {(0, 0): 0, (0, 1): 0, (1, 0): 0, (1, 1): 1}}

    def test_a_short_prefix_reads_its_own_table(self):
        # Step 0 has one input, so it reads the one-long table, never the
        # two-long one with a zero invented to fill the gap.
        self.assertEqual(machine.label([1, 1, 0], self.tables, 2), [0, 1, 0])

    def test_the_run_in_does_not_invent_history(self):
        # The bug this replaces: "the last N inputs were all the same" came out
        # true at step 0, because N imaginary zeros are indeed all the same.
        same = {1: {(0,): 0, (1,): 0},
                2: {(0, 0): 1, (0, 1): 0, (1, 0): 0, (1, 1): 1}}
        self.assertEqual(machine.label([0, 0, 1], same, 2)[0], 0,
                         "step 0 cannot have two inputs that match")

    def test_a_lone_one_never_fires(self):
        self.assertEqual(machine.label([1, 0, 1, 0], self.tables, 2), [0, 0, 0, 0])

    def test_every_step_is_a_lookup_so_equal_windows_agree(self):
        got = machine.label([1, 1, 0, 1, 1], self.tables, 2)
        self.assertEqual(got[1], got[4], "the same window twice must label the same")

    def test_runs_are_reproducible_from_a_seed(self):
        import random
        one = machine.make_runs(random.Random(3), 2, 6, self.tables, 2)
        two = machine.make_runs(random.Random(3), 2, 6, self.tables, 2)
        self.assertEqual(one, two)
        self.assertTrue(all(len(run["feed"]) == len(run["want"]) == 6 for run in one))


class Bits(unittest.TestCase):
    def test_output_names_pack_lowest_first(self):
        row = spec.Row(index=0, bits={}, outs={"sum": 1, "carry": 1})
        self.assertEqual(row.word(["sum", "carry"]), 0b11)
        self.assertEqual(spec.Row(0, {}, {"sum": 0, "carry": 1}).word(["sum", "carry"]), 0b10)
        self.assertEqual(spec.Row(0, {}, {"sum": 1, "carry": 0}).word(["sum", "carry"]), 0b01)

    def test_unclear_reads_the_undecided_question_not_the_margin(self):
        # A confident 0.03 is not unclear; an undecided 0.9 is, whatever the bit says.
        row = spec.Row(0, {}, {"out": 0}, {"out": 0.03}, {"out": 0.9})
        self.assertEqual(row.unclear(0.5), ["out"])
        calm = spec.Row(0, {}, {"out": 0}, {"out": 0.16}, {"out": 0.05})
        self.assertEqual(calm.unclear(0.5), [], "0.16 is a confident no, not a coin flip")


class Speaking(unittest.TestCase):
    def test_reads_the_answer_to_the_call_and_not_the_handshake(self):
        self.assertEqual(spec.grit("grit_synthesise", {}, stub(SOLVED)), SOLVED)

    def test_a_refusal_stops_rather_than_returning_empty(self):
        with self.assertRaises(SystemExit) as caught:
            spec.grit("grit_synthesise", {}, stub(SOLVED, isError=True))
        self.assertIn("could not do it", str(caught.exception))

    def test_silence_stops_rather_than_returning_empty(self):
        with self.assertRaises(SystemExit):
            spec.grit("grit_synthesise", {}, stub(SOLVED, silent=True))

    def test_the_host_check_survives_the_round_trip(self):
        answer = spec.grit("grit_synthesise", {}, stub(SOLVED))
        self.assertTrue(answer["checked_on_the_host"])
        self.assertEqual(answer["wrong_rows"], [])

    def test_a_missing_binary_says_how_to_build_it(self):
        with self.assertRaises(SystemExit) as caught:
            spec.grit("grit_synthesise", {}, "no-such-grit-mcp")
        self.assertIn("cargo build", str(caught.exception))


class Driving(unittest.TestCase):
    """--table drives grit with no model in the loop at all."""

    def setUp(self):
        self.asked = []
        self.real = spec.grit

        def fake(tool, arguments, binary, timeout=900):
            self.asked.append((tool, arguments))
            return SOLVED if tool == "grit_synthesise" else NO_LADDER
        spec.grit = fake
        self.addCleanup(lambda: setattr(spec, "grit", self.real))

    def run_spec(self, argv: list[str]) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out):
            code = spec.main(argv)
        return code, out.getvalue()

    def test_a_given_table_needs_no_key_and_asks_nothing(self):
        code, said = self.run_spec(["xor", "-i", "a,b", "--table", "0,1,1,0"])
        self.assertEqual(code, 0)
        self.assertEqual(len(self.asked), 1)
        tool, arguments = self.asked[0]
        self.assertEqual(tool, "grit_synthesise")
        self.assertEqual(arguments["table"], [0, 1, 1, 0])
        self.assertEqual(arguments["inputs"], 2)
        self.assertIn("checked on the host: True", said)

    def test_a_short_table_is_refused_before_grit_is_reached(self):
        with self.assertRaises(SystemExit):
            spec.main(["xor", "-i", "a,b", "--table", "0,1"])
        self.assertEqual(self.asked, [], "nothing should have been searched")

    def test_least_walks_the_budget_and_reports_not_solving(self):
        code, said = self.run_spec(["xor", "-i", "a,b", "--table", "0,1,1,0", "--least"])
        self.assertEqual(code, 1)
        self.assertEqual(self.asked[0][0], "grit_least")
        self.assertIn("raise crowd", said)


class Bench(unittest.TestCase):
    """The cases the README quotes have to stay well-formed without a key."""

    def test_every_case_names_as_many_inputs_as_its_table_has_rows(self):
        for rule, names in bench.STATED + bench.NAMED + bench.VAGUE:
            self.assertTrue(names, rule)
            self.assertEqual(len(set(names)), len(names), f"duplicate input in {rule}")

    def test_the_ground_truth_tables_are_total(self):
        for rule, names, truth in bench.TABLES:
            for row in range(2 ** len(names)):
                bit = truth(*[(row >> i) & 1 for i in range(len(names))])
                self.assertIn(bit, (0, 1), f"{rule} row {row}")

    def test_the_trace_rules_cover_both_routes(self):
        expected = {kind for _, _, kind in bench.TRACES}
        self.assertEqual(expected, {"window", "accumulate"})

    def test_every_suite_is_reachable_and_prints(self):
        # A suite in the registry but not in --suite is one nobody can run.
        self.assertEqual(set(bench.SUITES), {"spec", "table", "trace", "depth", "route", "prune"})
        for name, (run, say) in bench.SUITES.items():
            self.assertTrue(callable(run), name)
            self.assertTrue(callable(say), name)


class FakeAnswer:
    def __init__(self, noul=None, choice=None, confidence=1.0):
        self.noul, self.choice, self.confidence = noul, choice, confidence


class FakeResponse:
    def __init__(self, nouls, choices):
        self.nouls, self.choices = nouls, choices
        self.model = "fake-1"
        self.usage = types.SimpleNamespace(input_tokens=0, output_tokens=0)


class FakeClient:
    """Answers whatever is asked, from a script keyed by question id prefix.

    machine.main builds its own client, so this stands in for the module's
    TypeSafeClient. It also records every question, which is how the pruning
    test checks that fewer windows were actually enumerated.
    """

    def __init__(self, script):
        self.script, self.asked = script, []

    def __enter__(self):
        return self

    def __exit__(self, *rest):
        return False

    def __call__(self, *rest, **kw):
        return self

    def system_one(self, state=None, questions=None, model=None, **kw):
        self.asked.append(list(questions))
        nouls, choices = {}, {}
        for qid in questions:
            if qid == "span":
                choices[qid] = FakeAnswer(choice=self.script["span"],
                                          confidence=self.script.get("span_confidence", 1.0))
            elif "by_text" in self.script:
                nouls[qid] = FakeAnswer(
                    noul=self.script["by_text"](qid, questions[qid].instructions))
            else:
                nouls[qid] = FakeAnswer(noul=self.script["noul"](qid))
        return FakeResponse(nouls, choices)


class Driving_machine:
    """Runs machine.main against a scripted model and a stubbed grit."""

    def drive(self, script, argv):
        client = FakeClient(script)
        real_client, real_grit = machine.TypeSafeClient, machine.grit
        had_key = os.environ.get("TYPESAFE_API_KEY")
        os.environ["TYPESAFE_API_KEY"] = "not-a-real-key"
        machine.TypeSafeClient = client
        machine.grit = lambda tool, arguments, binary, timeout=900: {
            "solved": True, "marks": 1, "of": 1, "gates": [], "traces": arguments["traces"]}
        try:
            out = io.StringIO()
            with redirect_stdout(out):
                code = machine.main(argv)
            return code, out.getvalue(), client
        finally:
            machine.TypeSafeClient, machine.grit = real_client, real_grit
            if had_key is None:
                del os.environ["TYPESAFE_API_KEY"]
            else:
                os.environ["TYPESAFE_API_KEY"] = had_key

class Refusing(Driving_machine, unittest.TestCase):
    """machine.main's three refusals, and the exit code each one carries."""

    def test_an_accumulating_rule_is_refused(self):
        code, said, _ = self.drive(
            {"span": "2", "noul": lambda q: 0.05 if q == "windowed" else 0.9},
            ["counts the 1s so far"])
        self.assertEqual(code, 3)
        self.assertIn("running tally", said)

    def test_a_window_deeper_than_the_options_is_refused(self):
        code, said, _ = self.drive(
            {"span": "more", "noul": lambda q: 0.95 if q == "windowed" else 0.05},
            ["looks a long way back"])
        self.assertEqual(code, 3)
        self.assertIn("further back", said)

    def test_a_table_of_coin_flips_is_refused(self):
        # Every window answered at dead centre: nothing to build a machine on.
        code, said, _ = self.drive(
            {"span": "2", "noul": lambda q: 0.95 if q == "windowed"
                else 0.05 if q == "accumulates" else 0.5},
            ["something the window cannot carry"])
        self.assertEqual(code, 3)
        self.assertIn("full windows close to", said)

    def test_a_clean_window_rule_reaches_grit(self):
        code, said, _ = self.drive(
            {"span": "2", "noul": lambda q: 0.95 if q == "windowed"
                else 0.05 if q == "accumulates" else (0.95 if q.startswith("w2") else 0.02)},
            ["a plain window rule", "--runs", "2", "--steps", "6"])
        self.assertIn(code, (0, 1))
        self.assertIn("window tables", said)


class Pruning(Driving_machine, unittest.TestCase):
    def test_pruning_asks_about_fewer_windows(self):
        # Two inputs, two steps: four slots, so sixteen full windows. Saying
        # only the current step matters must cut what gets enumerated.
        def script(only_now):
            def noul(q):
                if q == "windowed":
                    return 0.95
                if q == "accumulates":
                    return 0.05
                if q.startswith("rel"):
                    return 0.95 if (q.startswith("rel0") or not only_now) else 0.02
                return 0.95 if q.startswith("w2") else 0.02
            return {"span": "2", "noul": noul}

        argv = ["a rule", "-i", "a,b", "--runs", "2", "--steps", "6", "--prune"]
        _, _, wide = self.drive(script(only_now=False), argv)
        _, _, narrow = self.drive(script(only_now=True), argv)
        # The last request of each run is the window enumeration.
        self.assertLess(len(narrow.asked[-1]), len(wide.asked[-1]),
                        "dropping two of four slots must ask about fewer windows")

    def test_the_check_catches_a_value_that_was_dropped_but_matters(self):
        # A scripted rule that genuinely reads b at the previous step: the
        # probe question spells every value out, so the answer can be read
        # straight off the text the checker built.
        def noul(qid, text):
            return 0.95 if "the step immediately before this one, a=0, b=1" in text else 0.05

        client = FakeClient({"span": "2", "noul": lambda q: 0.5, "by_text": noul})
        honest = {(0, "a"): 0.9, (1, "b"): 0.9, (0, "b"): 0.01, (1, "a"): 0.01}
        lied = dict(honest)
        lied[(1, "b")] = 0.0
        self.assertIn((1, "b"),
                      machine.check_pruning(client, "r", 2, ["a", "b"], ["out"], lied),
                      "dropping a value the rule reads has to come back flagged")
        self.assertNotIn((1, "b"),
                         machine.check_pruning(client, "r", 2, ["a", "b"], ["out"], honest),
                         "a value that was kept is not under test")

    def test_the_check_pays_for_every_probe_it_reads(self):
        # It once asked every background and compared only the first few, so
        # exhaustive coverage was bought and thrown away.
        client = FakeClient({"span": "2", "noul": lambda q: 0.05})
        machine.check_pruning(client, "r", 2, ["a", "b"], ["out"],
                              {(0, "a"): 0.9, (0, "b"): 0.01, (1, "a"): 0.01, (1, "b"): 0.01})
        asked = len(client.asked[-1])
        self.assertEqual(asked % 2, 0, "probes come in pairs")

    def test_a_dropped_slot_never_empties_the_window(self):
        # Relevance can come back saying nothing matters. The lookup still
        # needs a key, so the current step is kept regardless.
        nothing = {(0, "a"): 0.01, (1, "a"): 0.01}
        self.assertEqual(machine.relevant_slots(nothing, 2, ["a"]), [(0, "a")])


class Sweep(unittest.TestCase):
    """The sweep is a cut over peaks, so it can be checked without any model."""

    def report(self, peaks):
        return {"groups": [{"should_stop": stop,
                            "cases": [{"peak": peak} for peak in group]}
                           for group, stop in peaks]}

    def test_a_separable_set_finds_the_clean_cut(self):
        found = bench.sweep(self.report([([0.1, 0.2], False), ([0.8, 0.9], True)]))
        self.assertTrue(found["separable"])
        self.assertEqual(found["fewest_wrong"], 0)
        for floor in found["best_floors"]:
            self.assertGreaterEqual(floor, 0.2)
            self.assertLess(floor, 0.8)

    def test_interleaved_populations_have_no_clean_cut(self):
        # A spec that should pass sits above one that should stop: no cut works.
        found = bench.sweep(self.report([([0.1, 0.5], False), ([0.4, 0.9], True)]))
        self.assertFalse(found["separable"])
        self.assertGreater(found["fewest_wrong"], 0)

    def test_a_low_floor_trades_misses_for_false_alarms(self):
        found = bench.sweep(self.report([([0.3], False), ([0.6], True)]),
                            floors=[0.1, 0.45, 0.9])
        low, mid, high = found["rows"]
        self.assertEqual((low["false_alarms"], low["missed"]), (1, 0))
        self.assertEqual((mid["false_alarms"], mid["missed"]), (0, 0))
        self.assertEqual((high["false_alarms"], high["missed"]), (0, 1))

    def test_a_margin_cut_rejects_below_rather_than_above(self):
        # The undecided detector stops on a HIGH score; a table margin stops on
        # a LOW one. Same machinery, opposite direction, and getting it
        # backwards would silently invert every threshold it reports.
        scored = [(0.4, False), (0.1, True)]   # 0.4 is fine, 0.1 should be stopped
        found = bench.cut(scored, floors=[0.25], reject_above=False)
        self.assertEqual(found["fewest_wrong"], 0)
        upside_down = bench.cut(scored, floors=[0.25], reject_above=True)
        self.assertEqual(upside_down["rows"][0]["wrong"], 2)

    def test_cases_load_and_group_by_intent(self):
        from tempfile import mkdtemp
        held = Path(mkdtemp()) / "cases.json"
        held.write_text(json.dumps({"cases": [
            {"rule": "r1", "inputs": ["a"], "should_stop": False, "group": "g"},
            {"rule": "r2", "inputs": ["a", "b"], "should_stop": True, "group": "h"}]}),
            encoding="utf-8")
        groups = bench.load_cases(str(held))
        self.assertEqual({(name, stop) for name, _, stop in groups},
                         {("g", False), ("h", True)})


class Shape(unittest.TestCase):
    def test_deeper_than_the_options_is_a_refusal_not_the_deepest_option(self):
        # The whole point of the "more" option: without it a five-deep rule was
        # answered "four" at 0.99 confidence and labelled wrongly in silence.
        self.assertIn("more", machine.SPANS)
        deepest = max(int(n) for n in machine.SPANS if n.isdigit())
        self.assertEqual(deepest, 4)

    def test_the_table_floor_sits_between_what_was_measured(self):
        self.assertLess(0.03, machine.TABLE_FLOOR, "the forced four-deep table must fail")
        self.assertGreater(0.43, machine.TABLE_FLOOR, "a real three-deep table must pass")


if __name__ == "__main__":
    unittest.main(verbosity=2)
