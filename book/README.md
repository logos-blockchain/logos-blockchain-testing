# Documentation Maintenance Guide

This guide helps maintainers keep the book synchronized with code changes. Use these checklists when modifying the framework.

**Key Tool:** The `examples/doc-snippets/` crate contains compilable versions of code examples from the book. Always run `cargo build -p doc-snippets` after API changes to catch broken examples early.

## Quick Reference: What to Update When

| Change Type | Pages to Check | Estimated Time |
|-------------|----------------|----------------|
| API method renamed/changed | [API Changes](#api-changes) | 1-2 hours |
| New workload/expectation added | [New Features](#new-features) | 30 minutes |
| Environment variable added/changed | [Environment Variables](#environment-variables) | 15 minutes |
| Script path/interface changed | [Scripts & Tools](#scripts--tools) | 30 minutes |
| New runner/deployer added | [New Runner](#new-runner) | 2-3 hours |
| Trait signature changed | [Trait Changes](#trait-changes) | 1-2 hours |

---

## Detailed Checklists

### API Changes

**When:** Builder API methods, trait methods, or core types change

**Examples:**
- Rename: `.transactions_with()` → `.with_transactions()`
- New method: `.with_timeout()`
- Parameter change: `.validators(3)` → `.validators(count, config)`

**Update these pages:**

```bash
# 1. Search for affected API usage
rg "old_method_name" book/src/

# 2. Update these files:
- [ ] src/dsl-cheat-sheet.md          # Builder API reference (highest priority)
- [ ] src/quickstart.md                # First example users see
- [ ] src/examples.md                  # 4 complete scenarios
- [ ] src/examples-advanced.md         # 3 advanced scenarios
- [ ] src/introduction.md              # "A Scenario in 20 Lines" example
- [ ] src/project-context-primer.md    # Quick example section
- [ ] src/authoring-scenarios.md       # Scenario patterns
- [ ] src/best-practices.md            # Code organization examples
- [ ] src/custom-workload-example.md   # Complete implementation
- [ ] src/extending.md                 # Trait implementation examples
```

**Verification:**
```bash
# Compile doc-snippets to catch API breakage
cargo build -p doc-snippets

# Check if book links are valid
mdbook test
```

---

### New Features

#### New Workload or Expectation

**When:** Adding a new traffic generator or success criterion

**Examples:**
- New workload: `MemoryPressureWorkload`
- New expectation: `ExpectZeroDroppedTransactions`

**Update these pages:**

```bash
- [ ] src/workloads.md                 # Add to built-in workloads section
- [ ] src/dsl-cheat-sheet.md          # Add DSL helper if provided
- [ ] src/examples-advanced.md         # Consider adding example usage
- [ ] src/glossary.md                  # Add term definition
- [ ] src/internal-crate-reference.md  # Document crate location
```

**Optional (if significant feature):**
```bash
- [ ] src/what-you-will-learn.md       # Add to learning outcomes
- [ ] src/best-practices.md            # Add usage guidance
```

#### New Runner/Deployer

**When:** Adding support for a new deployment target (e.g., AWS ECS)

**Update these pages:**

```bash
# Core documentation
- [ ] src/runners.md                   # Add to comparison table and decision guide
- [ ] src/operations-overview.md       # Update runner-agnostic matrix
- [ ] src/architecture-overview.md     # Update deployer list and diagram
- [ ] src/running-examples.md          # Add runner-specific section

# Reference pages
- [ ] src/dsl-cheat-sheet.md          # Add deployer import/usage
- [ ] src/internal-crate-reference.md  # Document new crate
- [ ] src/glossary.md                  # Add runner type definition

# Potentially affected
- [ ] src/ci-integration.md            # Add CI example if applicable
- [ ] src/troubleshooting.md           # Add common issues
- [ ] src/faq.md                       # Add FAQ entries
```

#### New Topology Helper

**When:** Adding topology generation helpers (e.g., `.network_mesh()`)

**Update these pages:**

```bash
- [ ] src/dsl-cheat-sheet.md          # Add to topology section
- [ ] src/authoring-scenarios.md       # Add usage pattern
- [ ] src/topology-chaos.md            # Add topology description
- [ ] src/examples.md                  # Consider adding example
```

---

### Trait Changes

**When:** Core trait signatures change (breaking changes)

**Examples:**
- `Workload::init()` adds new parameter
- `Expectation::evaluate()` changes return type
- `Deployer::deploy()` signature update

**Update these pages:**

```bash
# Critical - these show full trait definitions
- [ ] src/extending.md                 # Complete trait outlines (6+ examples)
- [ ] src/custom-workload-example.md   # Full implementation example
- [ ] src/scenario-model.md            # Core model documentation

# Important - these reference traits
- [ ] src/api-levels.md                # Trait usage patterns
- [ ] src/architecture-overview.md     # Extension points diagram
- [ ] src/internal-crate-reference.md  # Trait locations
```

**Verification:**
```bash
# Ensure trait examples would compile
cargo doc --no-deps --document-private-items
```

---

### Environment Variables

**When:** New environment variable added, changed, or removed

**Examples:**
- New: `NOMOS_NEW_FEATURE_ENABLED`
- Changed: `NOMOS_LOG_LEVEL` accepts new values
- Deprecated: `OLD_FEATURE_FLAG`

**Update these pages:**

```bash
# Primary location (single source of truth)
- [ ] src/environment-variables.md     # Add to appropriate category table

# Secondary mentions
- [ ] src/prerequisites.md             # If affects setup
- [ ] src/running-examples.md          # If affects runner usage
- [ ] src/troubleshooting.md           # If commonly misconfigured
- [ ] src/glossary.md                  # If significant/commonly referenced
```

**Environment Variables Table Location:**
```
src/environment-variables.md
  ├─ Runner Configuration
  ├─ Node Binary & Paths
  ├─ Circuit Assets
  ├─ Logging & Tracing
  ├─ Observability & Metrics
  ├─ Proof System
  ├─ Docker & Images
  ├─ Testing Behavior
  └─ CI/CD
```

---

### Scripts & Tools

**When:** Helper scripts move, rename, or change interface

**Examples:**
- Script moved: `scripts/run-examples.sh` → `scripts/run/run-examples.sh`
- New script: `scripts/clean-all.sh`
- Interface change: `run-examples.sh` adds new required flag

**Update these pages:**

```bash
# High impact
- [ ] src/quickstart.md                # Uses run-examples.sh prominently
- [ ] src/running-examples.md          # Documents all scripts
- [ ] src/prerequisites.md             # References setup scripts
- [ ] src/examples.md                  # Script recommendations
- [ ] src/examples-advanced.md         # Script recommendations

# Moderate impact
- [ ] src/ci-integration.md            # May reference scripts in workflows
- [ ] src/troubleshooting.md           # Cleanup scripts
- [ ] src/architecture-overview.md     # Asset preparation scripts
```

**Find all script references:**
```bash
rg "scripts/" book/src/ --no-heading
```

---

### Operational Changes

#### Docker Image Changes

**When:** Image build process, tag names, or embedded assets change

**Update these pages:**

```bash
- [ ] src/prerequisites.md             # Image build instructions
- [ ] src/runners.md                   # Compose/K8s prerequisites
- [ ] src/environment-variables.md     # NOMOS_TESTNET_IMAGE, NOMOS_BINARIES_TAR
- [ ] src/architecture-overview.md     # Assets and Images section
```

#### Observability Stack Changes

**When:** Prometheus, Grafana, OTLP, or metrics configuration changes

**Update these pages:**

```bash
- [ ] src/logging-observability.md     # Primary documentation
- [ ] src/environment-variables.md     # NOMOS_METRICS_*, NOMOS_OTLP_*
- [ ] src/architecture-overview.md     # Observability section
- [ ] src/runners.md                   # Runner observability support
```

#### CI/CD Changes

**When:** CI workflow changes, new actions, or integration patterns

**Update these pages:**

```bash
- [ ] src/ci-integration.md            # Complete workflow examples
- [ ] src/best-practices.md            # CI recommendations
- [ ] src/operations-overview.md       # CI mentioned in runner matrix
```

---

### Node Protocol Changes

**When:** Changes to Logos blockchain protocol or node behavior

**Examples:**
- New consensus parameter
- DA protocol change
- Network layer update

**Update these pages:**

```bash
# Context pages (high-level only)
- [ ] src/project-context-primer.md    # Protocol overview
- [ ] src/glossary.md                  # Protocol terms
- [ ] src/faq.md                       # May need protocol updates

# Usually NOT affected (framework is protocol-agnostic)
- Testing framework abstracts protocol details
- Only update if change affects testing methodology
```

---

### Crate Structure Changes

**When:** Crate reorganization, renames, or new crates added

**Examples:**
- New crate: `testing-framework-metrics`
- Crate rename: `runner-examples` → `examples`
- Module moved: `core::scenario` → `core::model`

**Update these pages:**

```bash
# Critical
- [ ] src/internal-crate-reference.md  # Complete crate listing
- [ ] src/architecture-overview.md     # Crate dependency diagram
- [ ] src/workspace-layout.md          # Directory structure
- [ ] src/annotated-tree.md            # File tree with annotations

# Code examples (update imports)
- [ ] src/dsl-cheat-sheet.md          # Import statements
- [ ] src/extending.md                 # use statements in examples
- [ ] src/custom-workload-example.md   # Full imports
```

**Find all import statements:**
```bash
rg "^use testing_framework" book/src/
```

---

## Testing Documentation Changes

### Build the Book

```bash
cd book
mdbook build

# Output: ../target/book/
```

### Test Documentation

```bash
# Check for broken links
mdbook test

# Preview locally
mdbook serve
# Open http://localhost:3000
```

### Test Code Examples (Doc Snippets)

**The `examples/doc-snippets/` crate contains compilable versions of code examples from the book.**

This ensures examples stay synchronized with the actual API and don't break when code changes.

**Why doc-snippets exist:**
- Code examples in the book (73 blocks across 18 files) can drift from reality
- Compilation failures catch API breakage immediately
- Single source of truth for code examples

**Current coverage:** 40+ snippet files corresponding to examples in:
- `quickstart.md` (7 snippets)
- `examples.md` (4 scenarios)
- `examples-advanced.md` (3 scenarios)
- `dsl-cheat-sheet.md` (11 DSL examples)
- `custom-workload-example.md` (2 trait implementations)
- `internal-crate-reference.md` (6 extension examples)
- And more...

**Testing snippets:**

```bash
# Compile all doc snippets
cargo build -p doc-snippets

# Run with full warnings
cargo build -p doc-snippets --all-features

# Check during CI
cargo check -p doc-snippets
```

**When to update snippets:**

1. **API method changed** → Update corresponding snippet file
   ```bash
   # Example: If .transactions_with() signature changes
   # Update: examples/doc-snippets/src/examples_transaction_workload.rs
   ```

2. **New code example added to book** → Create new snippet file
   ```bash
   # Example: Adding new topology pattern
   # Create: examples/doc-snippets/src/topology_mesh_example.rs
   ```

3. **Trait signature changed** → Update trait implementation snippets
   ```bash
   # Update: custom_workload_example_*.rs
   # Update: internal_crate_reference_add_*.rs
   ```

**Snippet naming convention:**
```
book/src/examples.md → examples_*.rs
book/src/quickstart.md → quickstart_*.rs
book/src/dsl-cheat-sheet.md → dsl_cheat_sheet_*.rs
```

**Best practice:**
When updating code examples in markdown, update the corresponding snippet file first, verify it compiles, then copy to the book. This ensures examples are always valid.

### Check for Common Issues

```bash
# Find outdated API references
rg "old_deprecated_api" src/

# Find broken GitHub links
rg "github.com.*404" src/

# Find TODO/FIXME markers
rg "(TODO|FIXME|XXX)" src/

# Check for inconsistent terminology
rg "(Nomos node|nomos blockchain)" src/  # Should be "Logos"
```

---

## Maintenance Schedule

### On Every PR

- [ ] Check if changes affect documented APIs
- [ ] Update relevant pages per checklist above
- [ ] Update corresponding doc-snippets if code examples changed
- [ ] Run `cargo build -p doc-snippets` to verify examples compile
- [ ] Build book to verify no broken links
- [ ] Verify code examples still make sense

### Monthly

- [ ] Review recent PRs for documentation impact
- [ ] Update environment variables table
- [ ] Check script references are current
- [ ] Verify GitHub source links are not 404

### Quarterly

- [ ] Full audit of code examples against latest API
- [ ] Verify all doc-snippets still compile with latest dependencies
- [ ] Check for code examples in book that don't have corresponding snippets
- [ ] Review troubleshooting for new patterns
- [ ] Update FAQ with common questions
- [ ] Check all Mermaid diagrams render correctly

### Major Release

- [ ] Complete review of all technical content
- [ ] Verify all version-specific references
- [ ] Update "What You Will Learn" outcomes
- [ ] Add release notes for documentation changes

---

## Content Organization

### Stability Tiers (Change Frequency)

**Stable (Rarely Change)**
- Part I — Foundations (philosophy, architecture, design rationale)
- Part VI — Appendix (glossary, FAQ, troubleshooting symptoms)
- Front matter (project context, introduction)

**Semi-Stable (Occasional Changes)**
- Part II — User Guide (usage patterns, best practices, examples)
- Part V — Operations (prerequisites, CI, logging)

**High Volatility (Frequent Changes)**
- API references (dsl-cheat-sheet.md, extending.md)
- Code examples (73 blocks across 18 files)
- Environment variables (50+ documented)
- Runner comparisons (features evolve)

### Page Dependency Map

**Core pages** (many other pages reference these):
- `dsl-cheat-sheet.md` ← Referenced by examples, quickstart, authoring
- `environment-variables.md` ← Referenced by operations, troubleshooting, runners
- `runners.md` ← Referenced by operations, quickstart, examples
- `glossary.md` ← Referenced throughout the book

**When updating core pages, check for broken cross-references.**

---

## Common Patterns

### Adding a Code Example

```markdown
# 1. Add the code block
```rust
use testing_framework_core::scenario::ScenarioBuilder;
// ... example code
```

# 2. Add context
**When to use:** [explain use case]

# 3. Link to complete source (if applicable)
[View in source](https://github.com/logos-blockchain/logos-blockchain-testing/blob/master/examples/src/bin/example.rs)
```

### Adding a Cross-Reference

```markdown
See [Environment Variables](environment-variables.md) for complete configuration reference.
```

### Adding a "When to Read" Callout

```markdown
> **When should I read this?** [guidance on when this content is relevant]
```

---

## Contact & Questions

When in doubt:
1. Check this README for guidance
2. Review recent similar changes in git history
3. Ask the team in technical documentation discussions

**Remember:** Documentation quality directly impacts framework adoption and user success. Taking time to update docs properly is an investment in the project's future.
