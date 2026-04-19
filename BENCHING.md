# Benching

The README numbers were taken on a 13th-gen Intel Core i7-1360P with the
setup below. Any serious comparison — A/B against a baseline, profiling a
change — should use the same hygiene.

## Setup

1. **Performance governor.** Variable frequency turns a 5 ns bench into a
   20% noise problem. On Linux:

       sudo cpupower frequency-set -g performance

   Confirm: `cpupower frequency-info | grep "current policy"`.

2. **Pin to a P-core and its SMT sibling.** `taskset -c 6,7` on the i7-1360P.
   Adjust for your CPU. Pinning keeps the bench off the scheduler and off any
   E-cores that would cause spurious migrations.

3. **Quiet system.** `uptime` load average under 1.0 before starting. Close
   the editor, the browser, the LSP — rust-analyzer alone will contaminate
   numbers with cc1-style compile spikes during the bench.

## Running

    taskset -c 6,7 cargo bench --bench dispatch

For A/B against a baseline:

    # save the current state as a baseline
    taskset -c 6,7 cargo bench --bench dispatch -- --save-baseline main

    # switch to the branch under test, compare
    taskset -c 6,7 cargo bench --bench dispatch -- --baseline main

Criterion prints `change: [-X% -Y% -Z%]` with a 95% confidence interval and
flags regressions at `p < 0.05`. Trust the p-value over the median.

## Interpreting

- `emit_zst/N` — dispatch loop cost with N subscribers, no payload work.
  Slope across N is the per-subscriber incremental cost.
- `emit_small_payload/N` / `emit_large_payload/N` — same with a payload
  that subscribers read. Slope reflects both dispatch and payload-touch cost.
- `emit_no_subscribers` — empty-bus floor. HashMap lookup that misses on
  an empty map.
- `emit_type_miss` — populated-bus miss. HashMap lookup that finds no key
  for this `TypeId`. Different slot distribution, slightly slower than the
  empty-bus case.
- `emit_cold/N` — first emit after subscribe, I-cache and D-cache cold.
  `iter_batched` rebuilds the bus per iteration, so setup isn't amortized.

The denser subscriber sweep (powers of two up to 1024) is there for slope
visibility — the L1D on this CPU is 32 KB, ~1024 subscribers at 32 B
stride, so the curve should start bending near the right edge.

## If your numbers disagree with the README

Likely causes, in order of how often they bite:

1. Not pinning (`taskset -c` missing). Scheduler migrations show up as 2–5x
   outliers.
2. Governor isn't `performance`. The CPU boosts up for the warmup and
   throttles during measurement.
3. Different hardware. Zen vs Intel, E-core vs P-core, DDR4 vs DDR5 all
   show up at these timescales.
4. Concurrent load. Close everything. Check `ps aux --sort=-%cpu`.

If all of those check out and the numbers still disagree, the difference
is real — open an issue with your hardware and the bench output.
