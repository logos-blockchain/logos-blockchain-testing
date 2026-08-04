#[cfg(unix)]
use std::time::Instant;
use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

use super::{
    BinaryProvider, BinaryProviderError, BinaryProviderRef, BuildBinaryProvider, BuildCommand,
    DownloadBinaryProvider, DownloadChecksum, DownloadUrl, FallbackBinaryProvider,
    PathBinaryProvider,
};

#[tokio::test]
async fn resolves_configured_absolute_path() {
    let temp = TempDir::new().expect("temp dir");
    let binary = temp.path().join("node");
    write_file(&binary, b"binary");

    let path = PathBinaryProvider::new(&binary)
        .resolve()
        .await
        .expect("path provider resolves");

    assert_eq!(path, binary);
}

#[tokio::test]
async fn rejects_relative_configured_path() {
    let error = PathBinaryProvider::new("relative-node")
        .resolve()
        .await
        .expect_err("relative path is rejected");

    assert!(matches!(error, BinaryProviderError::RelativePath { .. }));
}

#[tokio::test]
async fn resolves_first_available_fallback_provider() {
    let temp = TempDir::new().expect("temp dir");
    let binary = temp.path().join("node");
    write_file(&binary, b"binary");

    let providers: Vec<BinaryProviderRef> = vec![
        Arc::new(PathBinaryProvider::new(temp.path().join("missing-node"))),
        Arc::new(PathBinaryProvider::new(&binary)),
    ];
    let provider = FallbackBinaryProvider::new(providers);
    let path = provider
        .resolve()
        .await
        .expect("fallback provider resolves");

    assert_eq!(path, binary);
}

