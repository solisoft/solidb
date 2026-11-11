//! `solidb user` against a real server: the CLI client over HTTP, and the
//! admin password reset it relies on (`PUT /_api/auth/users/{username}/password`).

use solidb::cli::user::{exit_code, UserClient};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

mod common;

struct Server {
    url: String,
    admin_token: String,
    viewer_token: String,
    _tmp: TempDir,
}

/// A server on a free local port, with one admin and one viewer.
async fn start_server() -> Server {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    engine
        .create_database("shop".to_string())
        .expect("create database");
    let router = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    // After `create_router`, which only seeds the default admin into an
    // empty `_admins` (see rbac_admin_endpoints_tests).
    let admin_token = common::seed_user_token(&engine, "root", &["admin"]);
    let viewer_token = common::seed_user_token(&engine, "watcher", &["viewer"]);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    Server {
        url,
        admin_token,
        viewer_token,
        _tmp: tmp,
    }
}

/// The CLI client is blocking: run it off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.unwrap()
}

fn roles_of(client: &UserClient, username: &str) -> Vec<(String, Option<String>)> {
    let roles = client.user_roles(username).unwrap();
    let mut out: Vec<(String, Option<String>)> = roles
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["role"].as_str().unwrap().to_string(),
                r["database"].as_str().map(str::to_string),
            )
        })
        .collect();
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn create_grant_revoke_passwd_delete() {
    let server = start_server().await;
    let (url, token) = (server.url.clone(), server.admin_token.clone());

    blocking(move || {
        let admin = UserClient::with_key(&url, &token).unwrap();

        admin.create_user("app", "first-password-123").unwrap();
        let users = admin.list_users().unwrap();
        assert!(users["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == "app"));

        // A second create is "already exists" (exit code 4), not a failure.
        let again = admin.create_user("app", "first-password-123").unwrap_err();
        assert_eq!(exit_code(&again), 4);

        // The new user can log in with the password it was given.
        UserClient::login(&url, "app", "first-password-123").unwrap();

        // A role limited to one database, and a global one.
        admin.grant("app", "editor", Some("shop")).unwrap();
        admin.grant("app", "viewer", None).unwrap();
        assert_eq!(
            roles_of(&admin, "app"),
            vec![
                ("editor".to_string(), Some("shop".to_string())),
                ("viewer".to_string(), None),
            ]
        );

        // Revoking needs the same scope as the assignment.
        let wrong_scope = admin.revoke("app", "editor", None).unwrap_err();
        assert_eq!(exit_code(&wrong_scope), 3);
        admin.revoke("app", "editor", Some("shop")).unwrap();
        assert_eq!(roles_of(&admin, "app"), vec![("viewer".to_string(), None)]);

        // Unknown role, unknown database: refused, nothing assigned.
        assert_eq!(exit_code(&admin.grant("app", "nope", None).unwrap_err()), 3);
        assert!(admin.grant("app", "editor", Some("no_such_db")).is_err());
        assert_eq!(roles_of(&admin, "app"), vec![("viewer".to_string(), None)]);

        // Admin reset: the old password stops working, the new one works.
        admin.set_password("app", "second-password-456").unwrap();
        assert_eq!(
            exit_code(
                &UserClient::login(&url, "app", "first-password-123")
                    .err()
                    .unwrap()
            ),
            5,
            "a refused login is a refused authentication"
        );
        let app = UserClient::login(&url, "app", "second-password-456").unwrap();

        // A user changes its own password with the current one.
        app.change_own_password("second-password-456", "third-password-789")
            .unwrap();
        UserClient::login(&url, "app", "third-password-789").unwrap();

        admin.delete_user("app").unwrap();
        assert_eq!(exit_code(&admin.delete_user("app").unwrap_err()), 3);
        assert!(UserClient::login(&url, "app", "third-password-789").is_err());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn password_reset_is_checked() {
    let server = start_server().await;
    let (url, admin, viewer) = (
        server.url.clone(),
        server.admin_token.clone(),
        server.viewer_token.clone(),
    );

    blocking(move || {
        let admin = UserClient::with_key(&url, &admin).unwrap();
        admin.create_user("app", "first-password-123").unwrap();

        // Too short: refused, and the old password still works.
        let short = admin.set_password("app", "short").unwrap_err();
        assert!(short.to_string().contains("at least 12"), "{}", short);
        UserClient::login(&url, "app", "first-password-123").unwrap();

        // Unknown user: not found, not created.
        assert_eq!(
            exit_code(
                &admin
                    .set_password("ghost", "long-enough-password")
                    .unwrap_err()
            ),
            3
        );
        let users = admin.list_users().unwrap();
        assert!(!users["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == "ghost"));

        // A viewer may not reset anyone's password, nor manage users.
        let viewer = UserClient::with_key(&url, &viewer).unwrap();
        assert_eq!(
            exit_code(
                &viewer
                    .set_password("app", "taken-over-password")
                    .unwrap_err()
            ),
            5
        );
        UserClient::login(&url, "app", "first-password-123").unwrap();
        assert_eq!(
            exit_code(
                &viewer
                    .create_user("intruder", "long-enough-password")
                    .unwrap_err()
            ),
            5
        );
        assert_eq!(
            exit_code(&viewer.grant("watcher", "admin", None).unwrap_err()),
            5
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn odd_usernames_stay_in_their_path_segment() {
    let server = start_server().await;
    let (url, token) = (server.url.clone(), server.admin_token.clone());

    blocking(move || {
        let admin = UserClient::with_key(&url, &token).unwrap();
        admin
            .create_user("a/b?c#d", "long-enough-password")
            .unwrap();
        admin.grant("a/b?c#d", "viewer", None).unwrap();
        assert_eq!(
            roles_of(&admin, "a/b?c#d"),
            vec![("viewer".to_string(), None)]
        );
        admin
            .set_password("a/b?c#d", "another-long-password")
            .unwrap();
        UserClient::login(&url, "a/b?c#d", "another-long-password").unwrap();
        admin.delete_user("a/b?c#d").unwrap();
    })
    .await;
}
