use assert_cmd::Command;

fn bin() -> Command {
    Command::cargo_bin("rustdesk-cli").expect("binary exists")
}

#[test]
fn no_session_errors_go_to_stderr_only() {
    let output = bin()
        .args(["type", "hello"])
        .output()
        .expect("binary should run");

    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty(), "stdout should be empty on error");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No active session"),
        "stderr should contain the session error"
    );
}

#[test]
fn disconnect_without_session_exits_two_and_uses_stderr() {
    let output = bin()
        .args(["disconnect"])
        .output()
        .expect("binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty(), "stdout should be empty on error");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No active session"),
        "stderr should contain the disconnect error"
    );
}

#[test]
fn clap_argument_errors_exit_three() {
    let output = bin()
        .args(["capture", "shot.png", "--region", "100,200,300"])
        .output()
        .expect("binary should run");

    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("region must be in x,y,w,h format"),
        "stderr should contain the input validation error"
    );
}
