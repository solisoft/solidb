//! Password sourcing shared by `solidb-dump`, `solidb-restore` and
//! `solidb-fuse`, included with `#[path]` (it is not a binary target).
//!
//! Audit L5: the password used to be accepted only as `-p/--password`, which
//! puts it in `ps`, `/proc/*/cmdline` and shell history. The flag still works
//! for compatibility, with a warning.

use std::io::IsTerminal;

/// Environment variable read when neither `-p` nor `--password-file` is given.
pub const PASSWORD_ENV: &str = "SOLIDB_PASSWORD";

/// Resolve a password, in order: the command-line flag (warned about), the
/// `--password-file`, `SOLIDB_PASSWORD`, then an interactive prompt when
/// `prompt_for` names a user and a terminal is attached.
///
/// Returns `Ok(None)` when no source applies.
pub fn resolve_password(
    cli: Option<&str>,
    file: Option<&str>,
    prompt_for: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(pw) = cli {
        eprintln!(
            "Warning: a password given on the command line is visible to other \
             users in `ps` and is kept in shell history. Prefer {} or \
             --password-file.",
            PASSWORD_ENV
        );
        return Ok(Some(pw.to_string()));
    }

    if let Some(path) = file {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read password file '{}': {}", path, e))?;
        return Ok(Some(strip_line_ending(&contents).to_string()));
    }

    if let Ok(pw) = std::env::var(PASSWORD_ENV) {
        if !pw.is_empty() {
            return Ok(Some(pw));
        }
    }

    if let Some(user) = prompt_for {
        // rpassword reads from the controlling terminal, not stdin, so this
        // also works when stdin carries a dump (`cat dump | solidb-restore`).
        if std::io::stdin().is_terminal() || std::io::stderr().is_terminal() {
            let pw = rpassword::prompt_password(format!("Password for {}: ", user))
                .map_err(|e| format!("Cannot read password from terminal: {}", e))?;
            return Ok(Some(pw));
        }
    }

    Ok(None)
}

/// Drop one trailing newline (`\n` or `\r\n`), as left by `echo` or an
/// editor. Other whitespace is kept: it may be part of the password.
fn strip_line_ending(s: &str) -> &str {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

#[cfg(test)]
mod password_tests {
    use super::*;

    #[test]
    fn strips_one_line_ending_only() {
        assert_eq!(strip_line_ending("secret\n"), "secret");
        assert_eq!(strip_line_ending("secret\r\n"), "secret");
        assert_eq!(strip_line_ending("secret"), "secret");
        assert_eq!(strip_line_ending(" sec ret \n\n"), " sec ret \n");
    }

    #[test]
    fn cli_wins_then_file() {
        assert_eq!(
            resolve_password(Some("a"), None, None).unwrap(),
            Some("a".to_string())
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pw");
        std::fs::write(&path, "from-file\n").unwrap();
        assert_eq!(
            resolve_password(None, Some(path.to_str().unwrap()), None).unwrap(),
            Some("from-file".to_string())
        );
        assert!(resolve_password(None, Some("/nonexistent/solidb-pw"), None).is_err());
    }
}
