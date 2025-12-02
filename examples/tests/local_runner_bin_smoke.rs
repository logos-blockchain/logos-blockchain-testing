use std::{env, path::Path, process::Command};

// Manually run the local runner binary as a smoke test.
// This spins up real nodes and should be invoked explicitly:
// POL_PROOF_DEV_MODE=true cargo test -p runner-examples --test
// local_runner_bin_smoke -- --ignored --nocapture
#[test]
#[ignore = "runs local_runner binary (~2min) and requires local assets/binaries"]
fn local_runner_bin_smoke() {
    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "runner-examples",
            "--bin",
            "local_runner",
            "--",
            "--nocapture",
        ])
        .env("POL_PROOF_DEV_MODE", "true")
        .env(
            "NOMOS_CIRCUITS",
            env::var("NOMOS_CIRCUITS")
                .or_else(|_| {
                    let default = ".tmp/nomos-circuits";
                    if Path::new(default).exists() {
                        Ok(default.to_string())
                    } else {
                        Err(env::VarError::NotPresent)
                    }
                })
                .expect("NOMOS_CIRCUITS must be set or .tmp/nomos-circuits must exist"),
        )
        .env("LOCAL_DEMO_RUN_SECS", "120")
        .env("LOCAL_DEMO_VALIDATORS", "1")
        .env("LOCAL_DEMO_EXECUTORS", "1")
        .env("RUST_BACKTRACE", "1")
        .output()
        .expect("failed to spawn cargo run");

    if !output.status.success() {
        panic!(
            "local runner binary failed: status={}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
