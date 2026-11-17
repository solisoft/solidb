//! The admin console: the `admin` application of the installation, run on
//! the Soli runtime and reached on the loopback. What is here is neither
//! Win32 nor the network, and is tested on any system.
//!
//! The runtime is not part of the installation. It is fetched at the first
//! use, after asking: one release, named here, whose archive must have the
//! digest written here before anything of it is run.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::Path;

use crate::config::{quoted, Config};

/// The release of Soli the console was tested on.
pub const SOLI_VERSION: &str = "2.37.0";
pub const SOLI_ARCHIVE: &str =
    "https://github.com/solisoft/soli_lang/releases/download/v2.37.0/soli-windows-amd64.tar.gz";
/// SHA-256 of that archive, as its release publishes it.
pub const SOLI_SHA256: &str = "e432dc3cc6a37cbd0114108b873faef572720c75e749d7db1668bf79387af83f";
/// Its size, for the question asked before the download.
pub const SOLI_MEGABYTES: u32 = 28;

/// Beside `soli.exe`: the version this icon downloaded there.
const MARK: &str = "soli.version";

/// Whether the runtime has to be fetched: there is none, or the one there
/// was downloaded by an icon that named another release. A `soli.exe` with
/// no mark was put there by hand and is left alone.
pub fn needs_download(soli: &Path) -> bool {
    if !soli.is_file() {
        return true;
    }
    match std::fs::read_to_string(soli.with_file_name(MARK)) {
        Ok(version) => version.trim() != SOLI_VERSION,
        Err(_) => false,
    }
}

/// Notes that the runtime at `soli` is the release named here.
pub fn mark_downloaded(soli: &Path) -> io::Result<()> {
    std::fs::write(soli.with_file_name(MARK), SOLI_VERSION)
}

/// Where the browser is sent.
pub fn url(cfg: &Config) -> String {
    format!("http://127.0.0.1:{}/", cfg.console_port())
}

/// Fetches the release's archive into `archive` with the `curl.exe` of
/// Windows (`system` is its System32): it fails on anything but the file.
pub fn download_line(system: &Path, archive: &Path) -> String {
    format!(
        "{} --location --fail --silent --show-error --max-time 900 --output {} {}",
        quoted(&system.join("curl.exe").to_string_lossy()),
        quoted(&archive.to_string_lossy()),
        SOLI_ARCHIVE
    )
}

/// Unpacks `archive` into `dir` with the `tar.exe` of Windows.
pub fn unpack_line(system: &Path, archive: &Path, dir: &Path) -> String {
    format!(
        "{} -xf {} -C {}",
        quoted(&system.join("tar.exe").to_string_lossy()),
        quoted(&archive.to_string_lossy()),
        quoted(&dir.to_string_lossy())
    )
}

/// The console's command line: the application in `app`, on its port.
pub fn command_line(cfg: &Config, app: &Path) -> String {
    format!(
        "{} serve {} --port {}",
        quoted(&cfg.soli.to_string_lossy()),
        quoted(&app.to_string_lossy()),
        cfg.console_port()
    )
}

