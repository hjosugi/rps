//! Port of upstream's `test/xUnit/csharp/test_CommandLineParser.cs`.
//!
//! The upstream parser exposes every flag it parsed. rpsh's host has only the
//! handful it acts on, and refuses the rest by name (DESIGN.md §2), so these
//! tests pin the same cases to the smaller API: a switch that the host does
//! not implement is asserted to be `NotImplemented` instead of silently
//! accepted. The mapping is recorded in docs/09_xunit_port.md.

use std::path::PathBuf;

use rpsh_host::{CliError, Invocation, parse_args};

fn parse(args: &[&str]) -> Result<Invocation, CliError> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    parse_args(&args)
}

fn refused(args: &[&str]) -> bool {
    matches!(parse(args), Err(CliError::NotImplemented(_)))
}

fn file(path: &str) -> Result<Invocation, CliError> {
    Ok(Invocation::File(PathBuf::from(path)))
}

#[test]
fn test_defaults() {
    assert_eq!(parse(&[]), Ok(Invocation::Interactive));
}

#[test]
fn test_throws_on_reuse() {
    assert_eq!(parse(&["-Version"]), parse(&["-Version"]));
}

#[test]
fn test_args_with_null() {
    assert_eq!(parse(&[""]), file(""));
}

#[test]
fn test_default_parameter_is_file_name_not_exist() {
    assert_eq!(parse(&["noexistfilename"]), file("noexistfilename"));
}

#[test]
fn test_default_parameter_is_file_name_exist() {
    assert_eq!(
        parse(&["/tmp/does-not-need-to-exist.ps1"]),
        file("/tmp/does-not-need-to-exist.ps1")
    );
}

#[test]
fn test_parameter_is_file_name_dash() {
    for switch in ["-file", "-f"] {
        assert!(refused(&[switch, "-"]), "{switch}");
    }
}

#[test]
fn test_parameter_is_wrong_value() {
    assert_eq!(parse(&["-prof"]), file("-prof"));
}

#[test]
fn test_parameter_dash_or_slash() {
    for switch in ["-Version", "--Version", "/Version"] {
        assert_eq!(parse(&[switch]), Ok(Invocation::Version), "{switch}");
    }
}

