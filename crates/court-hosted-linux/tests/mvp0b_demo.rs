#![cfg(unix)]

use serde_json::Value;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn packet_rx_demo_runs_end_to_end() {
    let run_dir = std::env::temp_dir().join(format!(
        "ck-mvp0b-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&run_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ck-root"))
        .arg("--demo")
        .arg("packet-rx")
        .arg("--run-dir")
        .arg(&run_dir)
        .env("CK_APP_BIN", env!("CARGO_BIN_EXE_ck-app"))
        .env("CK_NET_BIN", env!("CARGO_BIN_EXE_ck-net"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "ck-root failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let trace_path = run_dir.join("trace.ndjson");
    let trace = std::fs::read_to_string(&trace_path).unwrap();
    let events = trace
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_event(&events, "lookup", "ok");
    assert_event(&events, "open_denied", "no_right");
    assert_event(&events, "grant", "ok");
    assert_event(&events, "open", "ok");
    assert_event(&events, "send", "ok");
    assert_event(&events, "recv", "ok");
    assert_event(&events, "revoke", "ok");
    assert_event(&events, "send_after_revoke", "revoked");
    assert_event(&events, "fault", "fault");
    assert_event(&events, "peer_down", "peer_down");
    assert_event(&events, "demo_done", "ok");

    let _ = std::fs::remove_dir_all(&run_dir);
}

#[test]
fn manifest_policy_demo_runs_end_to_end() {
    let root_dir = std::env::temp_dir().join(format!(
        "ck-mvp0c-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let run_dir = root_dir.join("run");
    std::fs::create_dir_all(&root_dir).unwrap();
    let manifest_path = root_dir.join("manifest.json");
    let policy_path = root_dir.join("policy.json");
    std::fs::write(
        &manifest_path,
        r#"{
  "courts": [
    { "name": "app", "role": "app" },
    { "name": "net0", "role": "net" }
  ],
  "corridors": [
    {
      "path": "/court/net0/packet/rx",
      "transport": "shared_ring",
      "from": "app",
      "to": "net0",
      "capacity": 64,
      "slot_size": 2048
    }
  ],
  "demo": {
    "packet_payload": "mvp0c-packet"
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        &policy_path,
        r#"{
  "grants": [
    { "court": "app", "path": "/court/net0/packet/rx", "rights": ["send", "observe"] },
    { "court": "net0", "path": "/court/net0/packet/rx", "rights": ["recv", "observe"] }
  ],
  "revokes": [
    { "court": "app", "path": "/court/net0/packet/rx", "after": "recv" }
  ],
  "faults": [
    { "court": "net0", "after": "revoke", "reason": "mvp0c simulated net court crash" }
  ],
  "peer_down": [
    { "court": "app", "path": "/court/net0/packet/rx", "after": "fault" }
  ]
}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ck-root"))
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--policy")
        .arg(&policy_path)
        .arg("--run-dir")
        .arg(&run_dir)
        .env("CK_APP_BIN", env!("CARGO_BIN_EXE_ck-app"))
        .env("CK_NET_BIN", env!("CARGO_BIN_EXE_ck-net"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "ck-root manifest demo failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let trace = std::fs::read_to_string(run_dir.join("trace.ndjson")).unwrap();
    let events = trace
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    assert_event(&events, "lookup", "ok");
    assert_event(&events, "open_denied", "no_right");
    assert_event(&events, "grant", "ok");
    assert_event(&events, "open", "ok");
    assert_event(&events, "send", "ok");
    assert_event(&events, "recv", "ok");
    assert_event(&events, "revoke", "ok");
    assert_event(&events, "send_after_revoke", "revoked");
    assert_event(&events, "fault", "fault");
    assert_event(&events, "peer_down", "peer_down");
    assert_event(&events, "demo_done", "ok");
    assert!(events.iter().any(|event| {
        event.get("event").and_then(Value::as_str) == Some("recv")
            && event.get("detail").and_then(Value::as_str) == Some("mvp0c-packet")
    }));

    let _ = std::fs::remove_dir_all(&root_dir);
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
