# Build job metrics

Each build job is measured while it runs, so the round ends with numbers and not with an impression. One row per job, filled in by the judge, in `docs/build/metrics.csv`. A field nobody measured stays empty; it is never estimated afterwards.

## Fields

| Field | Unit | Taken from |
|---|---|---|
| `job` | name | the spec file, `docs/build/<job>.md` |
| `spec_commit` | sha | the commit that holds the spec |
| `bounty` | SOL | the escrow's amount |
| `fees` | SOL | network fees paid by the judge's wallet for post, judge and pay, read from the wallet's balance before and after |
| `posted_at` | UTC timestamp | escrow transaction |
| `claimed_at` | UTC timestamp | claim transaction |
| `delivered_at` | UTC timestamp | delivery announcement |
| `paid_at` | UTC timestamp | release transaction |
| `worker_wall` | minutes | `delivered_at - claimed_at` |
| `judge_wall` | minutes | from fetching the delivery to the verdict, timed by the judge |
| `judge_steps` | count | commands the judge ran to reach the verdict (apply, gate steps, break, reruns) |
| `gate_wall` | minutes | wall time of the gate on the judge's run |
| `rounds` | count | deliveries until accepted; 1 is accepted first time |
| `verdict` | `accepted` / `refused` / `split` | the judge |
| `refusal` | text | which acceptance step failed, one line |
| `break_caught` | `yes` / `no` | whether the worker's `break.patch` failed the named test |
| `judge_break_caught` | `yes` / `no` | whether the judge's own break failed a test |
| `numbers_checked` | `n/m` | numbers in the patches the judge reproduced, out of those stated |
| `merged` | sha or empty | the commit on Grit main the patches landed as |
| `gain` | text | the measured effect after merge, with the command that measured it |
| `friction` | text | every point where the job tooling, not the work, cost time: a failed or repeated command, a step done by hand, an error message that did not say what was wrong, a missing feature. One entry per point, each with the minutes it cost |

## Derived at the end of the round

- Cost per accepted job: `(sum of bounty + fees over accepted jobs) / accepted jobs`.
- Judge effort per job: `judge_wall` and `judge_steps`, median and maximum.
- Acceptance rate: accepted first time, accepted after rework, refused.
- Test strength: the share of jobs where both breaks were caught.
- Friction: all `friction` entries grouped by cause, with their minutes summed.
