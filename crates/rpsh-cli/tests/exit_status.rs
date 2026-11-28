#![cfg(unix)]

use std::process::Command;

#[test]
fn command_mode_returns_pipeline_status_unless_exit_is_explicit() {
    for (script, expected) in [
        ("sh -c 'exit 7'", 1),
        ("sh -c 'exit 7' | Write-Output", 1),
        ("sh -c 'exit 7'; Write-Output ok", 0),
        ("sh -c 'exit 7'; return", 1),
        ("sh -c 'exit 7'; exit 0", 0),
        ("sh -c 'exit 7'; exit $LASTEXITCODE", 7),
        ("$false", 0),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rpsh"))
            .args(["-NoProfile", "-Command", script])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "{script}");
    }
}
