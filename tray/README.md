# solidb-tray and solidb-service

Two small Windows executables that run a SoliDB server on the machine. The
server stays the console program it is; neither links it.

- `solidb-tray.exe`: an icon in the notification area that starts, stops
  and watches the server.
- `solidb-service.exe`: runs the server as a Windows service, started with
  the machine.

## The icon

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

When the Windows service is installed, the icon drives the service instead
of starting a server of its own, and reads the service's settings. Without
it, the server runs in the user's session and ends with it.

### Settings

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

## The service

```
solidb-service install     (as administrator) register it, started with the machine
solidb-service start
solidb-service stop        the server closes its database
solidb-service status
solidb-service uninstall   (as administrator) data and settings are kept
```

- Runs under the local system account; keep the executables on a local disk.
- Settings: `%ProgramData%\SoliDB\service.conf` (`exe`, `data_dir`, `port`,
  `log`, `args`), written by `install`. Restart the service to apply a change.
- The server is started in `%ProgramData%\SoliDB` and reads a `.env` there
  (`SOLIDB_ADMIN_PASSWORD`, `JWT_SECRET`).
- Stop, and a machine shutdown, send Ctrl-C to the server and give it 60
  seconds. A server that ends on its own is started again after 5 seconds,
  twice a day at most.
- A user signed in on the machine may start, stop and query the service
  without elevation (the rights are set by `install`), which is what lets
  the icon drive it.

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
