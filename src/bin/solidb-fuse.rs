use clap::Parser;
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, Request,
};
use libc::{EIO, ENOENT};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Utc};
use tracing::{debug, error, info};
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u64 = 512;

#[derive(Parser, Debug)]
#[command(name = "solidb-fuse")]
#[command(about = "Mount SolidB blob collections as a filesystem", long_about = None)]
struct Args {
    #[arg(long, default_value = "localhost")]
    host: String,

    #[arg(long, default_value_t = 6745)]
    port: u16,

    #[arg(long, default_value = "admin")]
    username: String,

    #[arg(long)]
    password: Option<String>,

    #[arg(long, help = "Mount point path")]
    mount: String,

    #[arg(long, default_value_t = false, help = "Run in foreground")]
    foreground: bool,

    #[arg(short, long, help = "Run as a daemon (background process)")]
    daemon: bool,

    #[arg(
        long,
        default_value = "./solidb-fuse.pid",
        help = "PID file path (used in daemon mode)"
    )]
    pid_file: String,

    #[arg(
        long,
        default_value = "./solidb-fuse.log",
        help = "Log file path (used in daemon mode)"
    )]
    log_file: String,
}

#[derive(Debug, Deserialize)]
struct DatabaseList {
    databases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Collection {
    name: String,
    #[serde(default, rename = "type")]
    r#type: String,
}

#[derive(Debug, Deserialize)]
struct CollectionList {
    collections: Vec<Collection>,
}

#[derive(Debug, Deserialize, Clone)]
struct BlobMetadata {
    _key: String,
    #[serde(default, alias = "name")]
    filename: Option<String>,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Deserialize)]
struct CursorResponse<T> {
    result: Vec<T>,
}

#[derive(Debug, serde::Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    token: String,
}

#[derive(Debug)]
struct SolidBClient {
    client: reqwest::blocking::Client,
    base_url: String,
    username: String,
    password: Option<String>,
    token: Option<String>,
    // Cache for list_databases: (timestamp, data)
    db_list_cache: Option<(std::time::Instant, Vec<String>)>,
}

impl SolidBClient {
    fn new(host: &str, port: u16, username: &str, password: Option<&str>) -> Self {
        let base_url = format!("http://{}:{}", host, port);

        let mut client = Self {
            client: reqwest::blocking::Client::new(),
            base_url,
            username: username.to_string(),
            password: password.map(|s| s.to_string()),
            token: None,
            db_list_cache: None,
        };

        // Attempt login immediately
        if let Err(e) = client.authenticate() {
            error!(
                "Initial authentication failed: {}. Subsequent requests may fail.",
                e
            );
        }

        client
    }

