use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use serde_json::Value;

fn bin() -> Command {
    Command::cargo_bin("rustdesk-cli").expect("binary exists")
}

fn run_json(args: &[&str]) -> Value {
    let output = bin().args(args).assert().success().get_output().stdout.clone();
    serde_json::from_slice(&output).expect("stdout should be valid json")
}

#[test]
fn help_lists_all_subcommands() {
    bin()
        .arg("--help")
        .assert()
        .success()
        .stdout(
            contains("connect")
                .and(contains("disconnect"))
                .and(contains("status"))
                .and(contains("capture"))
                .and(contains("type"))
                .and(contains("key"))
                .and(contains("click"))
                .and(contains("move"))
                .and(contains("drag"))
                .and(contains("do")),
        );
}

#[test]
fn json_connect_matches_contract() {
    let value = run_json(&["--json", "connect", "123", "--password", "pw"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "connect");
    assert_eq!(value["id"], "123");
    assert_eq!(value["connected"], true);
    assert_eq!(value["width"], 1920);
    assert_eq!(value["height"], 1080);
}

#[test]
fn json_disconnect_matches_contract() {
    let value = run_json(&["--json", "disconnect"]);

    assert_eq!(value, serde_json::json!({
        "ok": true,
        "command": "disconnect",
        "was_connected": false
    }));
}

#[test]
fn json_status_matches_contract() {
    let value = run_json(&["--json", "status"]);

    assert_eq!(value, serde_json::json!({
        "ok": true,
        "command": "status",
        "connected": false
    }));
}

#[test]
fn json_capture_matches_contract() {
    let value = run_json(&["--json", "capture", "shot.png"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "capture");
    assert_eq!(value["file"], "shot.png");
    assert_eq!(value["format"], "png");
    assert_eq!(value["width"], 1920);
    assert_eq!(value["height"], 1080);
    assert!(value["bytes"].as_u64().is_some());
}

#[test]
fn json_type_matches_contract() {
    let value = run_json(&["--json", "type", "hello"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "type");
    assert_eq!(value["chars"], 5);
    assert_eq!(value["redacted"], true);
}

#[test]
fn json_key_matches_contract() {
    let value = run_json(&["--json", "key", "enter"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "key");
    assert_eq!(value["key"], "enter");
    assert_eq!(value["modifiers"], serde_json::json!([]));
}

#[test]
fn json_click_matches_contract() {
    let value = run_json(&["--json", "click", "500", "300"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "click");
    assert_eq!(value["button"], "left");
    assert_eq!(value["x"], 500);
    assert_eq!(value["y"], 300);
}

#[test]
fn json_move_matches_contract() {
    let value = run_json(&["--json", "move", "100", "200"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "move");
    assert_eq!(value["x"], 100);
    assert_eq!(value["y"], 200);
}

#[test]
fn json_drag_matches_contract() {
    let value = run_json(&["--json", "drag", "0", "0", "100", "100"]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "drag");
    assert_eq!(value["x1"], 0);
    assert_eq!(value["y1"], 0);
    assert_eq!(value["x2"], 100);
    assert_eq!(value["y2"], 100);
    assert_eq!(value["button"], "left");
}

#[test]
fn json_do_matches_contract() {
    let value = run_json(&[
        "--json",
        "do",
        "connect",
        "123",
        "--password",
        "pw",
        "click",
        "500",
        "300",
        "type",
        "hello",
        "key",
        "enter",
        "capture",
        "shot.png",
    ]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "do");

    let steps = value["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 5);
    assert_eq!(steps[0]["index"], 1);
    assert_eq!(steps[0]["command"], "connect");
    assert_eq!(steps[1]["command"], "click");
    assert_eq!(steps[2]["command"], "type");
    assert_eq!(steps[3]["command"], "key");
    assert_eq!(steps[4]["command"], "capture");
}

#[test]
fn stub_commands_exit_zero() {
    let cases = [
        vec!["connect", "123"],
        vec!["disconnect"],
        vec!["status"],
        vec!["capture", "shot.png"],
        vec!["type", "hello"],
        vec!["key", "enter"],
        vec!["click", "500", "300"],
        vec!["move", "100", "200"],
        vec!["drag", "0", "0", "100", "100"],
        vec!["do", "connect", "123", "type", "hello"],
    ];

    for case in cases {
        bin().args(&case).assert().code(0);
    }
}

#[test]
fn capture_region_valid_parses_to_json_region() {
    let value = run_json(&[
        "--json",
        "capture",
        "shot.png",
        "--region",
        "100,200,300,400",
    ]);

    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "capture");
    assert_eq!(
        value["region"],
        serde_json::json!({
            "x": 100,
            "y": 200,
            "w": 300,
            "h": 400
        })
    );
    assert_eq!(value["width"], 300);
    assert_eq!(value["height"], 400);
}

#[test]
fn capture_region_invalid_shape_fails() {
    bin()
        .args(["capture", "shot.png", "--region", "100,200,300"])
        .assert()
        .failure()
        .stderr(contains("region must be in x,y,w,h format"));
}

#[test]
fn capture_region_invalid_dimensions_fail() {
    bin()
        .args(["capture", "shot.png", "--region", "100,200,0,400"])
        .assert()
        .failure()
        .stderr(contains("region width and height must be positive"));
}
