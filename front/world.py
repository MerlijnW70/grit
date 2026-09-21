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
]

# Descriptions whose answer is arithmetic over what was counted, so the judging
# can be marked rather than admired. The rule picks the scene, never the model.
CHECKS: list[tuple[str, object]] = [
    ("the sand lying flat, in as few rows as it can",
     lambda held: min(range(len(held)), key=lambda n: held[n].tall)),
    ("the tallest, narrowest column of sand",
     lambda held: max(range(len(held)),
                      key=lambda n: held[n].tall / max(held[n].widest, 1))),
    ("the room with the most settled in it",
     lambda held: max(range(len(held)), key=lambda n: held[n].standing)),
    ("the widest spread across the room",
     lambda held: max(range(len(held)), key=lambda n: held[n].widest)),
    ("water rather than sand",
     lambda held: max(range(len(held)),
                      key=lambda n: held[n].tally.get("water", 0))),
    ("barely anything settled at all",
     lambda held: min(range(len(held)), key=lambda n: held[n].standing)),
]


@dataclass
class Poured:
    scene: str
    rows: list[int]
    tally: dict
    listed: int

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
                  tally=answer.get("tally", {}),
                  listed=answer.get("listed", 0))


def told(one: "Poured") -> str:
    """What was counted, said the same way for every scene."""
    return (f"{one.tally.get('water', 0)} cells of water and "
            f"{one.tally.get('sand', 0)} of sand settled; the widest row holds "
            f"{one.widest} cells; they lie across {one.tall} rows; the skyline "
            f"from the top down reads {one.shape()}")


def judge(client, wanted: str, poured: list["Poured"], model=None) -> dict[int, float]:
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
    spread = answer.choices["kept"].probabilities
    return {n: spread.get(f"o{n}", 0.0) for n in range(len(poured))}


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

    right = 0
    with TypeSafeClient() as client:
        for wanted, chooser in CHECKS:
            says = judge(client, wanted, tried, args.model)
            kept = max(range(len(tried)), key=lambda n: says[n])
            wants = chooser(tried)
            hit = kept == wants
            right += hit
            rest = [says[n] for n in range(len(tried)) if n != kept]
            margin = says[kept] - max(rest, default=0.0)
            print(f"  {'ok  ' if hit else 'MISS'} {says[kept]:.2f} "
                  f"(next {margin:+.2f})  {wanted}")
            if not hit:
                print(f"         kept   {tried[kept].scene}")
                print(f"         wanted {tried[wants].scene}")
    print()
    print(f"{right}/{len(CHECKS)} descriptions matched the scene the arithmetic picks.")
    return 0 if right == len(CHECKS) else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="world", description="Pour a room several ways and keep the one that was meant.")
    parser.add_argument("wanted", nargs="?", default="",
                        help="what the settled room should be like, in plain English")
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
            says = judge(client, args.wanted, tried, args.model)
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
