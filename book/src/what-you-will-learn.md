# What You Will Learn

This book gives you a clear mental model for Logos multi-node testing, shows how
to author scenarios that pair realistic workloads with explicit expectations,
and guides you to run them across local, containerized, and cluster environments
without changing the plan.

## By the End of This Book, You Will Be Able To:

**Understand the Framework**
- Explain the six-phase scenario lifecycle (Build, Deploy, Capture, Execute, Evaluate, Cleanup)
- Describe how Deployers, Runners, Workloads, and Expectations work together
- Navigate the crate architecture and identify extension points
- Understand when to use each runner (Host, Compose, Kubernetes)

**Author and Run Scenarios**
- Define multi-node topologies with validators
- Configure transaction and DA workloads with appropriate rates
- Add consensus liveness and inclusion expectations
- Run scenarios across all three deployment modes
- Use BlockFeed to monitor block production in real-time
- Implement chaos testing with node restarts

**Operate in Production**
- Set up prerequisites and dependencies correctly
- Configure environment variables for different runners
- Integrate tests into CI/CD pipelines (GitHub Actions)
- Troubleshoot common failure scenarios
- Collect and analyze logs from multi-node runs
- Optimize test durations and resource usage

**Extend the Framework**
- Implement custom Workload traits for new traffic patterns
- Create custom Expectation traits for domain-specific checks
- Add new Deployer implementations for different backends
- Contribute topology helpers and DSL extensions

## Learning Path

**Beginner** (0-2 hours)
- Read [Quickstart](quickstart.md) and run your first scenario
- Review [Examples](examples.md) to see common patterns
- Understand [Scenario Lifecycle](scenario-lifecycle.md) phases

**Intermediate** (2-8 hours)
- Study [Runners](runners.md) comparison and choose appropriate mode
- Learn [Workloads & Expectations](workloads.md) in depth
- Review [Prerequisites & Setup](prerequisites.md) for your environment
- Practice with [Advanced Examples](examples-advanced.md)

**Advanced** (8+ hours)
- Master [Environment Variables](environment-variables.md) configuration
- Implement [Custom Workloads](extending.md) for your use cases
- Set up [CI Integration](ci-integration.md) for automated testing
- Explore [Internal Crate Reference](internal-crate-reference.md) for deep dives

## What This Book Does NOT Cover

- **Logos node internals** — This book focuses on testing infrastructure, not the blockchain protocol implementation. See the Logos node repository (`nomos-node`) for protocol documentation.
- **Consensus algorithm theory** — We assume familiarity with basic blockchain concepts (validators, blocks, transactions, data availability).
- **Rust language basics** — Examples use Rust, but we don't teach the language. See [The Rust Book](https://doc.rust-lang.org/book/) if you're new to Rust.
- **Kubernetes administration** — We show how to use the K8s runner, but don't cover cluster setup, networking, or operations.
- **Docker fundamentals** — We assume basic Docker/Compose knowledge for the Compose runner.
