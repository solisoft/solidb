//! `solidb user`: manage users and their roles from the command line.
//!
//! The PostgreSQL equivalents, for orientation: `create` is `createuser`,
//! `delete` is `dropuser`, `passwd` is `ALTER ROLE … PASSWORD`, `grant` and
//! `revoke` are `GRANT` / `REVOKE`. Everything goes through the server's
//! `/_api/auth/users…` endpoints, so the server checks that the caller is an
//! admin and the change is replicated like any other.
//!
//! Passwords never go on the command line. The admin's comes from
//! `SOLIDB_PASSWORD`, `--password-file` or a prompt; the one being set comes
//! from a prompt (typed twice) or, for scripts, `--password-stdin`.
//!
//! Exit codes: 0 done, 1 any other error, 2 usage, 3 not found, 4 already
//! exists, 5 authentication or permission refused.

use clap::{Args, Subcommand};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::{Method, Url};
use serde::Deserialize;
use std::io::{BufRead, IsTerminal};

#[path = "../bin/shared/password.rs"]
mod password;

/// Environment variable naming the server, read when `--server` is absent.
pub const SERVER_ENV: &str = "SOLIDB_URL";

#[derive(Args, Debug)]
pub struct UserArgs {
    #[command(flatten)]
    pub connection: ConnectionArgs,

    #[command(subcommand)]
    pub command: UserCommand,
}

/// Who to connect to, and as whom. The principal must be an admin.
#[derive(Args, Debug, Clone)]
pub struct ConnectionArgs {
    /// Server URL
    #[arg(short, long, global = true, env = SERVER_ENV, default_value = "http://localhost:6745")]
    pub server: String,

    /// Admin user to log in as. Its password comes from SOLIDB_PASSWORD,
    /// --password-file, or a prompt.
    // Not named `username`: a global argument shares its id with the
    // subcommands' USERNAME positional, which would then overwrite it.
    #[arg(short = 'u', long = "username", global = true, default_value = "admin")]
    pub login_user: String,

    /// Read the admin password from this file (one trailing newline is ignored)
    #[arg(long, global = true)]
    pub password_file: Option<String>,

    /// API key (sk_…) or JWT to use instead of logging in
    #[arg(short = 'k', long, global = true, env = "SOLIDB_API_KEY")]
    pub api_key: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum UserCommand {
    /// List users and their roles
    #[command(visible_alias = "ls")]
    List {
        /// Print the server's JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Create a user; its password is prompted for (typed twice)
    Create {
        username: String,
        /// Role to assign: admin, editor, viewer, or a custom role
        #[arg(long)]
        role: Option<String>,
        /// Limit --role to this database (default: every database)
        #[arg(long, requires = "role")]
        database: Option<String>,
        /// Read the new user's password from the first line of stdin
        #[arg(long)]
        password_stdin: bool,
    },
    /// Delete a user and every role assigned to it
    #[command(visible_alias = "rm")]
    Delete { username: String },
    /// Set a password. Without USERNAME, changes your own; with one, an admin
    /// resets that user's password without needing the old one.
    Passwd {
        username: Option<String>,
        /// Read the new password from the first line of stdin
        #[arg(long)]
        password_stdin: bool,
    },
    /// Show the roles assigned to a user
    Roles {
        username: String,
        /// Print the server's JSON instead of a table
        #[arg(long)]
        json: bool,
    },
    /// Assign a role to a user
    Grant {
        username: String,
        role: String,
        /// Limit the role to this database (default: every database)
        #[arg(long)]
        database: Option<String>,
    },
    /// Remove a role from a user
    Revoke {
        username: String,
        role: String,
        /// The database the assignment is limited to, if it is
        #[arg(long)]
        database: Option<String>,
    },
}

/// A refusal from the server: its status and the message it gave.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (HTTP {})", self.message, self.status)
    }
}

impl std::error::Error for ApiError {}

