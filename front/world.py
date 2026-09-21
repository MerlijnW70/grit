"""English about a poured room -> a scenario grit ran, chosen by what it made.\n
The division is the same one the rest of this front door uses, in the one place
it still fits after the engine has done its exact work.

  code  enumerates the scenarios, runs every one, and decides
  grit  pours and settles them, and counts what lies where
  Jev   reads the shapes that came out and says which one the sentence meant

Nothing about a scenario is inferred. The tally and the rows are counted on the
host, cell by cell. What cannot be counted is whether a cone of sand is what
somebody meant by "a tall heap", and that is the whole of what gets asked.

    python front/world.py "a tall narrow heap"
    python front/world.py "water spread flat across the floor" --json
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    from typesafe_sdk import Choice, TypeSafeAPIError, TypeSafeClient
except ImportError:
    sys.exit("typesafe-sdk is not installed: pip install typesafe-sdk")

from spec import DEFAULT_MCP, grit

WIDE, DEEP = 128, 128

# Scenes chosen so the things a sentence can say about a room actually differ
# between them. The first set varied only the spout width and every outcome was
# a narrow column down to the floor, so asking which was wide and low was a
# question with no answer in the set. Measured here: the widest row runs from
# twenty four to a hundred and twenty six, and the rows holding anything from
# one to a hundred and twenty five.
SCENES: list[tuple[str, dict]] = [
    ("a narrow spout, pouring a long time",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "brush": {"at": 16 * 128 + 64, "is": "sand", "span": 9, "frames": 250}}),
    ("a wide spout, pouring a long time",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "brush": {"at": 16 * 128 + 64, "is": "sand", "span": 225, "frames": 250}}),
    ("a whole row of sand laid across the top, and no spout",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "cells": [{"at": 8 * 128 + column, "is": "sand"} for column in range(4, 124)]}),
    ("a shallow room with a wide spout",
     {"wide": 128, "deep": 32, "frames": 300, "most": 1,
      "brush": {"at": 4 * 128 + 64, "is": "sand", "span": 97, "frames": 250}}),
    ("a narrow spout, shut off early",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "brush": {"at": 16 * 128 + 64, "is": "sand", "span": 9, "frames": 30}}),
    ("a wide spout of water",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "brush": {"at": 16 * 128 + 64, "is": "water", "span": 97, "frames": 250}}),
    # A spout makes one heap however it is aimed, so a set built only from
    # spouts cannot be asked how many heaps there are. Stacks laid apart fall
    # straight down and land apart. A shelf across the middle was tried first
    # and split nothing: the sand piles on it and pours around it and the
    # columns stay joined. That is why a set is counted before it is asked
    # about - the scene was named after what it was meant to do.
    ("two stacks of sand laid far apart",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "cells": [{"at": row * 128 + column, "is": "sand"}
                for row in range(4, 12)
                for column in list(range(18, 30)) + list(range(98, 110))]}),
    ("three stacks of sand laid apart",
     {"wide": 128, "deep": 128, "frames": 300, "most": 1,
      "cells": [{"at": row * 128 + column, "is": "sand"}
                for row in range(4, 12)
                for column in list(range(14, 22)) + list(range(60, 68))
                + list(range(106, 114))]}),
]

# Below this the Choice has spread itself thin across the rooms, which means the
# description does not tell them apart rather than that its answer is close.
#
# Measured over nine descriptions, this catches one kind of wrong pick and not
# the other. "Heaped up rather than lying flat" spread a hundredth across two
# scenes and was wrong; that is what a floor is for. "The room where the engine
# did the least work" was wrong at eighty one hundredths of certainty, because
# it is eighty three quads against seventy two and close numbers compared
# against each other is the standing weakness. Nothing about a spread says that.
#
# The floor sits at the edge of the band that drops only the wrong one: a step
# higher and it starts dropping right answers. --check prints the whole sweep.
TELLING = 0.20


def say_spread(held: list[tuple[float, bool]]) -> None:
    """Does a thin spread say the pick is not to be trusted?"""
    if not held:
        return
    print()
    print(f"  {'floor':>6} {'dropped':>8} {'of them wrong':>14} {'kept wrong':>11}")
    for floor in [round(0.05 * n, 2) for n in range(1, 13)]:
        dropped = [hit for sure, hit in held if sure < floor]
        kept = [hit for sure, hit in held if sure >= floor]
        print(f"  {floor:>6.2f} {len(dropped):>8} "
              f"{sum(1 for hit in dropped if not hit):>14} "
              f"{sum(1 for hit in kept if not hit):>11}")
    print("  A floor worth having drops the wrong picks and keeps the right ones.")


# What a description can be marked against. Every one of these is arithmetic
# over what grit counted, so the mark is on the judging and never on the pour.
# They are proxies, and naming them out loud matters: the one disagreement this
# suite has ever found was the proxy losing an argument with the sentence, not
# the model getting it wrong.
MEASURED = {
    "rows": lambda one: one.tall,
    "widest": lambda one: one.widest,
    "settled": lambda one: one.standing,
    "water": lambda one: one.tally.get("water", 0),
    "sand": lambda one: one.tally.get("sand", 0),
    "listed": lambda one: one.listed,
    # Tall against wide, for the descriptions that are about the shape rather
    # than about any one side of it.
    "slenderness": lambda one: one.tall / max(one.widest, 1),
    "flatness": lambda one: one.widest / max(one.tall, 1),
    "clumps": lambda one: one.clumps,
}

CHECKS: list[tuple[str, str]] = [
    ("the sand lying flat, in as few rows as it can", "min rows"),
    ("the tallest, narrowest column of sand", "max slenderness"),
    ("the room with the most settled in it", "max settled"),
    ("the widest spread across the room", "max flatness"),
    ("water rather than sand", "max water"),
    ("barely anything settled at all", "min settled"),
    ("the sand split into more than one heap", "max clumps"),
]


def picked(rule: str, held: list) -> int:
    """Work out which scene a description means, from what was counted.

    Every measurable named here is also spelled out in the question the model
    is asked, and that is not a coincidence to be kept by hand: a rule marking
    on something the question never says is unanswerable rather than hard.
    """
    way, _, name = rule.partition(" ")
    measure = MEASURED.get(name)
    if measure is None or way not in ("min", "max"):
        raise SystemExit(
            f"{rule!r} is not a rule: say min or max, then one of "
            + ", ".join(sorted(MEASURED))
        )
    chosen = min if way == "min" else max
    return chosen(range(len(held)), key=lambda n: measure(held[n]))


def load_checks(path: str) -> list[tuple[str, str]]:
    """Your descriptions instead of mine.

    Six descriptions invented by whoever wrote the tool, marked against proxies
    the same person chose, is enough to find a wrong primitive - it did - and
    not enough to put a number on. Bring your own:

    {"checks": [{"wanted": "a flat sheet of water", "pick": "min rows"}]}
    """
    held = json.loads(Path(path).read_text(encoding="utf-8"))
    return [(one["wanted"], one["pick"]) for one in held["checks"]]


@dataclass
class Poured:
    scene: str
    rows: list[int]
    columns: list[int]
    tally: dict
    listed: int

    @property
    def clumps(self) -> int:
        """How many separate heaps, counted as runs of columns holding anything."""
        held, running = 0, False
        for one in self.columns:
            if one and not running:
                held += 1
            running = bool(one)
        return held

    @property
    def standing(self) -> int:
        return self.tally.get("sand", 0) + self.tally.get("water", 0)

    @property
    def widest(self) -> int:
        return max(self.rows, default=0)

    @property
    def tall(self) -> int:
        return sum(1 for one in self.rows if one)

    def shape(self) -> str:
        """The skyline, thinned to something a sentence can be held against."""
        held = [one for one in self.rows if one]
        if not held:
            return "nothing settled"
        steps = max(1, len(held) // 8)
        return " ".join(str(one) for one in held[::steps][:8])

    def named(self) -> str:
        return self.scene


def pour(binary, scene: str, arguments: dict) -> Poured:
    answer = grit("grit_pour", arguments, binary)
    return Poured(scene=scene,
                  rows=list(answer.get("rows", [])),
                  columns=list(answer.get("columns", [])),
                  tally=answer.get("tally", {}),
                  listed=answer.get("listed", 0))


def told(one: "Poured") -> str:
    """What was counted, said the same way for every scene."""
    # Everything a description can be marked against has to be in here.
    # A check once marked on how much work the engine did while the question
    # never mentioned it, which is not a hard question but an unanswerable one.
    return (f"{one.tally.get('water', 0)} cells of water and "
            f"{one.tally.get('sand', 0)} of sand settled; the widest row holds "
            f"{one.widest} cells; they lie across {one.tall} rows in "
            f"{one.clumps} separate heaps; the engine "
            f"stepped {one.listed} quads on an average frame; the skyline from "
            f"the top down reads {one.shape()}")


def judge(client, wanted: str, poured: list["Poured"],
          model=None) -> tuple[dict[int, float], float]:
    """One request, one question, every outcome as an option beside the others.

    This was a Noul each to begin with, asked on its own. That cannot answer a
    superlative: "the tallest" or "barely any" is a claim about the others, and
    a question holding one outcome has no others in it. Measured, the plain
    descriptions came out right and every comparative one was a coin toss.

    A Choice is the primitive that compares, so the outcomes are its options and
    the probabilities it spreads over them are the ranking. Each option is only
    what was counted: no scene is told what it was set to, so none can be picked
    for sounding like the words in the sentence.
    """
    options = {f"o{n}": told(one) for n, one in enumerate(poured)}
    answer = client.system_one(
        state={"wanted": wanted,
               "reading": "the room is 128 cells across; a row holding many "
                          "cells is a wide layer and many rows holding a few is "
                          "a tall column"},
        questions={
            "kept": Choice(
                instructions="Which of these settled rooms is the one `wanted` describes?",
                criteria=options,
            )
        },
        model=model,
    )
    kept = answer.choices["kept"]
    spread = kept.probabilities
    return ({n: spread.get(f"o{n}", 0.0) for n in range(len(poured))},
            kept.confidence)


def marked(tried: list[Poured], args) -> int:
    """Judge the descriptions whose answer can be worked out, and mark it.

    Every scene is poured once and judged against every description, so what is
    measured here is the judging and not the pouring.
    """
    print("what was counted, before anything was judged:")
    print()
    print(f"  {'widest':>7} {'rows':>5} {'settled':>8} {'water':>6}  scene")
    for one in tried:
        print(f"  {one.widest:>7} {one.tall:>5} {one.standing:>8} "
              f"{one.tally.get('water', 0):>6}  {one.scene}")
    widths = [one.widest for one in tried]
    depths = [one.tall for one in tried]
    print()
    print(f"  widest runs {min(widths)} to {max(widths)}, "
          f"rows {min(depths)} to {max(depths)} - a set has to differ in a thing")
    print("  before it can be asked about, and the first one here did not.")
    print()

    checks = load_checks(args.cases) if args.cases else CHECKS
    right = 0
    spreads: list[tuple[float, bool]] = []
    with TypeSafeClient() as client:
        for wanted, chooser in checks:
            says, sure = judge(client, wanted, tried, args.model)
            kept = max(range(len(tried)), key=lambda n: says[n])
            wants = picked(chooser, tried)
            hit = kept == wants
            right += hit
            rest = [says[n] for n in range(len(tried)) if n != kept]
            margin = says[kept] - max(rest, default=0.0)
            told_sure = "   too flat to say" if sure < TELLING else ""
            print(f"  {'ok  ' if hit else 'MISS'} {says[kept]:.2f} "
                  f"(next {margin:+.2f}, sure {sure:.2f})  {wanted}   "
                  f"[{chooser}]{told_sure}")
            spreads.append((sure, hit))
            if not hit:
                print(f"         kept   {tried[kept].scene}")
                print(f"         wanted {tried[wants].scene}")
    print()
    print(f"{right}/{len(checks)} descriptions matched the scene its rule picks.")
    say_spread(spreads)
    print("Those rules are proxies, named beside each line. Where one loses an "
          "argument with a sentence the mark belongs on the proxy.")
    return 0 if right == len(checks) else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="world", description="Pour a room several ways and keep the one that was meant.")
    parser.add_argument("wanted", nargs="?", default="",
                        help="what the settled room should be like, in plain English")
    parser.add_argument("--cases", help="JSON file of your own descriptions and the "
                                        "rule each is marked against")
    parser.add_argument("--check", action="store_true",
                        help="run the descriptions whose answer is arithmetic over what "
                             "was counted, and mark the judging against them")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--model")
    parser.add_argument("--mcp", default=os.environ.get("GRIT_MCP", DEFAULT_MCP))
    args = parser.parse_args(argv)

    if not os.environ.get("TYPESAFE_API_KEY"):
        print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
        return 2

    tried = [pour(args.mcp, scene, arguments) for scene, arguments in SCENES]

    if args.check:
        return marked(tried, args)

    try:
        with TypeSafeClient() as client:
            says, sure = judge(client, args.wanted, tried, args.model)
    except TypeSafeAPIError as error:
        print(f"TypeSafe error {error.status} (request {error.request_id})", file=sys.stderr)
        return 2

    ranked = sorted(range(len(tried)), key=lambda n: -says[n])
    best = tried[ranked[0]] if ranked else None
    report = {
        "wanted": args.wanted,
        "tried": [
            {"scene": one.scene, "widest": one.widest, "rows": one.tall,
             "settled": one.standing, "listed": one.listed,
             "matches": round(says[n], 3)}
            for n, one in enumerate(tried)
        ],
        "kept": None if best is None else
        {"scene": best.scene, "matches": round(says[ranked[0]], 3)},
    }
    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(f"poured {len(tried)} ways, judged against: {args.wanted}\n")
    print(f"  {'match':>6} {'widest':>7} {'rows':>5} {'settled':>8}  scene")
    for n in ranked:
        one = tried[n]
        print(f"  {says[n]:>6.2f} {one.widest:>7} {one.tall:>5} "
              f"{one.standing:>8}  {one.scene}")
    if best is not None:
        print(f"\nkept: {best.named()}")
        print("Everything in those columns was counted on the host. Only the "
              "match was judged, and\nonly against the numbers beside it - no "
              "scenario was told what it was called.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
