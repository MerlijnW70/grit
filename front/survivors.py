"""Mutation survivors, sorted into test gaps and changes that do nothing.

noha makes the mutants and runs the suite. Neither needs a model: one is an
edit to a line, the other is a test run. What is left over is the expensive
part - a mutant the suite did not kill is either a hole in the tests or a
change that cannot alter what the program does, and telling those apart is a
judgement about what the code means.

The measurement that shaped this one is worth stating, because it is the whole
design. Asked whether a line changing from `> floor` to `>= floor` is
behaviour-preserving, the honest answer is no: it depends on whether any value
ever lands exactly on the floor, and that fact is not on the line. Asked with
that fact supplied - the floor is 0.25 and these are model probabilities, never
exactly 0.25 - the answer is yes. Over eight mutants of real code from this
repo, four of eight without the surrounding facts and eight of eight with them.

So the facts are gathered here, by reading the tree, and the judging is about
what they mean. Nothing about a mutant's fate is inferred: a killed mutant was
killed by a test, and this only ever looks at ones that lived.

    python front/survivors.py --from report.json
    python front/survivors.py --check          # the eight, marked
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

try:
    from typesafe_sdk import Noul, TypeSafeAPIError, TypeSafeClient
except ImportError:
    sys.exit("typesafe-sdk is not installed: pip install typesafe-sdk")

# Above this, the change cannot alter what the program does and the mutant
# survived because there was nothing to catch. Below it, the tests have a hole.
# Set from the marked set below, and it should be reset from your own.
EQUIVALENT = 0.50

ROOT = Path(__file__).resolve().parent.parent


def enclosing(lines: list[str], at: int) -> tuple[int, int]:
    """The function a line sits in, by walking back to its signature.

    Brace counting rather than parsing, which is enough for what this needs and
    wrong in the ways brace counting is always wrong. It is only ever used to
    quote code into a question, so a ragged edge costs a little context and
    never a wrong answer.
    """
    start = at
    while start > 0 and not re.match(r"\s*(pub\s+)?(async\s+)?fn\s+\w+", lines[start]):
        start -= 1
    depth, end = 0, start
    for n in range(start, len(lines)):
        depth += lines[n].count("{") - lines[n].count("}")
        end = n
        if depth <= 0 and n > start and "{" in "".join(lines[start:n + 1]):
            break
    return start, min(end, len(lines) - 1)


def named_on(line: str) -> list[str]:
    """Identifiers a line leans on, worth looking up before judging it."""
    held = re.findall(r"\b[A-Za-z_][A-Za-z0-9_]*\b", line)
    skip = {"let", "mut", "if", "else", "for", "in", "while", "return", "fn",
            "match", "self", "Some", "None", "Ok", "Err", "true", "false",
            "as", "usize", "u32", "i32", "u64", "i64", "f64", "bool", "and",
            "or", "not", "is", "the"}
    return [one for one in dict.fromkeys(held) if one not in skip and len(one) > 2]


def facts_about(path: str, at: int, most: int = 6) -> list[str]:
    """What the tree says about the names on the mutated line.

    Where a constant is defined, and who calls the function it sits in. These
    are the facts a mutant turns on and that its diff does not carry.
    """
    held: list[str] = []
    for name in named_on(Path(ROOT / path).read_text(encoding="utf-8")
                         .splitlines()[at - 1] if (ROOT / path).exists() else ""):
        found = subprocess.run(
            ["git", "grep", "-n", "-E",
             rf"(const|static|let)\s+(mut\s+)?{re.escape(name)}\s*(:|=)", "--", "src"],
            cwd=ROOT, capture_output=True, text=True)
        for line in found.stdout.splitlines()[:2]:
            held.append(line.strip())
        if len(held) >= most:
            break
    return held[:most]


def callers_of(name: str, most: int = 4) -> list[str]:
    found = subprocess.run(["git", "grep", "-n", "-E", rf"\b{re.escape(name)}\s*\(",
                            "--", "src", "tests"], cwd=ROOT,
                           capture_output=True, text=True)
    return [line.strip() for line in found.stdout.splitlines()
            if "fn " + name not in line][:most]


def gathered(one: dict) -> dict:
    """Everything a question about this mutant needs, read off the tree."""
    path, at = one.get("file", ""), int(one.get("line", 0))
    held = (ROOT / path)
    lines = held.read_text(encoding="utf-8").splitlines() if held.exists() else []
    start, end = enclosing(lines, max(at - 1, 0)) if lines else (0, 0)
    signature = lines[start].strip() if lines else ""
    name = re.search(r"fn\s+(\w+)", signature)
    return {
        "file": path, "line": at,
        "before": one.get("before", lines[at - 1].strip() if 0 < at <= len(lines) else ""),
        "after": one.get("after", ""),
        "function": "\n".join(lines[start:end + 1])[:1200],
        "facts": facts_about(path, at),
        "callers": callers_of(name.group(1)) if name else [],
    }


def judge(client, held: list[dict], model=None) -> dict[int, float]:
    """One request. Each mutant is its own question, with its own facts in it."""
    questions = {}
    for n, one in enumerate(held):
        questions[f"m{n}"] = Noul(instructions=(
            f"In `code` a line of {one['file']} was changed. Before:\n"
            f"{one['before']}\nAfter:\n{one['after']}\n"
            "Given everything in `code` - the function it sits in, where the names "
            "on that line are defined, and who calls it - the two versions do the "
            "same thing for every input this program can actually receive. No test "
            "could tell them apart, so nothing is missing from the tests."
        ))
    return client.system_one(
        state={"code": held,
               "note": "a mutant that survived the suite; the question is whether "
                       "it could ever have been killed"},
        questions=questions, model=model)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="survivors", description=__doc__.split("\n")[0])
    parser.add_argument("--from", dest="source",
                        help="noha report --json, or - to read it from stdin")
    parser.add_argument("--check", action="store_true",
                        help="run the marked mutants and score the judging")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--model")
    args = parser.parse_args(argv)

    if not os.environ.get("TYPESAFE_API_KEY"):
        print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
        return 2
    if args.check:
        return marked(args)
    if not args.source:
        parser.error("say --from <report.json> or --check")

    held = json.loads(sys.stdin.read() if args.source == "-"
                      else Path(args.source).read_text(encoding="utf-8"))
    survivors = held.get("prober", {}).get("survivors", [])
    if not survivors:
        print("No survivors in that report. Nothing here has anything to do.")
        return 0

    gathered_all = [gathered(one) for one in survivors]
    try:
        with TypeSafeClient() as client:
            answers = judge(client, gathered_all, args.model)
    except TypeSafeAPIError as error:
        print(f"TypeSafe error {error.status} (request {error.request_id})",
              file=sys.stderr)
        return 2

    said = [{"file": one["file"], "line": one["line"], "before": one["before"],
             "after": one["after"],
             "equivalent": round(answers.nouls[f"m{n}"].noul, 3)}
            for n, one in enumerate(gathered_all)]
    if args.json:
        print(json.dumps({"survivors": said}, indent=2))
        return 0

    gaps = [one for one in said if one["equivalent"] <= EQUIVALENT]
    idle = [one for one in said if one["equivalent"] > EQUIVALENT]
    print(f"{len(said)} survivors: {len(gaps)} read as a hole in the tests, "
          f"{len(idle)} as changes that cannot do anything.\n")
    for label, group in (("a test could have caught this", gaps),
                         ("nothing to catch", idle)):
        if not group:
            continue
        print(f"  {label}:")
        for one in group:
            print(f"    {one['equivalent']:.2f}  {one['file']}:{one['line']}  "
                  f"{one['before'][:60]}")
            print(f"          -> {one['after'][:60]}")
    print("\nThe probabilities are the judging. Which mutants survived, and that "
          "they survived,\ncame from running the suite and is not in doubt.")
    return 0


# The eight from the measurement that shaped this, each with the facts it turns
# on. `equivalent` is what the code actually does, worked out by hand.
MARKED = [
    {"file": "front/spec.py", "line": 0,
     "before": "if len(seen) == len(reachable): drop the value",
     "after": "if len(seen) >= len(reachable): drop the value",
     "function": "seen = {tuple(case[one] for one in rest) for case in reachable}",
     "facts": ["seen is a set built by iterating reachable, one entry per case"],
     "callers": [], "equivalent": True},
    {"file": "front/world.py", "line": 0,
     "before": "held, running = 0, False",
     "after": "held, running = 0, True",
     "function": ("for one in self.columns:\n"
                  "    if one and not running: held += 1\n"
                  "    running = bool(one)"),
     "facts": ["columns is a list of counts left to right; held counts runs of non-zero"],
     "callers": [], "equivalent": False},
    {"file": "front/spec.py", "line": 0,
     "before": "if relevance[(name, out)] > floor",
     "after": "if relevance[(name, out)] >= floor",
     "function": "kept = [name for name in inputs if relevance[(name, out)] > floor]",
     "facts": ["floor is PRUNE_FLOOR = 0.25",
               "relevance values are model probabilities, never exactly 0.25"],
     "callers": [], "equivalent": True},
    {"file": "front/machine.py", "line": 0,
     "before": "length = min(t + 1, k)", "after": "length = min(t, k)",
     "function": ("for t in range(len(feed)):\n"
                  "    length = min(t + 1, k)\n"
                  "    key = tuple(feed[t - i] for i in range(length))"),
     "facts": ["t counts steps from 0", "no table holds a window of length 0"],
     "callers": [], "equivalent": False},
    {"file": "front/spec.py", "line": 0,
     "before": "sum(self.outs[name] << j for j, name in enumerate(outputs))",
     "after": "sum(self.outs[name] << j for j, name in enumerate(reversed(outputs)))",
     "function": "def word(self, outputs): return sum(...)",
     "facts": ["outputs is the caller-given order, lowest bit first"],
     "callers": ["spec.py: table = [row.word(outputs) for row in rows]"],
     "equivalent": False},
    {"file": "front/spec.py", "line": 0,
     "before": "min(abs(p - 0.5) for p in self.probs.values())",
     "after": "min(p - 0.5 for p in self.probs.values())",
     "function": "def margin(self): return min(abs(p - 0.5) for p in self.probs.values())",
     "facts": ["probs are probabilities in 0..1",
               "the result is compared against a positive floor"],
     "callers": [], "equivalent": False},
    {"file": "src/serve.rs", "line": 0,
     "before": "ended.iter().zip(want).map(|(a,b)| (a-b).abs()).sum()",
     "after": "want.iter().zip(ended).map(|(a,b)| (a-b).abs()).sum()",
     "function": "fn apart(ended: &[u32], want: &[u32]) -> i64",
     "facts": ["ended and want are the same length", "the difference is taken absolute"],
     "callers": [], "equivalent": True},
    {"file": "src/serve.rs", "line": 0,
     "before": 'held.counted("batch").unwrap_or(20).clamp(1, 256)',
     "after": 'held.counted("batch").unwrap_or(20).clamp(1, 512)',
     "function": "let batch = held.counted(\"batch\").unwrap_or(20).clamp(1, 256);",
     "facts": ["batch is frames sent to the device at a time",
               "every caller in this repo passes 20 or leaves it out"],
     "callers": [], "equivalent": True},
]


def marked(args) -> int:
    """The eight, judged and scored against what the code does."""
    with TypeSafeClient() as client:
        answers = judge(client, MARKED, args.model)
    right = 0
    print(f"  {'P(eq)':>6} {'said':>11} {'truth':>11}")
    for n, one in enumerate(MARKED):
        p = answers.nouls[f"m{n}"].noul
        said = p > EQUIVALENT
        right += said == one["equivalent"]
        print(f"  {p:>6.2f} {'equivalent' if said else 'a gap':>11} "
              f"{'equivalent' if one['equivalent'] else 'a gap':>11} "
              f"{'  ' if said == one['equivalent'] else '<<'} {one['before'][:44]}")
    print(f"\n{right}/{len(MARKED)} judged as the code behaves.")
    print("Eight mutants chosen by whoever wrote this, which is not a score to "
          "trust. Bring\nyour own survivors: noha report --json, then --from.")
    return 0 if right == len(MARKED) else 1


if __name__ == "__main__":
    raise SystemExit(main())
