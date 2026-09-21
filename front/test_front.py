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
import sys
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
