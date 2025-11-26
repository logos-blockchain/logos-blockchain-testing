# Project Context Primer

This book focuses on the Nomos Testing Framework. It assumes familiarity with
the Nomos architecture, but for completeness, here is a short primer.

- **Nomos** is a modular blockchain protocol composed of validators, executors,
  and a data-availability (DA) subsystem.
- **Validators** participate in consensus and produce blocks.
- **Executors** run application logic or off-chain computations referenced by
  blocks.
- **Data Availability (DA)** ensures that data referenced in blocks is
  published and retrievable, including blobs or channel data used by workloads.

These roles interact tightly, which is why meaningful testing must be performed
in multi-node environments that include real networking, timing, and DA
interaction.
