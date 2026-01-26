# Book → Repo Sync Review (2025-12-20)

Reviewed against `git rev-parse HEAD` at the time of writing, plus local working tree changes.

## Checks Run

- `mdbook build book`
- `mdbook test book`
- `cargo build -p doc-snippets`
- Verified `book/src/SUMMARY.md` covers all pages in `book/src/` (no orphaned pages)
- Verified all `scripts/...` paths referenced from the book exist
- Compared `NOMOS_*` environment variables used in `scripts/`, `testing-framework/`, and `examples/` vs. `book/src/environment-variables.md`

## Findings / Fixes Applied

- `book/src/environment-variables.md` was not a complete reference: it missed multiple `NOMOS_*` variables used by the repo (scripts + framework). Added the missing variables and corrected a misleading note about `RUST_LOG` vs node logging.
- `book/src/running-examples.md` “Quick Smoke Matrix” section didn’t reflect current `scripts/run/run-test-matrix.sh` flags. Added the commonly used options and clarified the relationship to `LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD`.
- `book/src/part-iv.md` existed but was not in `book/src/SUMMARY.md`. Removed it so the rendered book doesn’t silently diverge from the filesystem.
- `mdbook test book` was failing because:
  - Many Rust examples were written as ` ```rust` (doctested by default) but depend on workspace crates; they aren’t standalone doctest snippets.
  - Several unlabeled code blocks (e.g. tree/log output) were treated as Rust by rustdoc.
  - Updated code fences to ` ```rust,ignore` for non-standalone Rust examples and to ` ```text` for non-Rust output blocks so `mdbook test book` succeeds.

## Pages Reviewed (No Skips)

All pages under `book/src/` currently included by `book/src/SUMMARY.md`:

- `annotated-tree.md`
- `api-levels.md`
- `architecture-overview.md`
- `authoring-scenarios.md`
- `best-practices.md`
- `chaos.md`
- `ci-integration.md`
- `custom-workload-example.md`
- `design-rationale.md`
- `dsl-cheat-sheet.md`
- `environment-variables.md`
- `examples-advanced.md`
- `examples.md`
- `extending.md`
- `faq.md`
- `glossary.md`
- `internal-crate-reference.md`
- `introduction.md`
- `logging-observability.md`
- `node-control.md`
- `operations-overview.md`
- `part-i.md`
- `part-ii.md`
- `part-iii.md`
- `part-v.md`
- `part-vi.md`
- `prerequisites.md`
- `project-context-primer.md`
- `quickstart.md`
- `runners.md`
- `running-examples.md`
- `running-scenarios.md`
- `scenario-builder-ext-patterns.md`
- `scenario-lifecycle.md`
- `scenario-model.md`
- `testing-philosophy.md`
- `topology-chaos.md`
- `troubleshooting.md`
- `usage-patterns.md`
- `what-you-will-learn.md`
- `workloads.md`
- `workspace-layout.md`

