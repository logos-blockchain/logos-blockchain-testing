# Annotated Tree

High-level view of the workspace and how pieces relate:
```
nomos-testing/
├─ testing-framework/
│  ├─ configs/          # shared configuration helpers
│  ├─ core/             # scenario model, runtime, topology
│  ├─ workflows/        # workloads, expectations, DSL extensions
│  └─ runners/          # local, compose, k8s deployment backends
├─ tests/               # integration scenarios using the framework
└─ scripts/             # supporting setup utilities (e.g., assets)
```

Each area maps to a responsibility: describe configs, orchestrate scenarios,
package common traffic and assertions, adapt to environments, and demonstrate
end-to-end usage.
