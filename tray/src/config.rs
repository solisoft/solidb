//! Settings of the icon: a `key = value` file the user may edit.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The server to start.
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    /// Where the server's output goes: it has no console of its own.
    pub log: PathBuf,
    /// Arguments added after `--port` and `--data-dir`.
    pub args: Vec<String>,
    /// Opened by "Open admin console"; the entry is absent when empty.
    pub console_url: String,
    /// Start the server when the icon starts.
    pub start_server: bool,
}

impl Config {
    /// What is used for a key the file does not set. `home` is the folder
    /// the icon keeps its files in, `beside` the folder of its executable.
    pub fn defaults(home: &Path, beside: &Path) -> Config {
        Config {
            exe: beside.join("solidb.exe"),
            data_dir: home.join("data"),
            port: 6745,
            log: home.join("solidb.log"),
            args: Vec::new(),
            console_url: String::new(),
            start_server: true,
        }
    }

    /// Reads the file's text over `self`. A line that cannot be read is
    /// reported and left out: the icon must still come up, to say so.
    pub fn read(mut self, text: &str) -> (Config, Vec<String>) {
        let mut refused = Vec::new();
        for (at, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                refused.push(format!("line {}: no '='", at + 1));
                continue;
            };
            let value = unquoted(value.trim());
            match key.trim() {
                "exe" => self.exe = PathBuf::from(value),
                "data_dir" => self.data_dir = PathBuf::from(value),
                "log" => self.log = PathBuf::from(value),
                "console_url" => self.console_url = value.to_string(),
                "args" => self.args = words(value),
                "port" => match value.parse::<u16>() {
                    Ok(port) if port != 0 => self.port = port,
                    _ => refused.push(format!("line {}: port '{}'", at + 1, value)),
                },
                "start_server" => match value {
                    "true" | "yes" | "1" => self.start_server = true,
                    "false" | "no" | "0" => self.start_server = false,
                    _ => refused.push(format!("line {}: start_server '{}'", at + 1, value)),
                },
                other => refused.push(format!("line {}: unknown key '{}'", at + 1, other)),
            }
        }
        (self, refused)
    }

    /// Replaces `%NAME%` in the paths by the environment's value, as
    /// Windows does in its own settings. A name that is not set is left as
    /// written, so that the error names it.
    pub fn expand(mut self, env: impl Fn(&str) -> Option<String>) -> Config {
        for path in [&mut self.exe, &mut self.data_dir, &mut self.log] {
            *path = PathBuf::from(expanded(&path.to_string_lossy(), &env));
        }
        self
    }

    /// The icon's file as written the first time, every key shown with its
    /// default.
    pub fn template(&self) -> String {
        let mut text = self.shared(
            "# Settings of the SoliDB icon. Quit the icon and start it again to apply a change.",
        );
        text.push_str(
            "\r\n\
             # Address opened by \"Open admin console\", when an admin application runs.\r\n\
             console_url =\r\n\
             \r\n\
             # Start the server when the icon starts.\r\n\
             start_server = true\r\n",
        );
        text
    }

    /// The service's file as written when it is installed.
    pub fn service_template(&self) -> String {
        self.shared("# Settings of the SoliDB service. Restart the service to apply a change.")
    }

    fn shared(&self, header: &str) -> String {
        format!(
            "{header}\r\n\
             # A path may name a variable of the environment: %ProgramData%\\SoliDB\\data\r\n\
             \r\n\
             # The server to start.\r\n\
             exe = {}\r\n\
             \r\n\
             data_dir = {}\r\n\
             port = {}\r\n\
             \r\n\
             # The server's output, where a new instance prints its admin password.\r\n\
             log = {}\r\n\
             \r\n\
             # Arguments added to the server's command line, separated by spaces\r\n\
             # (quote one that holds a space). Example: args = --host 0.0.0.0\r\n\
             args =\r\n",
            self.exe.display(),
            self.data_dir.display(),
            self.port,
            self.log.display(),
        )
    }

    /// The server's command line. Windows hands a process one string, which
    /// the server splits again: each argument is quoted the way it reads it.
    pub fn command_line(&self) -> String {
        let mut line = quoted(&self.exe.to_string_lossy());
        for arg in ["--port", &self.port.to_string(), "--data-dir"] {
            line.push(' ');
            line.push_str(arg);
        }
        line.push(' ');
        line.push_str(&quoted(&self.data_dir.to_string_lossy()));
        for arg in &self.args {
            line.push(' ');
            line.push_str(&quoted(arg));
        }
        line
    }
}