#[test]
fn test_parameter_wrong_dash_and_slash() {
    for switch in ["/-Version", "-/Version"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_version() {
    for args in [&["-Version"][..], &["-V"][..], &["-Version", "abbra"][..]] {
        assert_eq!(parse(args), Ok(Invocation::Version));
    }
}

#[test]
fn test_parameter_help() {
    for switch in ["-Help", "-h", "-?"] {
        assert_eq!(parse(&[switch]), Ok(Invocation::Help), "{switch}");
    }
}

#[test]
fn test_parameter_login() {
    for switch in ["-Login", "-l"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_noexit() {
    for switch in ["-noexit", "-noe"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_noprofile() {
    for switch in ["-noprofile", "-nop"] {
        assert_eq!(parse(&[switch]), Ok(Invocation::Interactive), "{switch}");
    }
}

#[test]
fn test_parameter_nologo() {
    for switch in ["-nologo", "-nol"] {
        assert_eq!(parse(&[switch]), Ok(Invocation::Interactive), "{switch}");
    }
}

#[test]
fn test_parameter_nointeractive() {
    for switch in ["-noninteractive", "-noni"] {
        assert_eq!(parse(&[switch]), Ok(Invocation::Interactive), "{switch}");
    }
}

#[test]
fn test_parameter_socketservermode() {
    for switch in ["-socketservermode", "-so"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_v2socketservermode() {
    for args in [
        &[
            "-v2socketservermode",
            "-token",
            "natoheusatoehusnatoeu",
            "-utctimestamp",
            "2023-10-01T12:00:00Z",
        ][..],
        &[
            "-v2so",
            "-token",
            "asentuhasoneuthsaoe",
            "-utctimestamp",
            "2025-06-09T12:00:00Z",
        ][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_servermode() {
    for switch in ["-servermode", "-s"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_namedpipeservermode() {
    for switch in ["-namedpipeservermode", "-nam"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_sshservermode() {
    for switch in ["-sshservermode", "-sshs"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_interactive() {
    for switch in ["-interactive", "-i"] {
        assert!(refused(&[switch]), "{switch}");
    }
}

#[test]
fn test_parameter_configuration_name() {
    for args in [
        &["-configurationname"][..],
        &["-config"][..],
        &["-configurationname", "qwerty"][..],
        &["-config", "qwerty"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_configuration_file() {
    for args in [
        &["-configurationfile"][..],
        &["-configurationfile", "qwerty"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_custom_pipe_name() {
    for args in [
        &["-custompipename"][..],
        &["-cus"][..],
        &["-custompipename", "qwerty"][..],
        &["-cus", "qwerty"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_command_no_value() {
    for switch in ["-command", "-c"] {
        assert_eq!(parse(&[switch]), Err(CliError::MissingCommand), "{switch}");
    }
}

#[test]
fn test_parameter_command_with_value() {
    for switch in ["-command", "-c"] {
        assert_eq!(
            parse(&[switch, "qwerty"]),
            Ok(Invocation::Command("qwerty".to_owned())),
            "{switch}"
        );
    }
}

#[test]
fn test_parameter_command_with_dash() {
    for switch in ["-command", "-c"] {
        assert!(refused(&[switch, "-"]), "{switch}");
    }
}

#[test]
fn test_parameter_command_with_dash_and_tail() {
    for switch in ["-command", "-c"] {
        assert!(refused(&[switch, "-", "abbra"]), "{switch}");
    }
}

#[test]
fn test_parameter_windowstyle() {
    for args in [
        &["-windowstyle"][..],
        &["-w"][..],
        &["-windowstyle", "abbra"][..],
        &["-w", "abbra"][..],
        &["-windowstyle", "Maximized"][..],
        &["-w", "Maximized"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_outputformat() {
    for args in [
        &["-outputformat"][..],
        &["-o"][..],
        &["-of"][..],
        &["-outputformat", "abbra"][..],
        &["-o", "abbra"][..],
        &["-of", "abbra"][..],
        &["-outputformat", "XML"][..],
        &["-o", "XML"][..],
        &["-of", "XML"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_inputformat() {
    for args in [
        &["-inputformat"][..],
        &["-inp"][..],
        &["-if"][..],
        &["-inputformat", "abbra"][..],
        &["-inp", "abbra"][..],
        &["-if", "abbra"][..],
        &["-inputformat", "XML"][..],
        &["-inp", "XML"][..],
        &["-if", "XML"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_executionpolicy() {
    for args in [
        &["-executionpolicy"][..],
        &["-ex"][..],
        &["-ep"][..],
        &["-executionpolicy", "InvalidPolicy"][..],
        &["-ex", "InvalidPolicy"][..],
        &["-ep", "InvalidPolicy"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_encodedcommand() {
    for args in [
        &["-encodedcommand"][..],
        &["-e"][..],
        &["-ec"][..],
        &["-encodedcommand", "YQBiAGIAcgBhAA=="][..],
        &["-e", "YQBiAGIAcgBhAA=="][..],
        &["-ec", "YQBiAGIAcgBhAA=="][..],
        &["-encodedcommand", "-"][..],
        &["-e", "-"][..],
        &["-ec", "-"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_encodedarguments() {
    for args in [
        &["-encodedarguments"][..],
        &["-encodeda"][..],
        &["-ea"][..],
        &["-encodedarguments", "abbra"][..],
        &["-encodeda", "abbra"][..],
        &["-ea", "abbra"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_settingsfile() {
    for args in [
        &["-settingsfile"][..],
        &["-settings"][..],
        &["-settingsfile", "noexistfilename"][..],
        &["-settings", "noexistfilename"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_sta() {
    for args in [&["-sta"][..], &["-sta", "-mta"][..]] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_mta() {
    for args in [&["-mta"][..], &["-mta", "-sta"][..]] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_workingdirectory() {
    for args in [
        &["-workingdirectory"][..],
        &["-wo"][..],
        &["-wd"][..],
        &["-workingdirectory", "dirname"][..],
        &["-wo", "dirname"][..],
        &["-wd", "dirname"][..],
        &[
            "-workingdirectory",
            "dirname",
            "-removeworkingdirectorytrailingcharacter",
        ][..],
        &["-wo", "dirname", "-removeworkingdirectorytrailingcharacter"][..],
        &["-wd", "dirname", "-removeworkingdirectorytrailingcharacter"][..],
    ] {
        assert!(refused(args));
    }
}

#[test]
fn test_parameter_last_parameter_is_file_name() {
    assert_eq!(parse(&["-noprofile", "script.ps1"]), file("script.ps1"));
}
