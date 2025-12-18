# Core Content: ScenarioBuilderExt Patterns

> **When should I read this?** After writing 2-3 scenarios. This page documents patterns that emerge from real usage—come back when you're refactoring or standardizing your test suite.

Patterns that keep scenarios readable and reusable:

- **Topology-first**: start by shaping the cluster (counts, layout) so later
  steps inherit a clear foundation.
- **Bundle defaults**: use the DSL helpers to attach common expectations (like
  liveness) whenever you add a matching workload, reducing forgotten checks.
- **Intentional rates**: express traffic in per-block terms to align with
  protocol timing rather than wall-clock assumptions.
- **Opt-in chaos**: enable restart patterns only in scenarios meant to probe
  resilience; keep functional smoke tests deterministic.
- **Wallet clarity**: seed only the number of actors you need; it keeps
  transaction scenarios deterministic and interpretable.

These patterns make scenario definitions self-explanatory while staying aligned
with the framework’s block-oriented timing model.
