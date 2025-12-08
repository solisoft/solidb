use clap::Parser;
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, Request,
};
use libc::{ENOENT, ENOSYS, EIO};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use std::io::Read; // Keep needed if we use Read trait? Actually read() on response uses it implicitly? 
// No, reqwest response.bytes() doesn't need Read trait imported unless using read_to_end on reader.
// The error said it was unused.

use chrono::{DateTime, Utc, Datelike, TimeZone};
use uuid::Uuid;
use tracing::{info, error, debug};

const TTL: Duration = Duration::from_secs(1);
const BLOCK_SIZE: u64 = 512;

#[derive(Parser, Debug)]
#[command(name = "solidb-fuse")]
#[command(about = "Mount SolidB blob collections as a filesystem", long_about = None)]
struct Args {
    #[arg(long, default_value = "localhost")]
    host: String,

    #[arg(long, default_value_t = 6755)]
    port: u16,

    #[arg(long, default_value = "admin")]
    username: String,

    #[arg(long)]
    password: Option<String>,

    #[arg(long, help = "Mount point path")]
    mount: String,

    #[arg(long, default_value_t = false, help = "Run in foreground")]
    foreground: bool,
}

#[derive(Debug, Deserialize)]
struct DatabaseList {
    result: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Collection {
    id: String,
    name: String,
    #[serde(default)]
    r#type: String, 
}

#[derive(Debug, Deserialize)]
struct CollectionList {
    result: Vec<Collection>,
}

#[derive(Debug, Deserialize, Clone)]
struct BlobMetadata {
    _key: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    size: u64,
}

#[derive(Debug, Deserialize)]
struct CursorResponse<T> {
    result: Vec<T>,
}

struct SolidBClient {
    client: reqwest::blocking::Client,
    base_url: String,
    // Add auth headers if needed
    username: String,
    password: Option<String>,
}

impl SolidBClient {
    fn new(host: &str, port: u16, username: &str, password: Option<&str>) -> Self {
        let base_url = format!("http://{}:{}", host, port);
        Self {
            client: reqwest::blocking::Client::new(),
            base_url,
            username: username.to_string(),
            password: password.map(|s| s.to_string()),
        }
    }

    fn list_databases(&self) -> Result<Vec<String>, anyhow::Error> {
        let url = format!("{}/_api/database", self.base_url);
        info!("Fetching databases from {}", url);
        let resp = self.client.get(&url)
            .basic_auth(&self.username, self.password.as_deref())
            .send()?;
        
        if !resp.status().is_success() {
            error!("API Error listing databases: Status {}", resp.status());
        }
        
        let list = resp.json::<DatabaseList>()?;
        Ok(list.result)
    }