#[tokio::test]
async fn fallback_reuses_inner_provider_cache() {
    let temp = TempDir::new().expect("temp dir");
    let binary = temp.path().join("node");
    write_file(&binary, b"binary");

    let resolve_count = Arc::new(AtomicUsize::new(0));
    let cached_provider: BinaryProviderRef = Arc::new(CountingBinaryProvider::new(
        &binary,
        Arc::clone(&resolve_count),
    ));
    let first = FallbackBinaryProvider::new([
        missing_binary_provider(temp.path().join("missing-first")),
        Arc::clone(&cached_provider),
    ]);
    let second = FallbackBinaryProvider::new([
        missing_binary_provider(temp.path().join("missing-second")),
        cached_provider,
    ]);

    assert_eq!(
        first.resolve().await.expect("first fallback resolves"),
        binary
    );
    assert_eq!(
        second.resolve().await.expect("second fallback resolves"),
        binary
    );
    assert_eq!(resolve_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn runs_build_command_and_returns_output_path() {
    let temp = TempDir::new().expect("temp dir");
    let output = temp.path().join("built-node");
    let script = temp.path().join("build.sh");
    write_file(
        &script,
        format!("#!/bin/sh\nprintf built > '{}'\n", output.display()).as_bytes(),
    );

    let provider = BuildBinaryProvider {
        command: BuildCommand::new("sh").with_args([script.to_string_lossy().to_string()]),
        output_path: output.clone(),
        working_dir: Some(temp.path().to_owned()),
        lock_dir: Some(temp.path().join("locks")),
    };
    let path = provider.resolve().await.expect("build provider resolves");

    assert_eq!(path, output);
    assert_eq!(fs::read(path).expect("built file"), b"built");
}

#[tokio::test]
async fn build_provider_runs_even_when_output_exists() {
    let temp = TempDir::new().expect("temp dir");
    let output = temp.path().join("built-node");
    let script = temp.path().join("build.sh");
    write_file(&output, b"old");
    write_file(
        &script,
        format!("#!/bin/sh\nprintf new > '{}'\n", output.display()).as_bytes(),
    );

    let provider = BuildBinaryProvider {
        command: BuildCommand::new("sh").with_args([script.to_string_lossy().to_string()]),
        output_path: output.clone(),
        working_dir: Some(temp.path().to_owned()),
        lock_dir: Some(temp.path().join("locks")),
    };
    let path = provider.resolve().await.expect("build provider resolves");

    assert_eq!(path, output);
    assert_eq!(fs::read(path).expect("built file"), b"new");
}

#[tokio::test]
async fn fails_when_build_command_does_not_create_output() {
    let temp = TempDir::new().expect("temp dir");
    let output = temp.path().join("missing-node");
    let provider = BuildBinaryProvider {
        command: BuildCommand::new("sh").with_args(["-c", "true"]),
        output_path: output,
        working_dir: Some(temp.path().to_owned()),
        lock_dir: Some(temp.path().join("locks")),
    };

    let error = provider
        .resolve()
        .await
        .expect_err("missing build output is rejected");

    assert!(matches!(
        error,
        BinaryProviderError::MissingBuildOutput { .. }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_build_resolution_stops_the_build_process() {
    let temp = TempDir::new().expect("temp dir");
    let output = temp.path().join("built-node");
    let pid_file = temp.path().join("build.pid");

    let provider = BuildBinaryProvider {
        command: BuildCommand::new("sh")
            .with_args(["-c", "printf '%s' \"$$\" > build.pid; exec sleep 60"]),
        output_path: output,
        working_dir: Some(temp.path().to_owned()),
        lock_dir: Some(temp.path().join("locks")),
    };
    let resolution = tokio::spawn(async move { provider.resolve().await });
    let pid = wait_for_pid(&pid_file).await;

    resolution.abort();
    let error = resolution
        .await
        .expect_err("resolution should be cancelled");
    assert!(error.is_cancelled());
    assert!(
        wait_until_process_exits(pid, Duration::from_secs(2)),
        "build process {pid} should exit when resolution is cancelled"
    );
}

#[tokio::test]
async fn downloads_binary_from_minimal_http_server() {
    let temp = TempDir::new().expect("temp dir");
    let body = b"downloaded-node";
    let server = SingleResponseServer::start(body);

    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed(sha256_hex(body))),
        cache_dir: Some(temp.path().join("cache")),
        processor: None,
    };
    let path = provider
        .resolve()
        .await
        .expect("download provider resolves");

    assert_eq!(fs::read(path).expect("downloaded file"), body);
}

#[tokio::test]
async fn concurrent_downloads_share_one_cached_artifact() {
    let temp = TempDir::new().expect("temp dir");
    let body = b"downloaded-node";

    let mut server = SingleResponseServer::start_paused(body);
    let url = server.url();
    let cache_dir = temp.path().join("cache");
    let checksum = sha256_hex(body);

    let first_provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(url.clone()),
        sha256: Some(DownloadChecksum::Fixed(checksum.clone())),
        cache_dir: Some(cache_dir.clone()),
        processor: None,
    };

    let second_provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(url),
        sha256: Some(DownloadChecksum::Fixed(checksum)),
        cache_dir: Some(cache_dir),
        processor: None,
    };

    let first = tokio::spawn(async move { first_provider.resolve().await });
    server.wait_for_request().await;

    let second = tokio::spawn(async move { second_provider.resolve().await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!second.is_finished(), "second resolution should wait");

    server.release_response();
    let (first, second) = tokio::time::timeout(Duration::from_secs(2), async {
        (first.await, second.await)
    })
    .await
    .expect("concurrent resolutions timed out");

    let first = first.expect("first task failed").expect("first resolution");
    let second = second
        .expect("second task failed")
        .expect("second resolution");

    assert_eq!(first, second);
    assert_eq!(fs::read(first).expect("downloaded file"), body);
}

#[tokio::test]
async fn rejects_download_checksum_mismatch() {
    let temp = TempDir::new().expect("temp dir");
    let server = SingleResponseServer::start(b"downloaded-node");
    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed("00".repeat(32))),
        cache_dir: Some(temp.path().join("cache")),
        processor: None,
    };

    let error = provider
        .resolve()
        .await
        .expect_err("checksum mismatch is rejected");

    assert!(matches!(
        error,
        BinaryProviderError::ChecksumMismatch { .. }
    ));
}

