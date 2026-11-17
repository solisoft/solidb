# solidb-tray

An icon in the Windows notification area that starts, stops and watches a
SoliDB server on the machine. A separate, small executable: the server stays
the console program it is.

- **Start / Stop / Restart.** `solidb.exe` is started without a window, its
  output appended to a log file. Stop asks it to stop as Ctrl-C does, so it
  closes its database; it is ended only if it has not gone after 60 seconds.
- **State by colour.** Grey: stopped. Amber: starting, stopping, or not
  answering. Green: `GET /_api/health` answers. Red: the server ended without
  having been asked to (a notification says why).
- **Quit leaves the server running.** The icon takes it back when it is
  started again (its pid is kept in `solidb.pid`, and believed only if that
  process is a `solidb.exe`).
- **Start with Windows** adds the icon to the session's startup programs.

The server runs in the user's session and ends with it. Running SoliDB as a
Windows service is a separate piece of work.

## Settings

`%LOCALAPPDATA%\SoliDB\tray.conf`, written with the defaults the first time
(menu: *Settings*). Quit the icon and start it again to apply a change.

| Key | Default | |
|---|---|---|
| `exe` | `solidb.exe` beside `solidb-tray.exe` | The server to start. |
| `data_dir` | `%LOCALAPPDATA%\SoliDB\data` | |
| `port` | `6745` | |
| `log` | `%LOCALAPPDATA%\SoliDB\solidb.log` | The server's output. A new instance prints its admin password there. |
| `args` | | Added to the server's command line, e.g. `--host 127.0.0.1`. |
| `console_url` | | Adds *Open admin console* to the menu. |
| `start_server` | `true` | Start the server when the icon starts. |

A path may name an environment variable (`%LOCALAPPDATA%\SoliDB\data`).

A `tray.conf` beside `solidb-tray.exe` makes that folder the icon's own:
settings, log and pid file live there, for a copy that keeps to itself.

## Building

The crate is outside the workspace and depends on `windows-sys` alone.

```bash
# On Windows
cargo build --release --manifest-path tray/Cargo.toml

# From Linux (needs the mingw-w64 linker)
rustup target add x86_64-pc-windows-gnu
cargo build --release --manifest-path tray/Cargo.toml --target x86_64-pc-windows-gnu

# Settings, command line, states and the health probe are tested anywhere
cargo test --manifest-path tray/Cargo.toml
```