/// What the console is started with, over the icon's own environment.
///
/// It listens on the loopback only and holds no credential: with no
/// `SOLIDB_USERNAME` the application asks each browser session to sign in
/// to the server, so it is that sign-in which protects the database, and
/// `ADMIN_UI_ALLOW_NO_AUTH` only says that no second password is asked.
/// A `SOLIDB_USERNAME` given here would hand the server to every visitor.
pub fn settings(cfg: &Config) -> Vec<(String, String)> {
    [
        ("SOLI_HOST", "127.0.0.1".to_string()),
        ("SOLIDB_HOST", format!("http://127.0.0.1:{}", cfg.port)),
        ("ADMIN_UI_ALLOW_NO_AUTH", "1".to_string()),
        // The application's requests go to the server on this machine,
        // which Soli's HTTP client refuses unless told otherwise.
        ("SOLI_DEV_ALLOW_SSRF", "1".to_string()),
        (
            "SOLI_HTTP_ALLOW_HOSTS",
            "localhost,127.0.0.1,[::1]".to_string(),
        ),
        ("NO_COLOR", "1".to_string()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

/// Variables of the icon's environment the console must not be handed: a
/// credential there would be used for every visitor (see `settings`).
const WITHHELD: [&str; 3] = ["SOLIDB_USERNAME", "SOLIDB_PASSWORD", "ADMIN_UI_PASSWORD"];

/// An environment block as `CreateProcessW` takes one: `inherited` without
/// the names that `set` gives or that are withheld, then `set`, each as
/// `NAME=value` closed by a zero, the block by one more. Names are compared
/// as Windows compares them, without case.
pub fn environment(
    inherited: impl Iterator<Item = (OsString, OsString)>,
    set: &[(String, String)],
) -> Vec<u16> {
    let replaced = |name: &OsStr| {
        let name = name.to_string_lossy();
        set.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name))
            || WITHHELD.iter().any(|n| n.eq_ignore_ascii_case(&name))
    };
    let mut block = Vec::new();
    let mut entry = |name: &OsStr, value: &OsStr| {
        block.extend(wide_units(name));
        block.push(b'=' as u16);
        block.extend(wide_units(value));
        block.push(0);
    };
    for (name, value) in inherited {
        if !name.is_empty() && !replaced(&name) {
            entry(&name, &value);
        }
    }
    for (name, value) in set {
        entry(OsStr::new(name), OsStr::new(value));
    }
    block.push(0);
    block
}

#[cfg(windows)]
fn wide_units(text: &OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().collect()
}

#[cfg(not(windows))]
fn wide_units(text: &OsStr) -> Vec<u16> {
    text.to_string_lossy().encode_utf16().collect()
}

/// The parts of the application that are its code. Whatever else is in the
/// copy (its sessions, a `.env` the user wrote) is the copy's own.
const PARTS: [&str; 3] = ["app", "config", "public"];
const MANIFEST: &str = "soli.toml";

/// Whether `app` holds an admin application.
pub fn is_app(app: &Path) -> bool {
    app.join("config").join("routes.sl").is_file()
}

/// Makes `copy` the application in `app`: its code is replaced, so that an
/// upgrade leaves no file of the version before, and the rest is kept.
pub fn refresh(app: &Path, copy: &Path) -> io::Result<()> {
    std::fs::create_dir_all(copy)?;
    for part in PARTS {
        let to = copy.join(part);
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        let from = app.join(part);
        if from.is_dir() {
            copy_tree(&from, &to)?;
        }
    }
    let manifest = app.join(MANIFEST);
    if manifest.is_file() {
        std::fs::copy(manifest, copy.join(MANIFEST))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Whether the file at `path` has the SHA-256 `expected` (hexadecimal).
pub fn has_digest(path: &Path, expected: &str) -> io::Result<bool> {
    let digest = sha256(std::fs::File::open(path)?)?;
    Ok(hex(&digest).eq_ignore_ascii_case(expected.trim()))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (FIPS 180-4) of everything `input` gives. Written here so that
/// the crate stays without a dependency; held to the standard's vectors.
pub fn sha256(mut input: impl Read) -> io::Result<[u8; 32]> {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut block = [0u8; 64];
    let mut filled = 0usize;
    let mut length = 0u64;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = match input.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        length += read as u64;
        let mut rest = &chunk[..read];
        while !rest.is_empty() {
            let take = (64 - filled).min(rest.len());
            block[filled..filled + take].copy_from_slice(&rest[..take]);
            filled += take;
            rest = &rest[take..];
            if filled == 64 {
                compress(&mut state, &block);
                filled = 0;
            }
        }
    }
    // Padding: a one bit, zeros, then the length in bits on 8 bytes.
    block[filled] = 0x80;
    filled += 1;
    if filled > 56 {
        block[filled..].fill(0);
        compress(&mut state, &block);
        filled = 0;
    }
    block[filled..56].fill(0);
    block[56..].copy_from_slice(&(length.wrapping_mul(8)).to_be_bytes());
    compress(&mut state, &block);

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    Ok(out)
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in w.iter_mut().take(16).enumerate() {
        let at = i * 4;
        *word = u32::from_be_bytes([block[at], block[at + 1], block[at + 2], block[at + 3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(majority);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (word, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *word = word.wrapping_add(add);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn digest(bytes: &[u8]) -> String {
        hex(&sha256(bytes).unwrap())
    }

    #[test]
    fn sha256_is_the_standards() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // A million 'a': many blocks, and a length that needs the carry.
        assert_eq!(
            digest(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn padding_holds_at_every_length_around_a_block() {
        // 55, 56 and 64 bytes are where the padding changes shape.
        assert_eq!(
            digest(&[b'a'; 55]),
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318"
        );
        assert_eq!(
            digest(&[b'a'; 56]),
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
        );
        assert_eq!(
            digest(&[b'a'; 64]),
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb"
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("solidb-tray-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_held_to_its_digest() {
        let dir = scratch("digest");
        let file = dir.join("a");
        std::fs::write(&file, b"abc").unwrap();
        let abc = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        assert!(has_digest(&file, abc).unwrap());
        std::fs::write(&file, b"abd").unwrap();
        assert!(!has_digest(&file, abc).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_runtime_is_fetched_when_missing_or_of_another_release() {
        let dir = scratch("mark");
        let soli = dir.join("soli.exe");
        assert!(needs_download(&soli));
        // Put there by hand: used as it is.
        std::fs::write(&soli, b"x").unwrap();
        assert!(!needs_download(&soli));
        std::fs::write(dir.join(MARK), "0.0.1\n").unwrap();
        assert!(needs_download(&soli));
        mark_downloaded(&soli).unwrap();
        assert!(!needs_download(&soli));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refresh_replaces_the_code_and_keeps_the_rest() {
        let dir = scratch("refresh");
        let (app, copy) = (dir.join("admin"), dir.join("console"));
        std::fs::create_dir_all(app.join("config")).unwrap();
        std::fs::create_dir_all(app.join("app").join("views")).unwrap();
        std::fs::write(app.join("config").join("routes.sl"), "new").unwrap();
        std::fs::write(app.join("app").join("views").join("a.slv"), "a").unwrap();
        std::fs::write(app.join(MANIFEST), "[package]").unwrap();
        assert!(is_app(&app));
        assert!(!is_app(&copy));

        // A copy of the version before, with a file that is gone since.
        std::fs::create_dir_all(copy.join("app")).unwrap();
        std::fs::create_dir_all(copy.join("sessions")).unwrap();
        std::fs::write(copy.join("app").join("gone.sl"), "old").unwrap();
        std::fs::write(copy.join("sessions").join("s"), "kept").unwrap();
        std::fs::write(copy.join(".env"), "kept").unwrap();

        refresh(&app, &copy).unwrap();
        assert!(is_app(&copy));
        assert!(copy.join("app").join("views").join("a.slv").is_file());
        assert!(!copy.join("app").join("gone.sl").exists());
        assert!(copy.join("sessions").join("s").is_file());
        assert!(copy.join(".env").is_file());
        assert!(copy.join(MANIFEST).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn defaults() -> Config {
        Config::defaults(Path::new("home"), Path::new("beside"))
    }

    #[test]
    fn the_console_listens_on_the_loopback_and_holds_no_credential() {
        let set = settings(&defaults());
        let value = |name: &str| set.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        assert_eq!(value("SOLI_HOST"), Some("127.0.0.1"));
        assert_eq!(value("SOLIDB_HOST"), Some("http://127.0.0.1:6745"));
        assert_eq!(value("SOLIDB_USERNAME"), None);
        assert_eq!(value("SOLIDB_PASSWORD"), None);
        assert_eq!(url(&defaults()), "http://127.0.0.1:6746/");
    }

    fn entries(block: &[u16]) -> Vec<String> {
        assert_eq!(&block[block.len() - 2..], [0, 0], "closed by two zeros");
        block[..block.len() - 2]
            .split(|unit| *unit == 0)
            .map(String::from_utf16_lossy)
            .collect()
    }

    #[test]
    fn the_environment_is_the_icons_with_the_settings_over_it() {
        let inherited = [
            ("Path", "C:\\Windows"),
            ("solidb_host", "0.0.0.0"),
            ("SOLIDB_PASSWORD", "secret"),
            ("Admin_UI_Password", "secret"),
        ]
        .into_iter()
        .map(|(n, v)| (OsString::from(n), OsString::from(v)));
        let set = vec![(
            "SOLIDB_HOST".to_string(),
            "http://127.0.0.1:6745".to_string(),
        )];
        assert_eq!(
            entries(&environment(inherited, &set)),
            ["Path=C:\\Windows", "SOLIDB_HOST=http://127.0.0.1:6745"]
        );
    }

    #[test]
    fn the_command_line_quotes_what_holds_a_space() {
        let mut cfg = defaults();
        cfg.soli = PathBuf::from("C:\\Users\\me\\SoliDB\\soli\\soli.exe");
        assert_eq!(
            command_line(&cfg, Path::new("C:\\Users\\my name\\SoliDB\\console")),
            "C:\\Users\\me\\SoliDB\\soli\\soli.exe serve \"C:\\Users\\my name\\SoliDB\\console\" --port 6746"
        );
    }
}