#[tokio::test]
async fn processes_downloaded_artifact_before_publishing_binary() {
    let temp = TempDir::new().expect("temp dir");
    let body = b"archive:downloaded-node";
    let server = SingleResponseServer::start(body);
    let process_count = Arc::new(AtomicUsize::new(0));
    let callback_count = Arc::clone(&process_count);
    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed(sha256_hex(body))),
        cache_dir: Some(temp.path().join("cache")),
        processor: None,
    }
    .with_processor_fn("strip-test-archive-v1", move |artifact, output| {
        callback_count.fetch_add(1, Ordering::SeqCst);
        let contents = fs::read(artifact)?;
        fs::write(
            output,
            contents.strip_prefix(b"archive:").unwrap_or(&contents),
        )?;
        Ok(())
    });

    let path = provider
        .resolve()
        .await
        .expect("processed download resolves");

    assert_eq!(
        fs::read(path).expect("processed binary"),
        b"downloaded-node"
    );
    assert_eq!(process_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejects_processor_that_does_not_create_output() {
    let temp = TempDir::new().expect("temp dir");
    let body = b"archive";
    let server = SingleResponseServer::start(body);
    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed(sha256_hex(body))),
        cache_dir: Some(temp.path().join("cache")),
        processor: None,
    }
    .with_processor_fn("empty-v1", |_artifact, _output| Ok(()));

    let error = provider
        .resolve()
        .await
        .expect_err("missing processed output is rejected");

    assert!(matches!(
        error,
        BinaryProviderError::MissingProcessedOutput { .. }
    ));
}

fn write_file(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("write file");
}

#[cfg(unix)]
async fn wait_for_pid(path: &Path) -> u32 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(pid) = fs::read_to_string(path) {
            return pid.parse().expect("valid process id");
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "build process did not publish its id"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
fn wait_until_process_exits(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    !process_exists(pid)
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn missing_binary_provider(path: PathBuf) -> BinaryProviderRef {
    Arc::new(PathBinaryProvider::new(path))
}

struct CountingBinaryProvider {
    path: PathBuf,
    resolve_count: Arc<AtomicUsize>,
}

impl CountingBinaryProvider {
    fn new(path: &Path, resolve_count: Arc<AtomicUsize>) -> Self {
        Self {
            path: path.to_owned(),
            resolve_count,
        }
    }
}

#[async_trait::async_trait]
impl BinaryProvider for CountingBinaryProvider {
    async fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        self.resolve_count.fetch_add(1, Ordering::SeqCst);

        Ok(Some(self.path.clone()))
    }

    fn display(&self) -> String {
        "counting".to_owned()
    }

    fn cache_key(&self) -> String {
        format!("counting:{}", self.path.display())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct SingleResponseServer {
    addr: String,
    request_started: Option<tokio::sync::oneshot::Receiver<()>>,
    release_response: Option<std::sync::mpsc::Sender<()>>,
}

impl SingleResponseServer {
    fn start(body: &'static [u8]) -> Self {
        let mut server = Self::start_paused(body);
        server.release_response();
        server
    }

    fn start_paused(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let addr = listener.local_addr().expect("server addr").to_string();
        let (request_started_tx, request_started_rx) = tokio::sync::oneshot::channel();
        let (release_response_tx, release_response_rx) = std::sync::mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one request");
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let _ = request_started_tx.send(());
            release_response_rx.recv().expect("release response");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );

            stream
                .write_all(response.as_bytes())
                .expect("write headers");
            stream.write_all(body).expect("write body");
        });

        Self {
            addr,
            request_started: Some(request_started_rx),
            release_response: Some(release_response_tx),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/binary", self.addr)
    }

    async fn wait_for_request(&mut self) {
        self.request_started
            .take()
            .expect("request receiver")
            .await
            .expect("request started");
    }

    fn release_response(&mut self) {
        self.release_response
            .take()
            .expect("response sender")
            .send(())
            .expect("release response");
    }
}
