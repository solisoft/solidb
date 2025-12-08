# SoliDB FUSE Filesystem Driver

Access your SoliDB blob collections directly from your operating system's file manager (Finder, Explorer) using `solidb-fuse`.

## Prerequisites

### macOS
1. Install [macFUSE](https://osxfuse.github.io/):
   ```bash
   brew install macfuse
   ```

## Usage

Run the `solidb-fuse` tool to mount a SolidB instance to a local directory.

```bash
# Create a mount point
mkdir -p /tmp/solidb_mount

# Mount the filesystem
./target/debug/solidb-fuse \
  --host localhost \
  --port 6755 \
  --username admin \
  --password admin \
  --mount /tmp/solidb_mount \
  --foreground
```

To unmount:
```bash
umount /tmp/solidb_mount
# or press Ctrl+C if running in foreground
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
