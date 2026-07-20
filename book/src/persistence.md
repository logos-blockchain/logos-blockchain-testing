# Persistence, Snapshots, and Recovery Testing

This chapter explains how node working directories behave, what `persist_dir` and `snapshot_dir` actually do, and how to build stop/restore recovery tests on top of them.

---

## The Working Directory

Every locally spawned node runs inside a framework-created directory (`testing-framework/deployers/local/src/process.rs`):

- Launch files (the rendered config) are written into it before spawn, and the process starts with it as its current directory. A node that writes relative paths (a database under `./db`, logs under `./logs`) keeps all its state there.
- By default the directory is a random-named temp directory created **in the current working directory** of the test process, and it is deleted when the node is dropped.
- Deletion is skipped when the owning thread is panicking, when the node was spawned with `keep_tempdir`, when `TF_KEEP_LOGS=1` is set, or when the deployment policy sets `cleanup_policy.preserve_artifacts` (see [Readiness, Retry, and Artifact Preservation](deployment-policies.md)).

`restart` (used by `ManualCluster::restart_node` and `LocalProcessHandle::restart`) kills the child and respawns it **in the same directory** with the same launch spec. Launch files are rewritten; everything else is untouched, so state in the working directory survives the restart.

---

## persist_dir: a Predictable Location

`persist_dir` does **not** reuse the given path as-is. Verified semantics from `create_tempdir`:

- The working directory is created as `<basename>_<random-suffix>` **inside the parent** of the path you pass. `with_persist_dir("/tmp/kv-run/node-0")` yields a working directory like `/tmp/kv-run/node-0_a1B2c3/`. The parent is created if missing.
- Nothing is copied into it; it starts empty apart from launch files.
- It is still a managed temp directory: deleted on drop unless one of the retention switches above applies.

Use `persist_dir` when a test (or a human) must *find* the node's state. Pair it with `TF_KEEP_LOGS=1` or `preserve_artifacts` to keep the directory after the run, then feed it back in as a snapshot later.

---

## snapshot_dir: Seeding State at Start

`snapshot_dir` copies saved state into the fresh working directory before the process spawns. This is the restore half of a recovery test: a fresh node starts from state captured in an earlier run instead of an empty directory. Verified semantics from `copy_snapshot_dir`:

- The directory you pass is copied **as a subdirectory of the working directory, named after its final path component**, with overwrite enabled. `with_snapshot_dir("/snapshots/run1/db")` produces `<workdir>/db/...`.
- Consequently, the snapshot source's basename must match the relative path the node expects. If your node reads `./db`, snapshot a directory literally named `db`.
- The copy happens once, at spawn. Restarts do not re-apply it.
- A failed copy fails the spawn (`ProcessSpawnError::Snapshot`).

The framework copies the supplied directory byte-for-byte without interpreting its contents. The caller determines what constitutes a consistent snapshot, including which directories to copy, whether the node must be stopped first, and whether its on-disk state is crash-consistent.

---

## Where the Options Live

**Per dynamic node**: `StartNodeOptions` (full table in [ManualCluster](manual-cluster.md)):

```rust,ignore
let node = cluster.start_node_with(
    "restored",
    StartNodeOptions::default()
        .with_snapshot_dir(PathBuf::from("/snapshots/run1/db"))
        .with_start_timeout(Duration::from_secs(90)),
).await?;
```

Note `restart_node_with` rejects `persist_dir`/`snapshot_dir` overrides, since restarts keep the existing directory. Start a new node to restore from a snapshot.

**Per initial node**: `LocalDeployerEnv::initial_persist_dir(topology, node_name, index)` and `initial_snapshot_dir(...)` (default `None`). Override these to mount state under the whole initial cluster, e.g. restore every node of a 3-node cluster from a saved dataset before the scenario begins.

**Per composed process**: `LocalProcessApp` in the app layer (`testing-framework/app/src/process.rs`) exposes the same three switches for one-binary apps:

| Builder | Effect |
|---|---|
| `.with_persist_dir(path)` | Same placement rule as above |
| `.with_snapshot_dir(path)` | Same copy-as-subdirectory rule as above |
| `.keep_tempdir(true)` | Retain the working directory on teardown |

Its `LocalProcessHandle` offers `working_dir()`, `restart()`, `stop()`, `is_running()`, `pid()`, and `keep_tempdir()`; the process stops when the last handle clone drops (see [One Binary: LocalProcessApp](local-process-app.md)).

---

## Recovery-Testing Patterns

**Restart in place.** State persists because the node reuses its working directory.

```rust,ignore
write_data(&client).await?;
cluster.restart_node("node-1").await?;
cluster.wait_node_ready("node-1").await?;
assert_data_recovered(&cluster.node_client("node-1").unwrap()).await?;
```

**Stop, snapshot, restore.** Full recovery drill via [ManualCluster](manual-cluster.md):

```rust,ignore
// 1. Run a node whose working dir you can locate.
let node = cluster.start_node_with(
    "primary",
    StartNodeOptions::default().with_persist_dir(PathBuf::from("/tmp/kv-run/primary")),
).await?;
write_data(&node.client).await?;

// 2. Stop it, then copy its state out yourself (caller-owned step):
cluster.stop_node("node-primary").await?;
//    e.g. cp -r /tmp/kv-run/primary_*/db /snapshots/case1/db

// 3. Start a fresh node seeded from the snapshot.
let restored = cluster.start_node_with(
    "restored",
    StartNodeOptions::default().with_snapshot_dir(PathBuf::from("/snapshots/case1/db")),
).await?;
cluster.wait_node_ready("node-restored").await?;
assert_data_recovered(&restored.client).await?;
```

Step 2 is caller-owned because the framework neither snapshots on stop nor knows which files constitute application state. Set `TF_KEEP_LOGS=1` so stopped nodes' directories remain available for copying.

**Config continuity.** The first node started with a `snapshot_dir` has its generated config recorded as a template, and that template is passed to the config-build hooks of later dynamic starts (the `template_config` parameter). An env that honors it can keep restored nodes consistent with the configs their state was produced under.

**Cross-run state.** Combine `persist_dir` (findable location) with retention (`TF_KEEP_LOGS` / `preserve_artifacts`), archive the directory after run A, and hand it to run B via `snapshot_dir` or the `initial_snapshot_dir` hook. Upgrade tests, long-lived-ledger tests, and crash-recovery matrices all reduce to this loop.
