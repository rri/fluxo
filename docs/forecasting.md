# How forecasts work

fluxo plays out the whole plan 10,000 times. In each run, every task takes a random but plausible
length, every risk either happens or doesn't, and each goal finishes when the slowest chain of work
beneath it finishes. The spread of those 10,000 finish times is the forecast.

The engine is `crates/fluxo-sim/src/engine.rs` (pure, no I/O). `crates/fluxo-web/src/forecast.rs`
builds its input from the database and turns its output into dates; `crates/fluxo-web/src/ranking.rs`
uses it to rank Focus. Decision IDs (D11, D18–D21) refer to `docs/implementation-plan.md`.

## 1. What each item contributes

| Item | What one run does with it |
|---|---|
| Task | Draws a duration around its likely estimate (its remaining days if in progress). Clarity sets the range: High 0.85×–1.3×, Medium 0.75×–1.6×, Low 0.6×–2.2× (D18). |
| Goal | Finishes when its last step finishes, plus "unknown work" if coverage isn't full: a random 0–25% of its known length for Partial, 0–75% for Low (D19). |
| Goal with a rough size, no steps | A duration anywhere between its min and max, all equally likely. |
| Risk | Fires with its likelihood (a 30% risk fires in about 3,000 runs). When it fires, it adds a random delay between its min and max. |
| Done task, achieved goal | Takes no time. |
| Goal with no steps and no size | Left out; Focus asks someone to clarify it instead. |

Task durations follow a PERT curve: a Beta distribution over the clarity range whose peak sits at
the likely estimate. The range stretches further above the estimate than below it, so overruns are
bigger than underruns, as in real work. For example, a 10-day task of Medium clarity ranges from 7.5
to 16 days, is most often about 10, and averages about 10.6: 0.6 days of expected overrun.

## 2. One run

Every item has two moments:

- **Start:** the latest of the finishes of everything it needs, and the starts of the goals it
  feeds. A step can't start before its goal starts.
- **Finish:**
  - A task: its start, plus its drawn duration, plus any risks that fired.
  - A goal: its latest step's finish, plus its unknown work, plus any risks that fired.

These are worked out in dependency order, computed once when the model is compiled; loops are
refused there. Work that feeds several goals is drawn once per run, so if it runs late, it runs
late for every goal that waits on it.

## 3. From 10,000 runs to a forecast

- **Dates:** each goal's finish times are sorted.
  - **P50** is the 5,000th: an even chance of finishing by then.
  - **P85** is the 8,500th: the date to commit to.
  - The finish chart on a goal page is a 44-bar histogram of the same runs.
- **Calendar:** results are in working days. Pages turn them into dates with the goal owner's
  calendar: working days, days off and time zone (D20).
- **Critical path:** in each run, fluxo walks back from the goal's finish through whichever step or
  prerequisite set each "latest" time. A step's criticality is the share of runs in which it lay on
  that path. It drives the path highlighting and "Top risks on the path".

## 4. Same plan, same answer

- **Fixed seed:** random numbers come from a fixed seed, so identical inputs always give identical
  dates. Nothing jitters on refresh.
- **Separate streams:** each item draws from its own random stream in each run, keyed by seed, run
  and item. Results don't depend on how many cores share the work (D11). Changing one item leaves
  every other item's draws unchanged, so when a date moves, the move comes from the change, not
  from noise.
- **When it recalculates:** when a page needs a forecast and the fingerprint of its inputs has
  changed. The whole instance is simulated at once, so shared work is consistent everywhere.

## 5. How Focus uses it

Focus asks, for each item, how much earlier its goals would finish on average if it were resolved:

- **Do (a task):** fix the task at its likely estimate, so it can't overrun.
- **De-risk (a risk):** remove the risk.
- **Clarify (a goal):** set its coverage to Full, removing its unknown work.

A quick estimate ranks everything. The top 20 are then re-simulated with 2,000 runs each, using the
same random draws as the plan without the change ("common random numbers", D21). The difference in
mean finish is the item's gain, summed across every top-level goal it feeds: the "−2.4d" on Focus.
Because both sides share their draws, even small gains are measured precisely; gains under 0.1
days show as "<0.1d".