    fn authenticate(&mut self) -> Result<(), anyhow::Error> {
        if let Some(pass) = &self.password {
            // Correct auth endpoint is /auth/login (not /_api/auth/login)
            let url = format!("{}/auth/login", self.base_url);
            let body = LoginRequest {
                username: &self.username,
                password: pass,
            };

            info!("Authenticating as {} to {}...", self.username, url);
            let resp = self.client.post(&url).json(&body).send()?;

            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().unwrap_or_default();
                error!("Login failed with status {}. Response: {}", status, text);
                return Err(anyhow::anyhow!("Login failed: {}", status));
            }

            match resp.json::<LoginResponse>() {
                Ok(data) => {
                    self.token = Some(data.token);
                    info!("Authentication successful. Token received.");
                }
                Err(e) => {
                    error!("Failed to parse login response: {}", e);
                    return Err(e.into());
                }
            }
        }
        Ok(())
    }

    fn list_databases(&mut self) -> Result<Vec<String>, anyhow::Error> {
        // Check cache (TTL 5 seconds)
        if let Some((ts, dbs)) = &self.db_list_cache {
            if ts.elapsed() < Duration::from_secs(5) {
                return Ok(dbs.clone());
            }
        }

        // Correct endpoint is /_api/databases (plural)
        let url = format!("{}/_api/databases", self.base_url);
        info!("Fetching databases from {}", url);

        let mut req = self.client.get(&url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        } else {
            // Fallback or Basic? Server only supports Bearer probably.
            // Try Basic if no token and no password? No, token is key.
            if self.password.is_some() {
                // Try re-auth?
                // For now, if token is missing and we have password, maybe we failed init.
            }
        }

        let resp = req.send()?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Try re-login once
            info!("Got 401, attempting re-login...");
            self.authenticate()?;
            let mut req = self.client.get(&url);
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
            let resp = req.send()?;
            if !resp.status().is_success() {
                error!(
                    "API Error listing databases after re-login: Status {}",
                    resp.status()
                );
            }
            let list = resp.json::<DatabaseList>()?;

            // Update cache
            self.db_list_cache = Some((std::time::Instant::now(), list.databases.clone()));
            return Ok(list.databases);
        }

        if !resp.status().is_success() {
            error!("API Error listing databases: Status {}", resp.status());
        }

        let list = resp.json::<DatabaseList>()?;

        // Update cache
        self.db_list_cache = Some((std::time::Instant::now(), list.databases.clone()));

        Ok(list.databases)
    }

    fn list_collections(&self, db: &str) -> Result<Vec<String>, anyhow::Error> {
        let url = format!("{}/_api/database/{}/collection", self.base_url, db);
        let mut req = self.client.get(&url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        let resp = req.send()?.json::<CollectionList>()?;

        Ok(resp
            .collections
            .into_iter()
            .filter(|c| c.r#type == "blob")
            .map(|c| c.name)
            .collect())
    }

    fn list_blobs(&self, db: &str, coll: &str) -> Result<Vec<BlobMetadata>, anyhow::Error> {
        // Endpoint is /_api/database/:db/cursor
        let url = format!("{}/_api/database/{}/cursor", self.base_url, db);
        let query = format!("FOR doc IN {} RETURN doc", coll);
        let body = serde_json::json!({
            "query": query,
            "database": db
        });

        let mut req = self.client.post(&url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        let resp = req
            .json(&body)
            .send()?
            .json::<CursorResponse<BlobMetadata>>()?;

        Ok(resp.result)
    }

    fn get_blob_content(&self, db: &str, coll: &str, key: &str) -> Result<Vec<u8>, anyhow::Error> {
        let url = format!("{}/_api/blob/{}/{}/{}", self.base_url, db, coll, key);
        let mut req = self.client.get(&url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        let resp = req.send()?;

        if resp.status().is_success() {
            Ok(resp.bytes()?.to_vec())
        } else {
            Err(anyhow::anyhow!("Failed to fetch blob: {}", resp.status()))
        }
    }
}

#[derive(Debug, Clone)]
enum InodeType {
    Root,
    Database(String),
    Collection(String, String),         // db, coll
    Year(String, String, i32),          // db, coll, year
    Month(String, String, i32, u32),    // db, coll, year, month
    Day(String, String, i32, u32, u32), // db, coll, year, month, day
    Blob(String, String, BlobMetadata), // db, coll, metadata
}

struct SolidBFS {
    client: Arc<Mutex<SolidBClient>>,
    inodes: HashMap<u64, InodeType>,
    next_inode: u64,
    uid: u32,
    gid: u32,
}

impl SolidBFS {
    fn new(client: SolidBClient) -> Self {
        let mut inodes = HashMap::new();
        inodes.insert(1, InodeType::Root);
        unsafe {
            Self {
                client: Arc::new(Mutex::new(client)),
                inodes,
                next_inode: 2,
                uid: libc::getuid(),
                gid: libc::getgid(),
            }
        }
    }

    fn allocate_inode(&mut self, kind: InodeType) -> u64 {
        // Linear search to deduplicate would be slow.
        // For now, simpler: just allocate. Kernel handles lookup caching.
        // If we want stable inodes, we need a reverse map.
        // Let's rely on readdir/lookup being consistent during a session.
        let ino = self.next_inode;
        self.next_inode += 1;
        self.inodes.insert(ino, kind);
        ino
    }

    fn get_inode(&self, ino: u64) -> Option<&InodeType> {
        self.inodes.get(&ino)
    }

    fn get_file_attr(&self, ino: u64) -> Option<FileAttr> {
        let kind = self.get_inode(ino)?;
        Some(match kind {
            InodeType::Root
            | InodeType::Database(_)
            | InodeType::Collection(_, _)
            | InodeType::Year(_, _, _)
            | InodeType::Month(_, _, _, _)
            | InodeType::Day(_, _, _, _, _) => FileAttr {
                ino,
                size: 0,
                blocks: 0,
                atime: UNIX_EPOCH,
                mtime: UNIX_EPOCH,
                ctime: UNIX_EPOCH,
                crtime: UNIX_EPOCH,
                kind: FileType::Directory,
                perm: 0o755,
                nlink: 2,
                uid: self.uid,
                gid: self.gid,
                rdev: 0,
                flags: 0,
                blksize: 512,
            },
            InodeType::Blob(_, _, meta) => {
                FileAttr {
                    ino,
                    size: meta.size,
                    blocks: (meta.size + BLOCK_SIZE - 1) / BLOCK_SIZE,
                    atime: UNIX_EPOCH,
                    mtime: UNIX_EPOCH, // Could extract from UUID if desired
                    ctime: UNIX_EPOCH,
                    crtime: UNIX_EPOCH,
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: self.uid,
                    gid: self.gid,
                    rdev: 0,
                    flags: 0,
                    blksize: 512,
                }
            }
        })
    }
}

impl Filesystem for SolidBFS {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        // This is inefficient: we reconstruct children to find the matching one.
        // In a real optimized FS we would cache children.
        // Here we just "simulate" finding it.

        let parent_kind = match self.get_inode(parent) {
            Some(k) => k.clone(),
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        let client_arc = self.client.clone();
        let mut client = client_arc.lock().unwrap();

        // debug!("FUSE: lookup called for {} in parent {}", name_str, parent);

        match parent_kind {
            InodeType::Root => {
                // Check databases
                if let Ok(dbs) = client.list_databases() {
                    if dbs.contains(&name_str.to_string()) {
                        drop(client); // Release lock before mutating
                        let ino = self.allocate_inode(InodeType::Database(name_str.to_string()));
                        reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                        return;
                    }
                }
            }
            InodeType::Database(db) => {
                // Check collections
                if let Ok(colls) = client.list_collections(&db) {
                    if colls.contains(&name_str.to_string()) {
                        drop(client);
                        let ino =
                            self.allocate_inode(InodeType::Collection(db, name_str.to_string()));
                        reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                        return;
                    }
                }
            }
            InodeType::Collection(db, coll) => {
                // Looking for Year?
                if let Ok(year) = name_str.parse::<i32>() {
                    // Verify if any blob exists in this year?
                    // Optimization: Just assume it exists or fetch all blobs to check.
                    // Fetched blobs are needed for hierarchy anyway.
                    if let Ok(blobs) = client.list_blobs(&db, &coll) {
                        let has_year = blobs.iter().any(|b| {
                            if let Ok(uuid) = Uuid::parse_str(&b._key) {
                                if let Some(ts) = uuid.get_timestamp() {
                                    let (secs, _) = ts.to_unix();
                                    let dt = DateTime::<Utc>::from(
                                        UNIX_EPOCH + Duration::from_secs(secs),
                                    );
                                    return dt.year() == year;
                                }
                            }
                            false
                        });
                        if has_year {
                            drop(client);
                            let ino = self.allocate_inode(InodeType::Year(db, coll, year));
                            reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                            return;
                        }
                    }
                }
            }
            InodeType::Year(db, coll, year) => {
                if let Ok(month) = name_str.parse::<u32>() {
                    // Check month
                    if let Ok(blobs) = client.list_blobs(&db, &coll) {
                        let has_month = blobs.iter().any(|b| {
                            if let Ok(uuid) = Uuid::parse_str(&b._key) {
                                if let Some(ts) = uuid.get_timestamp() {
                                    let (secs, _) = ts.to_unix();
                                    let dt = DateTime::<Utc>::from(
                                        UNIX_EPOCH + Duration::from_secs(secs),
                                    );
                                    return dt.year() == year && dt.month() == month;
                                }
                            }
                            false
                        });
                        if has_month {
                            drop(client);
                            let ino = self.allocate_inode(InodeType::Month(db, coll, year, month));
                            reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                            return;
                        }
                    }
                }
            }
            InodeType::Month(db, coll, year, month) => {
                if let Ok(day) = name_str.parse::<u32>() {
                    // Check day
                    if let Ok(blobs) = client.list_blobs(&db, &coll) {
                        let has_day = blobs.iter().any(|b| {
                            if let Ok(uuid) = Uuid::parse_str(&b._key) {
                                if let Some(ts) = uuid.get_timestamp() {
                                    let (secs, _) = ts.to_unix();
                                    let dt = DateTime::<Utc>::from(
                                        UNIX_EPOCH + Duration::from_secs(secs),
                                    );
                                    return dt.year() == year
                                        && dt.month() == month
                                        && dt.day() == day;
                                }
                            }
                            false
                        });
                        if has_day {
                            drop(client);
                            let ino =
                                self.allocate_inode(InodeType::Day(db, coll, year, month, day));
                            reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                            return;
                        }
                    }
                }
            }
            InodeType::Day(db, coll, year, month, day) => {
                // Find file
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    if let Some(blob) = blobs.into_iter().find(|b| {
                        let target_name = b.filename.as_deref().unwrap_or(&b._key);
                        if target_name != name_str {
                            return false;
                        }

                        if let Ok(uuid) = Uuid::parse_str(&b._key) {
                            if let Some(ts) = uuid.get_timestamp() {
                                let (secs, _) = ts.to_unix();
                                let dt =
                                    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                return dt.year() == year && dt.month() == month && dt.day() == day;
                            }
                        }
                        false
                    }) {
                        drop(client);
                        let ino = self.allocate_inode(InodeType::Blob(db, coll, blob));
                        reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                        return;
                    }
                }
            }
            _ => {}
        }

        reply.error(ENOENT);
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if let Some(attr) = self.get_file_attr(ino) {
            reply.attr(&TTL, &attr);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let parent_kind = match self.get_inode(ino) {
            Some(k) => k.clone(),
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if offset == 0 {
            if reply.add(ino, 0, FileType::Directory, ".") {
                return;
            }
            if reply.add(ino, 1, FileType::Directory, "..") {
                return;
            }
        }

        let client_arc = self.client.clone();
        let mut client = client_arc.lock().unwrap();
        let mut entries = Vec::new();

        // 0 and 1 are already sent.
        // We accumulate entries then skip based on offset (simplification)
        // Offset logic in FUSE is tricky. We'll use index + 2 as offset.

        // debug!("FUSE: readdir called on inode {}", ino);

        match parent_kind {
            InodeType::Root => {
                match client.list_databases() {
                    Ok(dbs) => {
                        // debug!("FUSE: Found databases: {:?}", dbs);
                        info!("Listing databases: {:?}", dbs);
                        for db in dbs {
                            let child_ino = self.allocate_inode(InodeType::Database(db.clone()));
                            entries.push((child_ino, FileType::Directory, db));
                        }
                    }
                    Err(e) => error!("Failed to list databases: {:?}", e),
                }
            }
            InodeType::Database(db) => match client.list_collections(&db) {
                Ok(colls) => {
                    debug!("Listing collections for {}: {:?}", db, colls);
                    for coll in colls {
                        let child_ino =
                            self.allocate_inode(InodeType::Collection(db.clone(), coll.clone()));
                        entries.push((child_ino, FileType::Directory, coll));
                    }
                }
                Err(e) => error!("Failed to list collections for {}: {:?}", db, e),
            },
            InodeType::Collection(db, coll) => {
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    let mut years = HashSet::new();
                    for b in blobs {
                        if let Ok(uuid) = Uuid::parse_str(&b._key) {
                            if let Some(ts) = uuid.get_timestamp() {
                                let (secs, _) = ts.to_unix();
                                let dt =
                                    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                years.insert(dt.year());
                            }
                        }
                    }
                    let mut sorted_years: Vec<_> = years.into_iter().collect();
                    sorted_years.sort();
                    for year in sorted_years {
                        let child_ino =
                            self.allocate_inode(InodeType::Year(db.clone(), coll.clone(), year));
                        entries.push((child_ino, FileType::Directory, year.to_string()));
                    }
                }
            }
            InodeType::Year(db, coll, year) => {
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    let mut months = HashSet::new();
                    for b in blobs {
                        if let Ok(uuid) = Uuid::parse_str(&b._key) {
                            if let Some(ts) = uuid.get_timestamp() {
                                let (secs, _) = ts.to_unix();
                                let dt =
                                    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                if dt.year() == year {
                                    months.insert(dt.month());
                                }
                            }
                        }
                    }
                    let mut sorted_months: Vec<_> = months.into_iter().collect();
                    sorted_months.sort();
                    for month in sorted_months {
                        let child_ino = self.allocate_inode(InodeType::Month(
                            db.clone(),
                            coll.clone(),
                            year,
                            month,
                        ));
                        // Pad month 01, 02...
                        entries.push((child_ino, FileType::Directory, format!("{:02}", month)));
                    }
                }
            }
            InodeType::Month(db, coll, year, month) => {
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    let mut days = HashSet::new();
                    for b in blobs {
                        if let Ok(uuid) = Uuid::parse_str(&b._key) {
                            if let Some(ts) = uuid.get_timestamp() {
                                let (secs, _) = ts.to_unix();
                                let dt =
                                    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                if dt.year() == year && dt.month() == month {
                                    days.insert(dt.day());
                                }
                            }
                        }
                    }
                    let mut sorted_days: Vec<_> = days.into_iter().collect();
                    sorted_days.sort();
                    for day in sorted_days {
                        let child_ino = self.allocate_inode(InodeType::Day(
                            db.clone(),
                            coll.clone(),
                            year,
                            month,
                            day,
                        ));
                        entries.push((child_ino, FileType::Directory, format!("{:02}", day)));
                    }
                }
            }
            InodeType::Day(db, coll, year, month, day) => {
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    for b in blobs {
                        if let Ok(uuid) = Uuid::parse_str(&b._key) {
                            if let Some(ts) = uuid.get_timestamp() {
                                let (secs, _) = ts.to_unix();
                                let dt =
                                    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                if dt.year() == year && dt.month() == month && dt.day() == day {
                                    let name = b.filename.clone().unwrap_or_else(|| b._key.clone());
                                    let child_ino = self.allocate_inode(InodeType::Blob(
                                        db.clone(),
                                        coll.clone(),
                                        b,
                                    ));
                                    entries.push((child_ino, FileType::RegularFile, name));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        // Apply offset
        let start_idx = if offset > 1 { (offset - 2) as usize } else { 0 };

        for (i, (ino, kind, name)) in entries.into_iter().enumerate().skip(start_idx) {
            if reply.add(ino, (i + 3) as i64, kind, name) {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, _ino: u64, _flags: i32, reply: ReplyOpen) {
        // Read-only check? flags & O_ACCMODE == O_RDONLY
        // Minimal impl: allow everything, read() will fail if it's directory
        reply.opened(0, 0);
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let inode_kind = match self.get_inode(ino) {
            Some(k) => k.clone(),
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        match inode_kind {
            InodeType::Blob(db, coll, meta) => {
                let client = self.client.lock().unwrap();
                match client.get_blob_content(&db, &coll, &meta._key) {
                    Ok(data) => {
                        let start = offset as usize;
                        if start >= data.len() {
                            reply.data(&[]);
                        } else {
                            let end = (start + size as usize).min(data.len());
                            reply.data(&data[start..end]);
                        }
                    }
                    Err(_) => reply.error(EIO),
                }
            }
            _ => reply.error(EIO), // Should contain EISDIR equivalent
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    #[cfg(unix)]
    if args.daemon {
        use daemonize::Daemonize;
        use std::fs::File;
        use std::path::Path;
        use std::time::Duration;

        // Check if PID file exists and kill existing process
        if Path::new(&args.pid_file).exists() {
            match std::fs::read_to_string(&args.pid_file) {
                Ok(pid_str) => {
                    if let Ok(pid) = pid_str.trim().parse::<i32>() {
                        eprintln!(
                            "Found existing FUSE process with PID {}. Stopping it...",
                            pid
                        );
                        // Send SIGTERM to gracefully stop the process
                        unsafe {
                            libc::kill(pid, libc::SIGTERM);
                        }
                        // Give it a moment to cleanup mounts
                        std::thread::sleep(Duration::from_millis(500));
                        let _ = std::fs::remove_file(&args.pid_file);
                    }
                }
                Err(e) => eprintln!("Warning: Could not read PID file: {}", e),
            }
        }

        let stdout = File::create(&args.log_file)?;
        let stderr = File::create(&args.log_file)?;

        let daemonize = Daemonize::new()
            .pid_file(&args.pid_file)
            // .working_directory(".") // Don't change dir, might break relative mount paths
            .stdout(stdout)
            .stderr(stderr);

        match daemonize.start() {
            Ok(_) => {
                // We're now in the daemon process
            }
            Err(e) => {
                eprintln!("Error starting daemon: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(unix))]
    if args.daemon {
        eprintln!("Daemon mode is only supported on Unix systems");
        std::process::exit(1);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let mountpoint = args.mount;

    // Check mountpoint
    if !std::path::Path::new(&mountpoint).exists() {
        // error!("Mount point '{}' does not exist. Attempting to create it...", mountpoint);
        std::fs::create_dir_all(&mountpoint)?;
    }

    // Options
    let options = vec![MountOption::RO, MountOption::FSName("solidb".to_string())];
    #[cfg(target_os = "macos")]
    let mut options = options;
    #[cfg(target_os = "macos")]
    options.push(MountOption::AutoUnmount);

    println!("Attempting to mount at {}", mountpoint);
    let client = SolidBClient::new(
        &args.host,
        args.port,
        &args.username,
        args.password.as_deref(),
    );
    let fs = SolidBFS::new(client);

    // We always use spawn_mount2 (background thread) and keep the main thread
    // alive with Tokio to handle signals and explicit unmounting.

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        // Spawn the filesystem in a background thread
        let session = fuser::spawn_mount2(fs, &mountpoint, &options)?;

        info!("Mount session started successfully.");
        println!("Filesystem mounted at {}.", mountpoint);
        println!("Press Ctrl+C to unmount.");

        // Wait for Ctrl+C (or SIGTERM if daemon)
        let ctrl_c = tokio::signal::ctrl_c();
        let sig_term = async {
            #[cfg(unix)]
            {
                let mut stream =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .unwrap();
                stream.recv().await;
            }
            #[cfg(not(unix))]
            std::future::pending::<()>().await;
        };

        tokio::select! {
            _ = ctrl_c => { println!("\nReceived Ctrl+C"); },
            _ = sig_term => { println!("\nReceived SIGTERM"); },
        }

        println!("Unmounting {}...", mountpoint);
        info!("Unmounting {}...", mountpoint);

        // Explicitly run system unmount command to be sure
        #[cfg(target_os = "linux")]
        {
            let status = std::process::Command::new("fusermount")
                .arg("-u")
                .arg(&mountpoint)
                .status();

            match status {
                Ok(s) if s.success() => info!("'fusermount -u' executed successfully."),
                Ok(s) => error!("'fusermount -u' failed with exit code: {:?}", s.code()),
                Err(e) => error!("Failed to execute 'fusermount -u': {}", e),
            }
        }

        #[cfg(target_os = "macos")]
        {
            let status = std::process::Command::new("umount")
                .arg(&mountpoint)
                .status();

            match status {
                Ok(s) if s.success() => info!("'umount' executed successfully."),
                Ok(s) => error!("'umount' failed with exit code: {:?}", s.code()),
                Err(e) => error!("Failed to execute 'umount': {}", e),
            }
        }

        // Dropping the session also triggers unmount (if not already done)
        drop(session);

        // Give it a moment to clean up
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok::<(), anyhow::Error>(())
    })?;

    println!("Exiting.");
    Ok(())
}
