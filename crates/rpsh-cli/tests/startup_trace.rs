use std::process::{Command, Output};

fn run(args: &[&str], trace: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rpsh"));
    command.env_remove("RPSH_TRACE_INIT").args(args);
    if let Some(value) = trace {
        command.env("RPSH_TRACE_INIT", value);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn cold_entry_points_touch_only_the_resources_they_use() {
    for (args, expected) in [
        (&["-Version"][..], &["host.arguments"][..]),
        (
            &["-NoProfile", "-Command", "exit"][..],
            &["host.arguments", "engine.session", "syntax.parse"][..],
        ),
        (
            &["-NoProfile", "-Command", "1 + 1"][..],
            &[
                "host.arguments",
                "engine.session",
                "syntax.parse",
                "host.writer",
            ][..],
        ),
        (
            &["-NoProfile", "-Command", "42 | Out-Null"][..],
            &[
                "host.arguments",
                "engine.session",
                "syntax.parse",
                "aliases.lookup",
                "cmdlets.lookup",
            ][..],
        ),
    ] {
        let traced = run(args, Some("1"));
        let stderr = String::from_utf8(traced.stderr).unwrap();
        let stages: Vec<_> = stderr
            .lines()
            .map(|line| line.strip_prefix("rpsh init\t").unwrap())
            .collect();
        assert_eq!(stages, expected, "{args:?}");
        let normal = run(args, None);
        assert_eq!(normal.stdout, traced.stdout);
        assert!(normal.stderr.is_empty());
    }
}

#[test]
fn first_use_is_reported_once_and_disabled_tracing_is_silent() {
    let args = [
        "-NoProfile",
        "-Command",
        "function f { [math]::Abs(-1) }; f; f; $x=2; $x; Get-PSProvider | Out-Null; Get-PSProvider | Out-Null; $Error.Count",
    ];
    let traced = run(&args, Some("1"));
    let stderr = String::from_utf8(traced.stderr).unwrap();
    for stage in [
        "functions.table",
        "types.lookup",
        "variables.table",
        "providers.lookup",
        "module.load\tMicrosoft.PowerShell.Management",
        "errors.history",
        "host.writer",
    ] {
        assert_eq!(
            stderr
                .lines()
                .filter(|line| line.strip_prefix("rpsh init\t") == Some(stage))
                .count(),
            1,
            "{stderr}"
        );
    }
    for flag in ["0", ""] {
        let normal = run(&args, Some(flag));
        assert_eq!(normal.stdout, traced.stdout);
        assert!(normal.stderr.is_empty());
    }
}
