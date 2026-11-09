//! One SoliDB server process the harness can start, kill with SIGKILL and
//! start again on the same data directory.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use crate::Res;

pub const PASSWORD: &str = "faults";

pub struct Server {
    pub bin: PathBuf,
    pub data: PathBuf,
    pub port: u16,
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
    child: Option<Child>,
}

impl Server {
    pub fn new(
        bin: PathBuf,
        data: PathBuf,
        port: u16,
        env: Vec<(String, String)>,
        log: PathBuf,
    ) -> Self {
        Server {
            bin,
            data,
            port,
            env,
            log,
            child: None,
        }
    }

    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Start the server and wait until it accepts a login. Its output is
    /// appended to `log`, so every start of a run is in one file.
    pub async fn start(&mut self, http: &reqwest::Client) -> Res<Duration> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)?;
        let mut cmd = Command::new(&self.bin);
        cmd.arg("--port")
            .arg(self.port.to_string())
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--data-dir")
            .arg(&self.data)
            .env("SOLIDB_ADMIN_PASSWORD", PASSWORD)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(true);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let started = Instant::now();
        self.child = Some(cmd.spawn()?);
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait()? {
                return Err(format!(
                    "server exited during startup ({status}); last lines:\n{}",
                    tail(&self.log, 30)
                )
                .into());
            }
            if login(http, &self.url()).await.is_ok() {
                return Ok(started.elapsed());
            }
            if started.elapsed() > Duration::from_secs(120) {
                return Err(format!(
                    "server not ready after 120 s; last lines:\n{}",
                    tail(&self.log, 30)
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// SIGKILL: no shutdown path runs, nothing is flushed by the server.
    pub async fn kill(&mut self) -> Res<()> {
        if let Some(mut child) = self.child.take() {
            child.start_kill()?;
            child.wait().await?;
        }
        Ok(())
    }
}

/// A JWT for the admin user.
pub async fn login(http: &reqwest::Client, url: &str) -> Res<String> {
    let r = http
        .post(format!("{url}/auth/login"))
        .json(
            &serde_json::json!({"database": "_system", "username": "admin", "password": PASSWORD}),
        )
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(format!("login: HTTP {}", r.status()).into());
    }
    let v: serde_json::Value = r.json().await?;
    v["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "login: no token".into())
}

pub fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}