/// The exit code for an error, so a script can tell "already exists" or
/// "not found" from a real failure (see the module docs).
pub fn exit_code(error: &anyhow::Error) -> i32 {
    match error.downcast_ref::<ApiError>().map(|e| e.status) {
        Some(404) => 3,
        Some(409) => 4,
        Some(401 | 403) => 5,
        _ => 1,
    }
}

#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub created_at: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RoleAssignment {
    pub role: String,
    pub database: Option<String>,
    pub assigned_at: String,
    pub assigned_by: String,
}

enum Credential {
    /// A JWT, from a login or given with --api-key
    Bearer(String),
    /// An `sk_…` API key
    ApiKey(String),
}

/// A blocking client for the user-management endpoints.
pub struct UserClient {
    http: Client,
    base: Url,
    credential: Credential,
}

impl UserClient {
    /// Log in as `username` and keep the token.
    pub fn login(server: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct LoginResponse {
            token: String,
        }

        let base = parse_server(server)?;
        let http = Client::new();
        let response = http
            .post(base.join("auth/login")?)
            .json(&serde_json::json!({ "username": username, "password": password }))
            .send()?;
        // The server answers a wrong password with 400, so as not to tell
        // a missing user from a wrong password; for the exit code it is a
        // refused authentication all the same.
        let response = check(response).map_err(|error| match error.downcast::<ApiError>() {
            Ok(api) if api.status == 400 => ApiError {
                status: 401,
                message: api.message,
            }
            .into(),
            Ok(api) => api.into(),
            Err(other) => other,
        })?;
        let login: LoginResponse = response.json()?;
        Ok(Self {
            http,
            base,
            credential: Credential::Bearer(login.token),
        })
    }

    /// Use an API key (`sk_…`) or a JWT as it is.
    pub fn with_key(server: &str, key: &str) -> anyhow::Result<Self> {
        let credential = if key.starts_with("sk_") {
            Credential::ApiKey(key.to_string())
        } else {
            Credential::Bearer(key.to_string())
        };
        Ok(Self {
            http: Client::new(),
            base: parse_server(server)?,
            credential,
        })
    }

    pub fn list_users(&self) -> anyhow::Result<serde_json::Value> {
        Ok(check(self.request(Method::GET, &["users"]).send()?)?.json()?)
    }

    pub fn create_user(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "username": username, "password": password });
        check(self.request(Method::POST, &["users"]).json(&body).send()?)?;
        Ok(())
    }

    pub fn delete_user(&self, username: &str) -> anyhow::Result<()> {
        check(self.request(Method::DELETE, &["users", username]).send()?)?;
        Ok(())
    }

    /// Admin reset of another user's password.
    pub fn set_password(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "password": password });
        let request = self.request(Method::PUT, &["users", username, "password"]);
        check(request.json(&body).send()?)?;
        Ok(())
    }

    /// The caller's own password, which needs the current one.
    pub fn change_own_password(&self, current: &str, new: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({ "current_password": current, "new_password": new });
        check(
            self.request(Method::PUT, &["password"])
                .json(&body)
                .send()?,
        )?;
        Ok(())
    }

    pub fn user_roles(&self, username: &str) -> anyhow::Result<serde_json::Value> {
        let request = self.request(Method::GET, &["users", username, "roles"]);
        Ok(check(request.send()?)?.json()?)
    }

    pub fn grant(&self, username: &str, role: &str, database: Option<&str>) -> anyhow::Result<()> {
        let body = serde_json::json!({ "role": role, "database": database });
        let request = self.request(Method::POST, &["users", username, "roles"]);
        check(request.json(&body).send()?)?;
        Ok(())
    }

    pub fn revoke(&self, username: &str, role: &str, database: Option<&str>) -> anyhow::Result<()> {
        let mut url = self.url(&["users", username, "roles", role]);
        if let Some(database) = database {
            url.query_pairs_mut().append_pair("database", database);
        }
        check(self.authorized(self.http.delete(url)).send()?)?;
        Ok(())
    }

    /// A request to `/_api/auth/<segments…>`, each segment percent-encoded,
    /// so a username cannot reach another path.
    fn request(&self, method: Method, segments: &[&str]) -> RequestBuilder {
        self.authorized(self.http.request(method, self.url(segments)))
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("an http(s) URL has path segments")
            .pop_if_empty()
            .extend(["_api", "auth"])
            .extend(segments);
        url
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.credential {
            Credential::Bearer(token) => request.bearer_auth(token),
            Credential::ApiKey(key) => request.header("X-API-Key", key),
        }
    }
}

