use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn redirected_input_follows_the_current_script_blocks_variable_references() {
    for (script, expected) in [
        (r#"process { "$_/$input" }"#, "a/a\nb/b\nc/c\n/\n"),
        (
            r#"process { $unused = { $input }; "single=$_" }"#,
            "single=\n",
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rpsh"))
            .args(["-NoProfile", "-Command", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"a\rb\r\nc\n\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{script}: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8(output.stdout)
                .unwrap()
                .replace("\r\n", "\n"),
            expected
        );
    }
}
