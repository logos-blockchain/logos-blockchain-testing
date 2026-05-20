use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::Path,
    sync::Arc,
    thread,
};

use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

use super::{
    BinaryProvider, BinaryProviderError, BinaryProviderRef, BuildBinaryProvider, BuildCommand,
    DownloadBinaryProvider, DownloadChecksum, DownloadUrl, FallbackBinaryProvider,
    PathBinaryProvider,
};

#[test]
fn resolves_configured_absolute_path() {
    let temp = TempDir::new().expect("temp dir");
    let binary = temp.path().join("node");
    write_file(&binary, b"binary");

    let path = PathBinaryProvider::new(&binary)
        .resolve()
        .expect("path provider resolves");

    assert_eq!(path, binary);
}

#[test]
fn rejects_relative_configured_path() {
    let error = PathBinaryProvider::new("relative-node")
        .resolve()
        .expect_err("relative path is rejected");

    assert!(matches!(error, BinaryProviderError::RelativePath { .. }));
}

#[test]
fn resolves_first_available_fallback_provider() {
    let temp = TempDir::new().expect("temp dir");
    let binary = temp.path().join("node");
    write_file(&binary, b"binary");

    let providers: Vec<BinaryProviderRef> = vec![
        Arc::new(PathBinaryProvider::new(temp.path().join("missing-node"))),
        Arc::new(PathBinaryProvider::new(&binary)),
    ];
    let provider = FallbackBinaryProvider::new(providers);
    let path = provider.resolve().expect("fallback provider resolves");

    assert_eq!(path, binary);
}

#[test]
fn runs_build_command_and_returns_output_path() {
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
    let path = provider.resolve().expect("build provider resolves");

    assert_eq!(path, output);
    assert_eq!(fs::read(path).expect("built file"), b"built");
}

#[test]
fn build_provider_runs_even_when_output_exists() {
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
    let path = provider.resolve().expect("build provider resolves");

    assert_eq!(path, output);
    assert_eq!(fs::read(path).expect("built file"), b"new");
}

#[test]
fn fails_when_build_command_does_not_create_output() {
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
        .expect_err("missing build output is rejected");

    assert!(matches!(
        error,
        BinaryProviderError::MissingBuildOutput { .. }
    ));
}

#[test]
fn downloads_binary_from_minimal_http_server() {
    let temp = TempDir::new().expect("temp dir");
    let body = b"downloaded-node";
    let server = SingleResponseServer::start(body);

    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed(sha256_hex(body))),
        cache_dir: Some(temp.path().join("cache")),
    };
    let path = provider.resolve().expect("download provider resolves");

    assert_eq!(fs::read(path).expect("downloaded file"), body);
}

#[test]
fn rejects_download_checksum_mismatch() {
    let temp = TempDir::new().expect("temp dir");
    let server = SingleResponseServer::start(b"downloaded-node");
    let provider = DownloadBinaryProvider {
        url: DownloadUrl::Fixed(server.url()),
        sha256: Some(DownloadChecksum::Fixed("00".repeat(32))),
        cache_dir: Some(temp.path().join("cache")),
    };

    let error = provider
        .resolve()
        .expect_err("checksum mismatch is rejected");

    assert!(matches!(
        error,
        BinaryProviderError::ChecksumMismatch { .. }
    ));
}

fn write_file(path: &Path, contents: &[u8]) {
    fs::write(path, contents).expect("write file");
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct SingleResponseServer {
    addr: String,
}

impl SingleResponseServer {
    fn start(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let addr = listener.local_addr().expect("server addr").to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept one request");
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );

            stream
                .write_all(response.as_bytes())
                .expect("write headers");
            stream.write_all(body).expect("write body");
        });

        Self { addr }
    }

    fn url(&self) -> String {
        format!("http://{}/binary", self.addr)
    }
}