    fn list_collections(&self, db: &str) -> Result<Vec<String>, anyhow::Error> {
        let url = format!("{}/_api/database/{}/collection", self.base_url, db);
        let resp = self.client.get(&url)
            .basic_auth(&self.username, self.password.as_deref())
            .send()?
            .json::<CollectionList>()?;
        
        Ok(resp.result.into_iter()
            .filter(|c| c.r#type == "blob")
            .map(|c| c.name)
            .collect())
    }

    fn list_blobs(&self, db: &str, coll: &str) -> Result<Vec<BlobMetadata>, anyhow::Error> {
        let url = format!("{}/_api/cursor", self.base_url);
        let query = format!("FOR doc IN {} RETURN doc", coll);
        let body = serde_json::json!({
            "query": query,
            "database": db
        });

        let resp = self.client.post(&url)
            .basic_auth(&self.username, self.password.as_deref())
            .json(&body)
            .send()?
            .json::<CursorResponse<BlobMetadata>>()?;
        
        Ok(resp.result)
    }
    
    fn get_blob_content(&self, db: &str, coll: &str, key: &str) -> Result<Vec<u8>, anyhow::Error> {
        let url = format!("{}/_api/blob/{}/{}/{}", self.base_url, db, coll, key);
         let resp = self.client.get(&url)
            .basic_auth(&self.username, self.password.as_deref())
            .send()?;
        
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
    Collection(String, String), // db, coll
    Year(String, String, i32), // db, coll, year
    Month(String, String, i32, u32), // db, coll, year, month
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
            InodeType::Root | InodeType::Database(_) | InodeType::Collection(_, _) |
            InodeType::Year(_, _, _) | InodeType::Month(_, _, _, _) | InodeType::Day(_, _, _, _, _) => {
                FileAttr {
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
                }
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
            None => { reply.error(ENOENT); return; }
        };

        // This is inefficient: we reconstruct children to find the matching one.
        // In a real optimized FS we would cache children.
        // Here we just "simulate" finding it.
        
        let parent_kind = match self.get_inode(parent) {
            Some(k) => k.clone(),
            None => { reply.error(ENOENT); return; }
        };

        let client_arc = self.client.clone();
        let client = client_arc.lock().unwrap();

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
            },
            InodeType::Database(db) => {
                // Check collections
                if let Ok(colls) = client.list_collections(&db) {
                    if colls.contains(&name_str.to_string()) {
                         drop(client);
                         let ino = self.allocate_inode(InodeType::Collection(db, name_str.to_string()));
                         reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                         return;
                    }
                }
            },
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
                                      let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
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
            },
            InodeType::Year(db, coll, year) => {
                if let Ok(month) = name_str.parse::<u32>() {
                    // Check month
                    if let Ok(blobs) = client.list_blobs(&db, &coll) {
                         let has_month = blobs.iter().any(|b| {
                             if let Ok(uuid) = Uuid::parse_str(&b._key) {
                                  if let Some(ts) = uuid.get_timestamp() {
                                      let (secs, _) = ts.to_unix();
                                      let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
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
            },
            InodeType::Month(db, coll, year, month) => {
                if let Ok(day) = name_str.parse::<u32>() {
                    // Check day
                     if let Ok(blobs) = client.list_blobs(&db, &coll) {
                         let has_day = blobs.iter().any(|b| {
                             if let Ok(uuid) = Uuid::parse_str(&b._key) {
                                  if let Some(ts) = uuid.get_timestamp() {
                                      let (secs, _) = ts.to_unix();
                                      let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                      return dt.year() == year && dt.month() == month && dt.day() == day;
                                  }
                             }
                             false
                         });
                         if has_day {
                             drop(client);
                             let ino = self.allocate_inode(InodeType::Day(db, coll, year, month, day));
                             reply.entry(&TTL, &self.get_file_attr(ino).unwrap(), 0);
                             return;
                         }
                     }
                }
            },
             InodeType::Day(db, coll, year, month, day) => {
                // Find file
                if let Ok(blobs) = client.list_blobs(&db, &coll) {
                    if let Some(blob) = blobs.into_iter().find(|b| {
                         let target_name = b.filename.as_deref().unwrap_or(&b._key);
                         if target_name != name_str { return false; }
                         
                         if let Ok(uuid) = Uuid::parse_str(&b._key) {
                              if let Some(ts) = uuid.get_timestamp() {
                                  let (secs, _) = ts.to_unix();
                                  let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
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
            },
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
            None => { reply.error(ENOENT); return; }
        };

        if offset == 0 {
            if reply.add(ino, 0, FileType::Directory, ".") { return; }
            if reply.add(ino, 1, FileType::Directory, "..") { return; }
        }

        let client_arc = self.client.clone();
        let client = client_arc.lock().unwrap();
        let mut entries = Vec::new();

        // 0 and 1 are already sent.
        // We accumulate entries then skip based on offset (simplification)
        // Offset logic in FUSE is tricky. We'll use index + 2 as offset.

        match parent_kind {
             InodeType::Root => {
                match client.list_databases() {
                    Ok(dbs) => {
                        info!("Listing databases: {:?}", dbs);
                        for db in dbs {
                             let child_ino = self.allocate_inode(InodeType::Database(db.clone()));
                             entries.push((child_ino, FileType::Directory, db));
                        }
                    },
                    Err(e) => error!("Failed to list databases: {:?}", e),
                }
            },
            InodeType::Database(db) => {
                 match client.list_collections(&db) {
                    Ok(colls) => {
                        debug!("Listing collections for {}: {:?}", db, colls);
                        for coll in colls {
                             let child_ino = self.allocate_inode(InodeType::Collection(db.clone(), coll.clone()));
                             entries.push((child_ino, FileType::Directory, coll));
                        }
                    },
                    Err(e) => error!("Failed to list collections for {}: {:?}", db, e),
                }
            },
            InodeType::Collection(db, coll) => {
                 if let Ok(blobs) = client.list_blobs(&db, &coll) {
                     let mut years = HashSet::new();
                     for b in blobs {
                         if let Ok(uuid) = Uuid::parse_str(&b._key) {
                             if let Some(ts) = uuid.get_timestamp() {
                                  let (secs, _) = ts.to_unix();
                                  let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                  years.insert(dt.year());
                              }
                         }
                     }
                     let mut sorted_years: Vec<_> = years.into_iter().collect();
                     sorted_years.sort();
                     for year in sorted_years {
                         let child_ino = self.allocate_inode(InodeType::Year(db.clone(), coll.clone(), year));
                         entries.push((child_ino, FileType::Directory, year.to_string()));
                     }
                 }
            },
            InodeType::Year(db, coll, year) => {
                 if let Ok(blobs) = client.list_blobs(&db, &coll) {
                     let mut months = HashSet::new();
                     for b in blobs {
                         if let Ok(uuid) = Uuid::parse_str(&b._key) {
                             if let Some(ts) = uuid.get_timestamp() {
                                  let (secs, _) = ts.to_unix();
                                  let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                  if dt.year() == year {
                                      months.insert(dt.month());
                                  }
                              }
                         }
                     }
                     let mut sorted_months: Vec<_> = months.into_iter().collect();
                     sorted_months.sort();
                     for month in sorted_months {
                         let child_ino = self.allocate_inode(InodeType::Month(db.clone(), coll.clone(), year, month));
                         // Pad month 01, 02...
                         entries.push((child_ino, FileType::Directory, format!("{:02}", month)));
                     }
                 }
            },
             InodeType::Month(db, coll, year, month) => {
                 if let Ok(blobs) = client.list_blobs(&db, &coll) {
                     let mut days = HashSet::new();
                     for b in blobs {
                         if let Ok(uuid) = Uuid::parse_str(&b._key) {
                             if let Some(ts) = uuid.get_timestamp() {
                                  let (secs, _) = ts.to_unix();
                                  let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                  if dt.year() == year && dt.month() == month {
                                      days.insert(dt.day());
                                  }
                              }
                         }
                     }
                     let mut sorted_days: Vec<_> = days.into_iter().collect();
                     sorted_days.sort();
                     for day in sorted_days {
                         let child_ino = self.allocate_inode(InodeType::Day(db.clone(), coll.clone(), year, month, day));
                         entries.push((child_ino, FileType::Directory, format!("{:02}", day)));
                     }
                 }
            },
            InodeType::Day(db, coll, year, month, day) => {
                 if let Ok(blobs) = client.list_blobs(&db, &coll) {
                     for b in blobs {
                         if let Ok(uuid) = Uuid::parse_str(&b._key) {
                             if let Some(ts) = uuid.get_timestamp() {
                                  let (secs, _) = ts.to_unix();
                                  let dt = DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_secs(secs));
                                  if dt.year() == year && dt.month() == month && dt.day() == day {
                                      let name = b.filename.clone().unwrap_or_else(|| b._key.clone());
                                      let child_ino = self.allocate_inode(InodeType::Blob(db.clone(), coll.clone(), b));
                                      entries.push((child_ino, FileType::RegularFile, name));
                                  }
                              }
                         }
                     }
                 }
            },
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
            None => { reply.error(ENOENT); return; }
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
                    },
                    Err(_) => reply.error(EIO),
                }
            },
            _ => reply.error(EIO), // Should contain EISDIR equivalent
        }
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    
    let args = Args::parse();
    let mountpoint = args.mount;
    let mut options = vec![MountOption::RO];
    options.push(MountOption::FSName("solidb".to_string()));
    
    #[cfg(target_os = "macos")]
    options.push(MountOption::AutoUnmount);

    let client = SolidBClient::new(&args.host, args.port, &args.username, args.password.as_deref());
    let fs = SolidBFS::new(client);

    println!("Mounting SolidB at {}", mountpoint);
    
    if args.foreground {
        fuser::mount2(fs, &mountpoint, &options)?;
    } else {
        fuser::spawn_mount2(fs, &mountpoint, &options)?;
        loop { std::thread::sleep(Duration::from_secs(3600)); }
    }

    Ok(())
}
