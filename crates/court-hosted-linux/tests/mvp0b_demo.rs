#![cfg(unix)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/packet-rx")
}

fn unique_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_root(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ck-root"))
        .args(args)
        .env("CK_APP_BIN", env!("CARGO_BIN_EXE_ck-app"))
        .env("CK_NET_BIN", env!("CARGO_BIN_EXE_ck-net"))
        .output()
        .unwrap()
}

fn read_events(trace_path: &Path) -> Vec<Value> {
    std::fs::read_to_string(trace_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect()
}

fn assert_success(output: &std::process::Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_packet_rx_trace(events: &[Value]) {
    assert_event(events, "lookup", "ok");
    assert_event(events, "open_denied", "no_right");
    assert_event(events, "grant", "ok");
    assert_event(events, "open", "ok");
    assert_event(events, "send", "ok");
    assert_event(events, "recv", "ok");
    assert_event(events, "revoke", "ok");
    assert_event(events, "send_after_revoke", "revoked");
    assert_event(events, "fault", "fault");
    assert_event(events, "peer_down", "peer_down");
    assert_event(events, "demo_done", "ok");
    assert!(
        events.iter().any(|event| {
            event.get("event").and_then(Value::as_str) == Some("recv")
                && event.get("detail").and_then(Value::as_str) == Some("mvp0c-packet")
        }),
        "missing recv of mvp0c-packet; events: {events:#?}"
    );
}

#[test]
fn packet_rx_demo_runs_end_to_end() {
    let run_dir = unique_dir("ck-mvp0c-demo");
    let output = run_root(&[
        "--demo",
        "packet-rx",
        "--run-dir",
        run_dir.to_str().unwrap(),
    ]);
    assert_success(&output, "ck-root --demo packet-rx");
    assert_packet_rx_trace(&read_events(&run_dir.join("trace.ndjson")));
    let _ = std::fs::remove_dir_all(&run_dir);
}

#[test]
fn fixture_files_run_end_to_end() {
    let run_dir = unique_dir("ck-mvp0c-fixture");
    let fixtures = fixture_dir();
    let manifest = fixtures.join("manifest.json");
    let policy = fixtures.join("policy.json");
    let output = run_root(&[
        "--manifest",
        manifest.to_str().unwrap(),
        "--policy",
        policy.to_str().unwrap(),
        "--run-dir",
        run_dir.to_str().unwrap(),
    ]);
    assert_success(&output, "ck-root fixture files");
    assert_packet_rx_trace(&read_events(&run_dir.join("trace.ndjson")));
    let _ = std::fs::remove_dir_all(&run_dir);
}

#[test]
fn invalid_policy_is_rejected_by_ck_root() {
    let root_dir = unique_dir("ck-mvp0c-invalid");
    let run_dir = root_dir.join("run");
    let policy_path = root_dir.join("policy.json");
    let fixtures = fixture_dir();
    let mut policy: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("policy.json")).unwrap())
            .unwrap();
    policy["revokes"][0]["after"] = Value::String("fault".into());
    std::fs::write(&policy_path, serde_json::to_vec_pretty(&policy).unwrap()).unwrap();

    let output = run_root(&[
        "--manifest",
        fixtures.join("manifest.json").to_str().unwrap(),
        "--policy",
        policy_path.to_str().unwrap(),
        "--run-dir",
        run_dir.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "ck-root should reject illegal after sequencing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not before 'revoke'"),
        "unexpected stderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&root_dir);
}

#[test]
fn help_prints_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_ck-root"))
        .arg("--help")
        .output()
        .unwrap();
    assert_success(&output, "ck-root --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--manifest"), "{stdout}");
    assert!(stdout.contains("packet-rx"), "{stdout}");
}

fn assert_event(events: &[Value], name: &str, status: &str) {
    assert!(
        events.iter().any(|event| {
            event.get("event").and_then(Value::as_str) == Some(name)
                && event.get("status").and_then(Value::as_str) == Some(status)
        }),
        "missing trace event {name} with status {status}; events: {events:#?}"
    );
}
