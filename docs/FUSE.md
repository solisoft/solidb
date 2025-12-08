# SoliDB FUSE Filesystem Driver

Access your SoliDB blob collections directly from your operating system's file manager (Finder, Explorer) using `solidb-fuse`.

## Prerequisites

### macOS
1. Install [macFUSE](https://osxfuse.github.io/):
   ```bash
   brew install macfuse
   ```

### Linux (Ubuntu/Debian)
1. Install `libfuse3-dev` and `pkg-config`:
   ```bash
   sudo apt-get update && sudo apt-get install libfuse3-dev fuse3 pkg-config
   ```

## Build

```bash
cargo build --bin solidb-fuse --features fuse
```

## Usage

Run the `solidb-fuse` tool to mount a SolidB instance to a local directory.

```bash
# Create a mount point
mkdir -p /tmp/mnt

# Mount the filesystem
./target/debug/solidb-fuse \
  --host localhost \
  --port 6745 \
  --username admin \
  --password admin \
  --mount /tmp/mnt \
  --foreground
```

### Running as a Daemon (Unix only)

You can run `solidb-fuse` as a background daemon:

```bash
./target/debug/solidb-fuse \
  --mount /tmp/mnt \
  --daemon \
  --pid-file /tmp/solidb-fuse.pid \
  --log-file /tmp/solidb-fuse.log
```

To stop the daemon, simply run the command again (it detects the PID file and kills the old process), or manually kill it:

```bash
kill $(cat /tmp/solidb-fuse.pid)
```

### Unmounting

```bash
umount /tmp/mnt
# or if running in foreground, press Ctrl+C
```

## Folder Structure

The filesystem exposes the following hierarchy:

```
/mount_point/
├── <database_name>/
│   ├── <blob_collection_name>/
│   │   ├── <YYYY>/
│   │   │   ├── <MM>/
│   │   │   │   ├── <DD>/
│   │   │   │   │   ├── filename.ext
│   │   │   │   │   └── ...
```

Files are automatically organized by date based on their UUIDv7 creation timestamp.
