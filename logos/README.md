# Logos Testing Framework Extension

This directory contains the **Logos-specific extension layer** that plugs into the generic
`testing-framework` core. The goal is to keep all Nomos logic in one place with a clear
structure so it can be reviewed and moved into the `logos-blockchain-node` repo cleanly.

## Layout

- `runtime/env`  
  Logos implementation of the core `Application` trait and runtime wiring.

- `runtime/ext`  
  Logos extension glue for compose/k8s/cfgsync integration and scenario helpers.

- `runtime/workloads`  
  Logos workloads and expectations (e.g., transaction workload, consensus liveness).

- `runtime/cfgsync`  
  Logos cfgsync server/client and config bundling.

- `infra/assets/stack`  
  Docker stack assets, scripts, and monitoring bundles.

- `infra/helm/logos-runner`  
  Helm chart used by the k8s deployer.

## Extension Boundary

The **core** (`testing-framework/*`) remains Logos-agnostic. All app assumptions should
live under `logos/runtime/*` and expose only the minimal surface needed by the core.

If you need to introduce new core capabilities, add them to the core and keep the Logos
implementation in `logos/runtime/*`.
