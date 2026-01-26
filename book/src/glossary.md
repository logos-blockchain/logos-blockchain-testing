# Glossary

- **Node**: process that participates in consensus and produces blocks.
- **Deployer**: component that provisions infrastructure (spawns processes,
  creates containers, or launches pods), waits for readiness, and returns a
  Runner. Examples: LocalDeployer, ComposeDeployer, K8sDeployer.
- **Runner**: component returned by deployers that orchestrates scenario
  execution—starts workloads, observes signals, evaluates expectations, and
  triggers cleanup.
- **Workload**: traffic or behavior generator that exercises the system during a
  scenario run.
- **Expectation**: post-run assertion that judges whether the system met the
  intended success criteria.
- **Topology**: declarative description of the cluster shape, roles, and
  high-level parameters for a scenario.
- **Scenario**: immutable plan combining topology, workloads, expectations, and
  run duration.
- **Blockfeed**: stream of block observations used for liveness or inclusion
  signals during a run.
- **Control capability**: the ability for a runner to start, stop, or restart
  nodes, used by chaos workloads.
- **Slot duration**: time interval between consensus rounds in Cryptarchia. Blocks
  are produced at multiples of the slot duration based on lottery outcomes.
- **Block cadence**: observed rate of block production in a live network, measured
  in blocks per second or seconds per block.
- **Cooldown**: waiting period after a chaos action (e.g., node restart) before
  triggering the next action, allowing the system to stabilize.
- **Run window**: total duration a scenario executes, specified via
  `with_run_duration()`. Framework auto-extends to at least 2× slot duration.
- **Readiness probe**: health check performed by runners to ensure nodes are
  reachable and responsive before starting workloads. Prevents false negatives
  from premature traffic.
- **Liveness**: property that the system continues making progress (producing
  blocks) under specified conditions. Contrasts with safety/correctness which
  verifies that state transitions are accurate.
- **State assertion**: expectation that verifies specific values in the system
  state (e.g., wallet balances, UTXO sets) rather than just progress signals.
  Also called "correctness expectations."
- **Mantle transaction**: transaction type in Logos that can contain UTXO transfers
  (LedgerTx) and operations (Op).
- **POL_PROOF_DEV_MODE**: environment variable that disables expensive Groth16 zero-knowledge
  proof generation for leader election. **Required for all runners** (local, compose, k8s)
  for practical testing—without it, proof generation causes timeouts. Should never be
  used in production environments.

---

## External Resources

- **[Logos Project Documentation](https://nomos-tech.notion.site/project)** — Protocol specifications, node internals, and architecture details
