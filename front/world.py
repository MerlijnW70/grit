"""English about a poured room -> a scenario grit ran, chosen by what it made.

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
import itertools
import json
import os
import subprocess
import sys
from dataclasses import dataclass

try:
    from typesafe_sdk import Noul, TypeSafeAPIError, TypeSafeClient
except ImportError:
    sys.exit("typesafe-sdk is not installed: pip install typesafe-sdk")

from spec import DEFAULT_MCP, grit

WIDE, DEEP = 128, 128


@dataclass
class Poured:
    span: int
    stuff: str
    frames: int
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
        return f"{self.stuff}, spout {self.span} wide, pouring {self.frames} frames"


def pour(binary, span: int, stuff: str, frames: int) -> Poured:
    answer = grit(
        "grit_pour",
        {
            "wide": WIDE, "deep": DEEP, "frames": 300, "most": 1,
            "brush": {"at": (DEEP // 8) * WIDE + WIDE // 2, "is": stuff,
                      "span": span, "frames": frames},
        },
        binary,
    )
    return Poured(span=span, stuff=stuff, frames=frames,
                  rows=[one for one in answer.get("rows", [])],
                  tally=answer.get("tally", {}),
                  listed=answer.get("listed", 0))


def judge(client, wanted: str, poured: list[Poured], model=None) -> dict[int, float]:
    """One request. Every scenario is judged against the sentence on its own.

    The model is given what was counted, not a picture and not a name: how wide
    the widest row is, how many rows hold anything, how much settled, and the
    skyline thinned to eight numbers. A scenario cannot be picked for sounding
    right, because its own settings are not in the question.
    """
    state = {
        "wanted": wanted,
        "reading": "each outcome gives the widest row, how many rows hold "
                   "anything, how many cells settled, and the skyline from the "
                   "top down; the room is 128 cells across and 128 deep",
    }
    questions = {}
    for n, one in enumerate(poured):
        questions[f"o{n}"] = Noul(instructions=(
            f"This is what settled: {one.tally.get('water', 0)} cells of water and "
            f"{one.tally.get('sand', 0)} of sand, the widest row holding {one.widest} "
            f"cells, spread over {one.tall} rows, with the skyline from the top down "
            f"reading {one.shape()}. This is what `wanted` describes."
        ))
    answers = client.system_one(state=state, questions=questions, model=model)
    return {n: answers.nouls[f"o{n}"].noul for n in range(len(poured))}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="world", description="Pour a room several ways and keep the one that was meant.")
    parser.add_argument("wanted", help="what the settled room should be like, in plain English")
    parser.add_argument("--spans", default="9,49,97", help="spout widths to try")
    parser.add_argument("--stuff", default="sand,water", help="what to pour")
    parser.add_argument("--frames", default="80,200", help="how long to pour for")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--model")
    parser.add_argument("--mcp", default=os.environ.get("GRIT_MCP", DEFAULT_MCP))
    args = parser.parse_args(argv)

    if not os.environ.get("TYPESAFE_API_KEY"):
        print("TYPESAFE_API_KEY is not set (console.typesafe.ai/keys)", file=sys.stderr)
        return 2

    tried = [
        pour(args.mcp, int(span), stuff, int(frames))
        for span, stuff, frames in itertools.product(
            args.spans.split(","), args.stuff.split(","), args.frames.split(","))
    ]

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
            {"span": one.span, "stuff": one.stuff, "frames": one.frames,
             "widest": one.widest, "rows": one.tall, "settled": one.standing,
             "listed": one.listed, "matches": round(says[n], 3)}
            for n, one in enumerate(tried)
        ],
        "kept": None if best is None else
        {"span": best.span, "stuff": best.stuff, "frames": best.frames,
         "matches": round(says[ranked[0]], 3)},
    }
    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(f"poured {len(tried)} ways, judged against: {args.wanted}\n")
    print(f"  {'match':>6} {'stuff':>6} {'spout':>6} {'frames':>7} "
          f"{'widest':>7} {'rows':>5} {'settled':>8}  skyline")
    for n in ranked:
        one = tried[n]
        print(f"  {says[n]:>6.2f} {one.stuff:>6} {one.span:>6} {one.frames:>7} "
              f"{one.widest:>7} {one.tall:>5} {one.standing:>8}  {one.shape()}")
    if best is not None:
        print(f"\nkept: {best.named()}")
        print("Everything in those columns was counted on the host. Only the "
              "match was judged, and\nonly against the numbers beside it - no "
              "scenario was told what it was called.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