fn parse_server(server: &str) -> anyhow::Result<Url> {
    let mut url = Url::parse(server)
        .map_err(|e| anyhow::anyhow!("Invalid server URL '{}': {}", server, e))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("Invalid server URL '{}': expected http or https", server);
    }
    // `join` replaces the last segment unless the path ends with '/'.
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// The response if it succeeded, else an [`ApiError`] with the server's message.
fn check(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| {
            if body.trim().is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("Request failed")
                    .to_string()
            } else {
                body.trim().to_string()
            }
        });
    Err(ApiError {
        status: status.as_u16(),
        message,
    }
    .into())
}

/// Run `solidb user …`.
pub fn execute(args: UserArgs) -> anyhow::Result<()> {
    let (client, own_password) = connect(&args.connection)?;

    match args.command {
        UserCommand::List { json } => {
            let users = client.list_users()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&users)?);
            } else {
                print_users(&users)?;
            }
        }
        UserCommand::Create {
            username,
            role,
            database,
            password_stdin,
        } => {
            let password = new_password(&username, password_stdin)?;
            client.create_user(&username, &password)?;
            if let Some(role) = role {
                // Assigned separately so it can be limited to a database, and
                // so an unknown role is an error rather than silently skipped.
                // A user without the role asked for is not what was asked:
                // take it back out.
                if let Err(error) = client.grant(&username, &role, database.as_deref()) {
                    let _ = client.delete_user(&username);
                    return Err(error.context(format!(
                        "User '{}' was not kept: assigning role '{}' failed",
                        username, role
                    )));
                }
            }
            println!("User '{}' created", username);
        }
        UserCommand::Delete { username } => {
            client.delete_user(&username)?;
            println!("User '{}' deleted", username);
        }
        UserCommand::Passwd {
            username,
            password_stdin,
        } => match username {
            Some(username) if username != args.connection.login_user || own_password.is_none() => {
                let password = new_password(&username, password_stdin)?;
                client.set_password(&username, &password)?;
                println!("Password of '{}' changed", username);
            }
            _ => {
                let current = own_password.ok_or_else(|| {
                    anyhow::anyhow!("Changing your own password needs a login, not an API key")
                })?;
                let me = &args.connection.login_user;
                let password = new_password(me, password_stdin)?;
                client.change_own_password(&current, &password)?;
                println!("Password of '{}' changed", me);
            }
        },
        UserCommand::Roles { username, json } => {
            let roles = client.user_roles(&username)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&roles)?);
            } else {
                print_roles(&username, &roles)?;
            }
        }
        UserCommand::Grant {
            username,
            role,
            database,
        } => {
            client.grant(&username, &role, database.as_deref())?;
            println!("Role '{}' granted to '{}'{}", role, username, on(&database));
        }
        UserCommand::Revoke {
            username,
            role,
            database,
        } => {
            client.revoke(&username, &role, database.as_deref())?;
            println!(
                "Role '{}' revoked from '{}'{}",
                role,
                username,
                on(&database)
            );
        }
    }
    Ok(())
}

fn on(database: &Option<String>) -> String {
    database
        .as_ref()
        .map(|d| format!(" on database '{}'", d))
        .unwrap_or_default()
}

