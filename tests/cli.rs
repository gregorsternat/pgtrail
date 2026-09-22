use std::process::{Command, Stdio};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgtrail"));
    command.stdin(Stdio::null());
    command
}

#[test]
fn help_works_without_a_terminal() -> std::io::Result<()> {
    let output = command().arg("--help").output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: pgtrail"));
    assert!(stdout.contains("does not connect to PostgreSQL"));
    Ok(())
}

#[test]
fn version_matches_package_metadata() -> std::io::Result<()> {
    let output = command().arg("--version").output()?;
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        concat!("pgtrail ", env!("CARGO_PKG_VERSION"))
    );
    Ok(())
}

#[test]
fn unknown_arguments_fail_with_usage() -> std::io::Result<()> {
    let output = command().arg("--does-not-exist").output()?;
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
    Ok(())
}

#[test]
fn non_interactive_start_fails_without_terminal_escape_sequences() -> std::io::Result<()> {
    let output = command().output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an interactive terminal"));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.contains(&0x1b));
    Ok(())
}