fn expanded(text: &str, env: &impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after
            .find('%')
            .map(|end| (&after[..end], &after[end + 1..]))
        {
            Some((name, tail)) => {
                match env(name).filter(|_| !name.is_empty()) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = tail;
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn unquoted(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
}

/// Splits on spaces, a double-quoted run being one word.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(word);
    }
    out
}

/// Quotes an argument as the C runtime's parser reads it back: backslashes
/// are literal except before a quote, where each is doubled.
fn quoted(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                slashes = 0;
                out.push('"');
                continue;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', slashes));
                slashes = 0;
            }
        }
        if c != '\\' {
            out.push(c);
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Config {
        Config::defaults(
            Path::new("C:\\Users\\me\\SoliDB"),
            Path::new("C:\\Program Files\\SoliDB"),
        )
    }

    #[test]
    fn a_file_sets_what_it_names_and_leaves_the_rest() {
        let (cfg, refused) = defaults().read(
            "# comment\n port = 7000 \nargs = --host 127.0.0.1 \"--x=a b\"\nstart_server=no\n",
        );
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(cfg.port, 7000);
        assert_eq!(cfg.args, ["--host", "127.0.0.1", "--x=a b"]);
        assert!(!cfg.start_server);
        assert_eq!(cfg.exe, defaults().exe);
    }

    #[test]
    fn a_line_that_cannot_be_read_is_reported_and_skipped() {
        let (cfg, refused) = defaults().read("port = 0\nnonsense\ncolour = blue\nport = 70000\n");
        assert_eq!(cfg.port, 6745);
        assert_eq!(refused.len(), 4, "{refused:?}");
    }

    #[test]
    fn the_template_reads_back_as_the_defaults() {
        let (cfg, refused) = defaults().read(&defaults().template());
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(cfg, defaults());
    }

    #[test]
    fn the_service_template_reads_back_as_the_defaults() {
        let (cfg, refused) = defaults().read(&defaults().service_template());
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(cfg, defaults());
    }

    #[test]
    fn a_path_takes_the_value_of_a_variable_it_names() {
        let env = |name: &str| {
            (name == "LOCALAPPDATA").then(|| "C:\\Users\\me\\AppData\\Local".to_string())
        };
        let (cfg, _) = defaults()
            .read("data_dir = %LOCALAPPDATA%\\SoliDB\\data\nlog = %NOPE%\\x.log\nexe = 100%\\a\n");
        let cfg = cfg.expand(env);
        assert_eq!(
            cfg.data_dir,
            PathBuf::from("C:\\Users\\me\\AppData\\Local\\SoliDB\\data")
        );
        assert_eq!(cfg.log, PathBuf::from("%NOPE%\\x.log"));
        assert_eq!(cfg.exe, PathBuf::from("100%\\a"));
    }

    #[test]
    fn the_command_line_quotes_what_holds_a_space() {
        let mut cfg = defaults();
        // Set by hand: `join` uses the separator of the system the test runs on.
        cfg.exe = PathBuf::from("C:\\Program Files\\SoliDB\\solidb.exe");
        cfg.data_dir = PathBuf::from("C:\\My Data\\");
        cfg.args = vec!["--host".into(), "127.0.0.1".into(), "say \"hi\"".into()];
        assert_eq!(
            cfg.command_line(),
            "\"C:\\Program Files\\SoliDB\\solidb.exe\" --port 6745 --data-dir \"C:\\My Data\\\\\" \
             --host 127.0.0.1 \"say \\\"hi\\\"\""
        );
    }
}