/// The client, and the password it logged in with (None with an API key).
fn connect(connection: &ConnectionArgs) -> anyhow::Result<(UserClient, Option<String>)> {
    if let Some(key) = connection.api_key.as_deref().filter(|k| !k.is_empty()) {
        return Ok((UserClient::with_key(&connection.server, key)?, None));
    }

    let user = &connection.login_user;
    let from_env = connection.password_file.is_none()
        && std::env::var(password::PASSWORD_ENV).is_ok_and(|p| !p.is_empty());
    let admin_password =
        password::resolve_password(None, connection.password_file.as_deref(), Some(user))
            .map_err(anyhow::Error::msg)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No password for '{}': set {}, pass --password-file or --api-key, or run on \
                     a terminal to be prompted",
                    user,
                    password::PASSWORD_ENV
                )
            })?;

    match UserClient::login(&connection.server, user, &admin_password) {
        Ok(client) => Ok((client, Some(admin_password))),
        // SOLIDB_PASSWORD is often an application's own password, picked up
        // from a .env in the current directory: say where this one came from.
        Err(error) if from_env => Err(error.context(format!(
            "Login as '{}' with the password from {} failed",
            user,
            password::PASSWORD_ENV
        ))),
        Err(error) => Err(error.context(format!("Login as '{}' failed", user))),
    }
}

/// The password being set: typed twice on a terminal, or one line of stdin.
fn new_password(username: &str, from_stdin: bool) -> anyhow::Result<String> {
    if from_stdin {
        return read_password_line(std::io::stdin().lock());
    }
    if !(std::io::stdin().is_terminal() || std::io::stderr().is_terminal()) {
        anyhow::bail!("No terminal to prompt for the new password: pass --password-stdin");
    }
    let first = rpassword::prompt_password(format!("New password for {}: ", username))?;
    let second = rpassword::prompt_password("Again: ")?;
    if first != second {
        anyhow::bail!("The two passwords differ");
    }
    if first.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }
    Ok(first)
}

/// The first line of `reader`, without its line ending.
fn read_password_line(mut reader: impl BufRead) -> anyhow::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let password = line
        .strip_suffix("\r\n")
        .or_else(|| line.strip_suffix('\n'))
        .unwrap_or(&line);
    if password.is_empty() {
        anyhow::bail!("No password on stdin");
    }
    Ok(password.to_string())
}

fn print_users(value: &serde_json::Value) -> anyhow::Result<()> {
    #[derive(Deserialize)]
    struct Users {
        users: Vec<UserInfo>,
    }
    let mut users = serde_json::from_value::<Users>(value.clone())?.users;
    users.sort_by(|a, b| a.username.cmp(&b.username));
    let rows: Vec<[String; 3]> = users
        .into_iter()
        .map(|u| {
            [
                u.username,
                if u.roles.is_empty() {
                    "-".to_string()
                } else {
                    u.roles.join(", ")
                },
                u.created_at.unwrap_or_default(),
            ]
        })
        .collect();
    print_table(["USERNAME", "ROLES", "CREATED"], &rows);
    Ok(())
}

fn print_roles(username: &str, value: &serde_json::Value) -> anyhow::Result<()> {
    let roles: Vec<RoleAssignment> = serde_json::from_value(value.clone())?;
    if roles.is_empty() {
        println!("No roles assigned to '{}'", username);
        return Ok(());
    }
    let rows: Vec<[String; 4]> = roles
        .into_iter()
        .map(|r| {
            [
                r.role,
                r.database.unwrap_or_else(|| "(all)".to_string()),
                r.assigned_by,
                r.assigned_at,
            ]
        })
        .collect();
    print_table(["ROLE", "DATABASE", "ASSIGNED BY", "ASSIGNED AT"], &rows);
    Ok(())
}

