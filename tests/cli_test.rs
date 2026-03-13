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

fn run_json_any_exit(args: &[&str]) -> Value {
    let output = bin().args(args).output().expect("binary should run");
    serde_json::from_slice(&output.stdout).expect("stdout should be valid json")
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
                .and(contains("shell"))
                .and(contains("exec"))
                .and(contains("clipboard"))
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
fn clipboard_help_lists_get_and_set() {
    bin()
        .args(["clipboard", "--help"])
        .assert()
        .success()
        .stdout(contains("get").and(contains("set")));
}

#[test]
fn json_connect_produces_valid_json() {
    // Connect spawns a daemon; result depends on whether one is already running.
    let value = run_json_any_exit(&["--json", "connect", "123", "--password", "pw"]);
    assert_eq!(value["command"], "connect");
    // Clean up: disconnect any daemon we may have spawned
    let _ = bin().args(["disconnect"]).output();
}

#[test]
fn json_disconnect_matches_contract() {
    // With no active session, disconnect returns session_error (exit 2)
    let value = run_json_any_exit(&["--json", "disconnect"]);

    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "disconnect");
    assert_eq!(value["error"]["code"], "session_error");
}

#[test]
fn json_shell_contract() {
    let value = run_json_any_exit(&["--json", "shell"]);
    assert_eq!(value["command"], "shell");
    if value["ok"] == true {
        assert_eq!(value["mode"], "interactive");
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_exec_contract() {
    let value = run_json_any_exit(&["--json", "exec", "--command", "whoami"]);
    assert_eq!(value["command"], "exec");
    if value["ok"] == true {
        assert_eq!(value["requested"], "whoami");
        assert_eq!(value["stdout"], "stub exec output");
        assert_eq!(value["stderr"], "");
        assert_eq!(value["exit_code"], 0);
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_clipboard_get_contract() {
    let value = run_json_any_exit(&["--json", "clipboard", "get"]);
    assert_eq!(value["command"], "clipboard");
    if value["ok"] == true {
        assert_eq!(value["action"], "get");
        assert_eq!(value["text"], "stub clipboard text");
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_clipboard_set_contract() {
    let value = run_json_any_exit(&["--json", "clipboard", "set", "--text", "hello"]);
    assert_eq!(value["command"], "clipboard");
    if value["ok"] == true {
        assert_eq!(value["action"], "set");
        assert_eq!(value["chars"], 5);
        assert_eq!(value["redacted"], true);
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_status_contract() {
    let value = run_json_any_exit(&["--json", "status"]);
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "status");
    assert!(value["connected"].is_boolean());
}

#[test]
fn json_capture_contract() {
    let value = run_json_any_exit(&["--json", "capture", "shot.png"]);
    assert_eq!(value["command"], "capture");
    if value["ok"] == true {
        assert_eq!(value["file"], "shot.png");
        assert_eq!(value["format"], "png");
        assert!(value["bytes"].as_u64().is_some());
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_type_contract() {
    let value = run_json_any_exit(&["--json", "type", "hello"]);
    assert_eq!(value["command"], "type");
    if value["ok"] == true {
        assert_eq!(value["chars"], 5);
        assert_eq!(value["redacted"], true);
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_key_contract() {
    let value = run_json_any_exit(&["--json", "key", "enter"]);
    assert_eq!(value["command"], "key");
    if value["ok"] == true {
        assert_eq!(value["key"], "enter");
        assert_eq!(value["modifiers"], serde_json::json!([]));
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_click_contract() {
    let value = run_json_any_exit(&["--json", "click", "500", "300"]);
    assert_eq!(value["command"], "click");
    if value["ok"] == true {
        assert_eq!(value["button"], "left");
        assert_eq!(value["x"], 500);
        assert_eq!(value["y"], 300);
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_move_contract() {
    let value = run_json_any_exit(&["--json", "move", "100", "200"]);
    assert_eq!(value["command"], "move");
    if value["ok"] == true {
        assert_eq!(value["x"], 100);
        assert_eq!(value["y"], 200);
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_drag_contract() {
    let value = run_json_any_exit(&["--json", "drag", "0", "0", "100", "100"]);
    assert_eq!(value["command"], "drag");
    if value["ok"] == true {
        assert_eq!(value["x1"], 0);
        assert_eq!(value["y1"], 0);
        assert_eq!(value["x2"], 100);
        assert_eq!(value["y2"], 100);
        assert_eq!(value["button"], "left");
    } else {
        assert!(value["error"]["code"].is_string());
    }
}

#[test]
fn json_do_matches_contract() {
    // Batch 'do' uses stubs — verifies output format
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
fn commands_without_daemon_exit_correctly() {
    // disconnect with no session returns exit 2 (session error per SPEC)
    bin().args(["disconnect"]).assert().code(2);
    // status always exit 0 (idempotent / always succeeds)
    bin().args(["status"]).assert().code(0);
    // batch 'do' uses stubs
    bin()
        .args(["do", "connect", "123", "type", "hello"])
        .assert()
        .code(0);
}

#[test]
fn do_verifies_output_format_for_all_commands() {
    // Batch 'do' uses stubs — verify output format for all command types
    let value = run_json(&[
        "--json",
        "do",
        "capture",
        "shot.png",
        "click",
        "500",
        "300",
        "move",
        "100",
        "200",
        "drag",
        "0",
        "0",
        "100",
        "100",
        "disconnect",
    ]);

    assert_eq!(value["ok"], true);
    let steps = value["steps"].as_array().expect("steps array");

    // capture format
    assert_eq!(steps[0]["command"], "capture");
    assert_eq!(steps[0]["file"], "shot.png");
    assert_eq!(steps[0]["format"], "png");
    assert!(steps[0]["bytes"].as_u64().is_some());

    // click format
    assert_eq!(steps[1]["command"], "click");
    assert_eq!(steps[1]["button"], "left");
    assert_eq!(steps[1]["x"], 500);
    assert_eq!(steps[1]["y"], 300);

    // move format
    assert_eq!(steps[2]["command"], "move");
    assert_eq!(steps[2]["x"], 100);
    assert_eq!(steps[2]["y"], 200);

    // drag format
    assert_eq!(steps[3]["command"], "drag");
    assert_eq!(steps[3]["x1"], 0);

    // disconnect format
    assert_eq!(steps[4]["command"], "disconnect");
}

#[test]
fn do_verifies_output_format_for_text_mode_pivot_commands() {
    let value = run_json(&[
        "--json",
        "do",
        "shell",
        "exec",
        "--command",
        "pwd",
        "clipboard",
        "get",
        "clipboard",
        "set",
        "--text",
        "hello",
    ]);

    assert_eq!(value["ok"], true);
    let steps = value["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 4);

    assert_eq!(steps[0]["command"], "shell");
    assert_eq!(steps[0]["mode"], "interactive");

    assert_eq!(steps[1]["command"], "exec");
    assert_eq!(steps[1]["requested"], "pwd");
    assert_eq!(steps[1]["stdout"], "stub exec output");
    assert_eq!(steps[1]["exit_code"], 0);

    assert_eq!(steps[2]["command"], "clipboard");
    assert_eq!(steps[2]["action"], "get");
    assert_eq!(steps[2]["text"], "stub clipboard text");

    assert_eq!(steps[3]["command"], "clipboard");
    assert_eq!(steps[3]["action"], "set");
    assert_eq!(steps[3]["chars"], 5);
    assert_eq!(steps[3]["redacted"], true);
}

#[test]
fn capture_region_valid_parses_to_json_region() {
    // Test region parsing through batch 'do' (which uses stubs)
    let value = run_json(&[
        "--json",
        "do",
        "capture",
        "shot.png",
        "--region",
        "100,200,300,400",
    ]);

    assert_eq!(value["ok"], true);
    let steps = value["steps"].as_array().expect("steps array");
    assert_eq!(steps[0]["command"], "capture");
    assert_eq!(
        steps[0]["region"],
        serde_json::json!({
            "x": 100,
            "y": 200,
            "w": 300,
            "h": 400
        })
    );
    assert_eq!(steps[0]["width"], 300);
    assert_eq!(steps[0]["height"], 400);
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
