use std::process::{Command, Stdio};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pgtrail"));
    command.stdin(Stdio::null());
    command.env_remove("PGTRAIL_DATABASE_URL");
    command.env_remove("PGTRAIL_STORE");
    command
}

#[test]
fn help_works_without_a_terminal() -> std::io::Result<()> {
    let output = command().arg("--help").output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: pgtrail"));
    assert!(stdout.contains("--demo"));
    assert!(stdout.contains("offline comparisons"));
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

#[test]
fn demo_capture_restart_export_compare_and_delete() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let store = directory.path().join("history.sqlite3");
    for label in ["Before", "After"] {
        let output = command()
            .arg("--store")
            .arg(&store)
            .args(["--demo", "capture", "--label", label])
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Saved capture #"));
    }
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["snapshots", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let list: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(list.as_array().map(Vec::len), Some(2));

    // Offline commands must work even if the user's live URL is invalid.
    let output = command()
        .env("PGTRAIL_DATABASE_URL", "invalid secret URL")
        .arg("--store")
        .arg(&store)
        .args(["show", "1", "--format", "json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(snapshot["schema_version"], 1);
    assert_eq!(snapshot["source"]["database"], "demo_shop");
    for session in snapshot["activity"]["data"]
        .as_array()
        .ok_or("missing activity")?
    {
        assert!(session["query"].is_null());
    }

    let export = directory.path().join("incident.md");
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["compare", "1", "2", "--output"])
        .arg(&export)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let markdown = std::fs::read_to_string(&export)?;
    assert!(markdown.contains("demo\\_shop"));
    assert!(markdown.contains("Statement") || markdown.contains("statement"));
    // Existing exports are never silently replaced.
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["show", "1", "--output"])
        .arg(&export)
        .output()?;
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(&export)?, markdown);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&export)?.permissions().mode() & 0o777,
            0o600
        );
    }
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["delete", "1"])
        .output()?;
    assert!(output.status.success());
    let output = command()
        .arg("--store")
        .arg(&store)
        .args(["show", "1"])
        .output()?;
    assert!(!output.status.success());
    Ok(())
}

#[test]
fn sql_text_requires_explicit_opt_in() -> Result<(), Box<dyn std::error::Error>> {
    let output = command()
        .args([
            "--demo",
            "--include-query-text",
            "check",
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert!(snapshot["activity"]["data"][0]["query"].as_str().is_some());
    Ok(())
}

#[test]
fn connection_errors_never_echo_credentials() -> std::io::Result<()> {
    for url in [
        "postgres://user:do-not-print-this@[invalid",
        "definitely-not-a-url-do-not-print-this",
    ] {
        let output = command()
            .env("PGTRAIL_DATABASE_URL", url)
            .arg("check")
            .output()?;
        assert!(!output.status.success());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!combined.contains("do-not-print-this"));
        assert!(!combined.contains(url));
    }
    Ok(())
}

#[test]
fn argument_bounds_and_missing_configuration_are_clear() -> std::io::Result<()> {
    for args in [["--refresh", "0"], ["--timeout", "121"], ["show", "0"]] {
        let output = command().args(args).output()?;
        assert_eq!(output.status.code(), Some(2));
    }
    let output = command().arg("check").output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("PGTRAIL_DATABASE_URL"));
    Ok(())
}
