# Troubleshooting Scenarios

Common symptoms and likely causes:

- **No or slow block progression**: runner started workloads before readiness, insufficient run window, or environment too slow—extend duration or enable slow-environment tuning.
- **Transactions not included**: missing or insufficient wallet seeding, misaligned transaction rate with block cadence, or network instability—reduce rate and verify wallet setup.
- **Chaos stalls the run**: node control not available for the chosen runner or restart cadence too aggressive—enable control capability and widen restart intervals.
- **Observability gaps**: metrics or logs unreachable because ports clash or services are not exposed—adjust observability ports and confirm runner wiring.
- **Flaky behavior across runs**: mixing chaos with functional smoke tests or inconsistent topology between environments—separate deterministic and chaos scenarios and standardize topology presets.