fn print_table<const N: usize>(header: [&str; N], rows: &[[String; N]]) {
    let mut widths = header.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{:<width$}", cell, width = width))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(header.to_vec());
    for row in rows {
        line(row.iter().map(String::as_str).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(clap::Parser)]
    struct Cli {
        #[command(subcommand)]
        command: Top,
    }

    #[derive(clap::Subcommand)]
    enum Top {
        User(UserArgs),
    }

    fn parse(args: &[&str]) -> UserArgs {
        use clap::Parser;
        let Top::User(user) = Cli::try_parse_from(args).unwrap().command;
        user
    }

    #[test]
    fn the_login_user_and_the_target_user_stay_apart() {
        let args = parse(&["solidb", "user", "create", "shop_app"]);
        assert_eq!(args.connection.login_user, "admin");
        assert!(
            matches!(args.command, UserCommand::Create { ref username, .. } if username == "shop_app")
        );

        let args = parse(&[
            "solidb", "user", "-u", "root", "grant", "shop_app", "viewer",
        ]);
        assert_eq!(args.connection.login_user, "root");
        assert!(
            matches!(args.command, UserCommand::Grant { ref username, .. } if username == "shop_app")
        );

        // Global, so it may also follow the subcommand.
        let args = parse(&["solidb", "user", "delete", "shop_app", "--username", "root"]);
        assert_eq!(args.connection.login_user, "root");
        assert!(
            matches!(args.command, UserCommand::Delete { ref username } if username == "shop_app")
        );
    }

    #[test]
    fn database_needs_a_role_on_create() {
        use clap::Parser;
        assert!(
            Cli::try_parse_from(["solidb", "user", "create", "a", "--database", "shop"]).is_err()
        );
    }

    #[test]
    fn exit_codes_tell_refusals_apart() {
        let api = |status| {
            anyhow::Error::from(ApiError {
                status,
                message: String::new(),
            })
        };
        assert_eq!(exit_code(&api(404)), 3);
        assert_eq!(exit_code(&api(409)), 4);
        assert_eq!(exit_code(&api(401)), 5);
        assert_eq!(exit_code(&api(403)), 5);
        assert_eq!(exit_code(&api(400)), 1);
        assert_eq!(exit_code(&anyhow::anyhow!("connection refused")), 1);
        // Context added on top keeps the code.
        assert_eq!(exit_code(&api(409).context("while creating")), 4);
    }

    #[test]
    fn password_line_drops_only_the_line_ending() {
        assert_eq!(read_password_line(&b"secret\n"[..]).unwrap(), "secret");
        assert_eq!(
            read_password_line(&b"secret\r\nnext\n"[..]).unwrap(),
            "secret"
        );
        assert_eq!(read_password_line(&b" spaced \n"[..]).unwrap(), " spaced ");
        assert_eq!(
            read_password_line(&b"no-newline"[..]).unwrap(),
            "no-newline"
        );
        assert!(read_password_line(&b"\n"[..]).is_err());
        assert!(read_password_line(&b""[..]).is_err());
    }

    #[test]
    fn server_urls_are_checked_and_keep_their_path() {
        assert_eq!(
            parse_server("http://localhost:6745").unwrap().as_str(),
            "http://localhost:6745/"
        );
        assert_eq!(
            parse_server("https://db.example.com/solidb")
                .unwrap()
                .as_str(),
            "https://db.example.com/solidb/"
        );
        assert!(parse_server("localhost:6745").is_err());
        assert!(parse_server("ftp://example.com").is_err());
    }

    #[test]
    fn path_segments_are_encoded() {
        let client = UserClient::with_key("http://localhost:6745/base", "sk_test").unwrap();
        let request = client
            .request(Method::GET, &["users", "../x?y", "roles"])
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "http://localhost:6745/base/_api/auth/users/..%2Fx%3Fy/roles"
        );
        assert_eq!(request.headers()["X-API-Key"], "sk_test");
    }
}
