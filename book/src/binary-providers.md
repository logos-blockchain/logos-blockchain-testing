# Binary Providers

Binary providers resolve the executable a local node process runs. The source can be a path, an env var, a build command, a download, or an ordered fallback chain.

Every local node launch has exactly one provider selected on its `LocalProcessSpec`. Providers live in `testing_framework_runner_local::binary` and are re-exported from the crate root. They apply to the [Local Deployer](deployer-local.md) only; compose and k8s nodes run container images instead (see the [Capability Matrix](capability-matrix.md)).

---

## The Trait

```rust,ignore
pub trait BinaryProvider: Send + Sync {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError>;
    fn display(&self) -> String;
    fn cache_key(&self) -> String;

    // Provided: cache lookup, then resolve_uncached.
    fn resolve(&self) -> Result<PathBuf, BinaryProviderError> { /* ... */ }
    fn resolve_uncached(&self) -> Result<PathBuf, BinaryProviderError> { /* ... */ }
}

pub type BinaryProviderRef = Arc<dyn BinaryProvider>;
```

`try_resolve` returns `Ok(None)` when the provider is valid but cannot produce a binary in the current environment, which is not an error. Standalone, `resolve` turns `None` into `BinaryProviderError::NotFound`; inside a `FallbackBinaryProvider`, `None` means "try the next provider". Other errors (a failed build, a checksum mismatch) abort resolution immediately.

---

## The Providers

| Provider | Resolves from | Unresolved (`None`) when |
|---|---|---|
| `PathBinaryProvider` | A fixed absolute path | Path is not a file (relative paths are an error) |
| `EnvBinaryProvider` | An env var containing a path | Var unset or not pointing at a file |
| `BuildBinaryProvider` | Running a build command | Never — build failure is an error |
| `DownloadBinaryProvider` | Fetching a URL into a cache | Never — download failure is an error |
| `FallbackBinaryProvider` | First chain member to resolve | Every member returned `None` |

**`PathBinaryProvider::new(path)`**: a deterministic explicit path. No filesystem search, no `PATH` lookup.

**`EnvBinaryProvider::new("MY_NODE_BIN")`**: the standard override hook. `LocalProcessSpec::new(env_var)` installs one of these by default.

**`BuildBinaryProvider`** delegates to any command:

```rust,ignore
BuildBinaryProvider {
    command: BuildCommand::new("cargo").with_args(["build", "-p", "kvstore-node"]),
    output_path: "target/debug/kvstore-node".into(), // relative to working_dir
    working_dir: Some(workspace_root),                // default: current dir
    lock_dir: None,                                   // default: <working_dir>/.tf-binaries
}
```

The command is not Cargo-specific; it can invoke a Make target, shell script, or cache fetch. After the command succeeds, the configured `output_path` must exist or resolution fails with `MissingBuildOutput`.

**`DownloadBinaryProvider`** fetches into a cache directory (default `target/.tf-binaries` under the current directory):

- `DownloadUrl::Fixed(url)` or `DownloadUrl::Env(var)` selects the source.
- `DownloadChecksum::Fixed(sha256)` or `DownloadChecksum::Env(var)` enables SHA-256 verification; mismatches fail with `ChecksumMismatch` before anything is written to the final path.
- A `DownloadProcessor` post-processes artifacts that are not directly executable (archives, bundles). It receives the verified download and must materialize the executable at the output path. `DownloadProcessorFn::new(cache_key, closure)` (or `.with_processor_fn(...)`) is the lightweight adapter; the `cache_key` is part of cache identity, so changing your extraction logic invalidates previously prepared binaries.
- On Unix, the result is marked executable (`0o755`). Downloads are staged through temporary `.download`/`.part` files and renamed into place.

**`FallbackBinaryProvider::new([a, b, ...])`**: an ordered chain, tried first to last. From the launch spec's perspective it is still a single provider.

---

## Caching and Cache Identity

Successful resolutions are cached **per process** in a global map keyed by `cache_key()`, so repeated node starts with the same provider config do not rebuild, redownload, or re-scan. Cache identity encodes the full request:

- `path:<path>`, `env:<var>`
- `build:<command>:<output_path>:<working_dir>`
- `download:<url-or-env>:<checksum-or-env>:<processor-key>:<cache_dir>`
- fallback: the members' keys joined with commas

Change any component and you get a fresh resolution. The download provider also caches on disk: the cached file name hashes the URL, resolved checksum, and processor key, so an already-downloaded binary is reused across processes without refetching.

---

## Concurrent Resolution Locking

Builds and downloads may be triggered by several test processes at once (e.g. `cargo nextest` running integration tests in parallel). Providers that materialize files take a **cross-process file lock** before doing work: a lock file created with `create_new` under `.tf-binaries` (build) or the download cache dir, retried every 200 ms for up to 10 minutes, then `BinaryProviderError::LockTimeout`. The lock file is removed when the guard drops.

A killed test process can leave a stale lock file behind. If resolution hangs and then times out, look for leftover `*.lock` files under `.tf-binaries` and delete them.

---

## Worked Example: kvstore's Fallback Chain

From `examples/kvstore/testing/integration/src/local_env.rs`, which prefers an explicit env override and otherwise builds from source:

```rust,ignore
use std::{path::PathBuf, sync::Arc};
use testing_framework_runner_local::{
    BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LocalProcessSpec,
};

fn kvstore_binary_provider() -> FallbackBinaryProvider {
    let providers: [BinaryProviderRef; 2] = [
        Arc::new(EnvBinaryProvider::new("KVSTORE_NODE_BIN")),
        Arc::new(BuildBinaryProvider {
            command: BuildCommand::new("cargo")
                .with_args(["build", "-p", "kvstore-node", "--bin", "kvstore-node"]),
            output_path: PathBuf::from(format!(
                "target/debug/kvstore-node{}",
                std::env::consts::EXE_SUFFIX
            )),
            working_dir: Some(workspace_root()),
            lock_dir: None,
        }),
    ];
    FallbackBinaryProvider::new(providers)
}

fn local_process_spec() -> LocalProcessSpec {
    LocalProcessSpec::new("KVSTORE_NODE_BIN")
        .with_binary_provider(kvstore_binary_provider())
        .with_rust_log("kvstore_node=info")
}
```

First run: `KVSTORE_NODE_BIN` is unset, the env provider yields `None`, the build provider compiles the node under the workspace lock, and the path is cached for the rest of the process. Set `KVSTORE_NODE_BIN=/path/to/kvstore-node` to skip the build entirely, which is useful for prebuilt release binaries or mixed-version clusters (via `local_process_spec_for_node`, see [Local Deployer](deployer-local.md)).
