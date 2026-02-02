# Runner review notes

File: `testing-framework/core/src/scenario/runtime/runner.rs`

## Concerns

1) **Potential hang if workloads don’t self-terminate**
- `run_workloads` never cancels workloads after the duration expires. It waits for `drive_until_timer` to time out, then optionally does a cooldown, then `drain_workloads` which waits for all tasks to finish. If a workload is long-lived or never exits on its own, the scenario can hang indefinitely after the duration window.
- Suggestion: call `workloads.abort_all()` when `drive_until_timer` returns `Ok(false)` (timeout) or add a second timeout around `drain_workloads`.

2) **Block-feed settle constants are inconsistent**
- `DEFAULT_BLOCK_FEED_SETTLE_WAIT` is 1s, but `MIN_BLOCK_FEED_SETTLE_WAIT` is 2s. In `settle_before_expectations`, any zero wait becomes 1s and then gets max’d to 2s, so the default is effectively unused.
- Suggestion: set default ≥ min, or remove the default constant and use min directly.

## Questions

- Are workloads guaranteed to finish on their own at the end of `scenario.duration()`? If not, the hang risk is real.
