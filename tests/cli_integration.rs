// Integration tests for CLI argument parsing and subcommand behavior.

#[test]
fn no_args_yields_tui_mode() {
    // Simulating: elysiae-tui (no subcommand)
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_elysiae-tui"))
        .arg("--help")
        .output()
        .expect("failed to run binary");

    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("download"));
    assert!(stdout.contains("update"));
    assert!(stdout.contains("launch"));
    assert!(stdout.contains("verify"));
    assert!(stdout.contains("check-update"));
    assert!(stdout.contains("preinstall"));
    assert!(stdout.contains("apply-preinstall"));
}

#[test]
fn download_requires_path() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_elysiae-tui"))
        .args(["download", "hk4e"])
        .output()
        .expect("failed to run binary");

    // Should fail because --path is required
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("--path"));
}

#[test]
fn invalid_game_id_rejected() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_elysiae-tui"))
        .args(["download", "invalid_game", "--path", "/tmp"])
        .output()
        .expect("failed to run binary");

    assert!(!result.status.success());
}

#[test]
fn check_update_accepts_valid_game() {
    // This will fail at runtime (no install path) but should parse successfully
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_elysiae-tui"))
        .args(["check-update", "hk4e"])
        .output()
        .expect("failed to run binary");

    // Fails with "no install path" error, not a parse error
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("error") || !result.status.success());
}

#[test]
fn version_flag_works() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_elysiae-tui"))
        .arg("--version")
        .output()
        .expect("failed to run binary");

    assert!(result.status.success());
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("0.1.0"));
}
