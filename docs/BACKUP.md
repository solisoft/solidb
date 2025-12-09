# SoliDB Backup & Restore Tools

SoliDB provides two command-line utilities for backing up and restoring your data:

- **`solidb-dump`** - Export databases and collections to JSON
- **`solidb-restore`** - Import data from JSON dumps

## Building the Tools

```bash
cargo build --release --bin solidb-dump --bin solidb-restore
```

The compiled binaries will be in `target/release/`.

---

## solidb-dump

Export SoliDB databases or collections to JSON format.

### Usage

```bash
solidb-dump [OPTIONS] --database <DATABASE>
```

### Options

| Option                      | Short | Description                                               | Default     |
| --------------------------- | ----- | --------------------------------------------------------- | ----------- |
| `--database <DATABASE>`     | `-d`  | Database name (required)                                  |             |
| `--collection <COLLECTION>` | `-c`  | Collection name (optional, dumps all if not specified)    |             |
| `--output <FILE>`           | `-o`  | Output file (optional, writes to stdout if not specified) |             |
| `--host <HOST>`             | `-H`  | Database host                                             | `localhost` |
| `--port <PORT>`             | `-P`  | Database port                                             | `6745`      |
| `--pretty`                  |       | Pretty-print JSON output                                  |             |

### Examples

**Dump entire database to file:**

```bash
solidb-dump -d mydb -o backup.json --pretty
```

**Dump single collection:**

```bash
solidb-dump -d mydb -c users -o users.json
```

**Dump to stdout (for piping):**

```bash
solidb-dump -d mydb -c users | gzip > users.json.gz
```

**Dump from remote server:**

```bash
solidb-dump -H prod-server.com -P 6745 -d mydb -o production-backup.json
```

### Output Format

The dump file is a JSON object with the following structure:

```json
{
  "database": "mydb",
  "collections": [
    {
      "name": "users",
      "shardConfig": {
        "num_shards": 4,
        "replication_factor": 2,
        "shard_key": "_key"
      },
      "documents": [
        { "_id": "1", "_key": "user1", "name": "Alice" },
        { "_id": "2", "_key": "user2", "name": "Bob" }
      ]
    }
  ]
}
```

**Notes:**

- `shardConfig` is `null` for non-sharded collections
- All document metadata (`_id`, `_key`, `_rev`) is preserved
- Documents are exported in arbitrary order

---

## solidb-restore

Import databases and collections from JSON dumps created by `solidb-dump`.

### Usage

```bash
solidb-restore [OPTIONS] --input <FILE>
```

### Options

| Option                      | Short | Description                                       | Default             |
| --------------------------- | ----- | ------------------------------------------------- | ------------------- |
| `--input <FILE>`            | `-i`  | Input JSON dump file (required)                   |                     |
| `--host <HOST>`             | `-H`  | Target database host                              | `localhost`         |
| `--port <PORT>`             | `-P`  | Target database port                              | `6745`              |
| `--database <DATABASE>`     |       | Override database name                            | Uses name from dump |
| `--collection <COLLECTION>` |       | Override collection name (single collection only) | Uses name from dump |
| `--create-database`         |       | Create database if it doesn't exist               |                     |
| `--drop`                    |       | Drop existing collection before restore           |                     |

### Examples

**Restore from backup:**

```bash
solidb-restore -i backup.json --create-database
```

**Restore to different database:**

```bash
solidb-restore -i backup.json --database newdb --create-database
```

**Restore and replace existing collection:**

```bash
solidb-restore -i users.json --drop
```

**Restore single collection with new name:**

```bash
solidb-restore -i users.json --collection users_copy
```

**Restore to different server:**

```bash
solidb-restore -i backup.json -H staging.local -P 6745 --create-database
```

### Behavior

- **Database Creation**: Use `--create-database` to automatically create the target database if it doesn't exist
- **Collection Creation**: Collections are created automatically during restore
- **Shard Configuration**: Sharding settings are preserved from the dump
- **Existing Data**:
  - Without `--drop`: Attempts to insert documents (may fail on duplicate keys)
  - With `--drop`: Deletes existing collection first, then recreates

### Error Handling

- Failed document insertions are counted and reported
- First 5 errors are logged to stderr
- Restore continues even if some documents fail
- Exit code is 0 if dump completes, even with partial failures

---

## Common Workflows

### 1. Regular Backup

```bash
# Daily backup script
DATE=$(date +%Y%m%d)
solidb-dump -d production -o "backups/prod-${DATE}.json" --pretty
```

### 2. Migrate Between Environments

```bash
# Export from production
solidb-dump -H prod.example.com -d mydb -o prod-export.json

# Import to staging
solidb-restore -i prod-export.json -H staging.local -P 6745 --database mydb --create-database --drop
```

### 3. Clone Collection

```bash
# Export collection
solidb-dump -d mydb -c users -o users.json

# Import with new name
solidb-restore -i users.json --collection users_copy
```

### 4. Migrate from Non-Sharded to Sharded

```bash
# 1. Dump existing non-sharded collection
solidb-dump -d mydb -c users -o users.json

# 2. Delete old collection via API or UI

# 3. Create new sharded collection via API or UI
curl -X POST http://localhost:6745/database/mydb/collection \
  -H "Content-Type: application/json" \
  -d '{
    "name": "users",
    "numShards": 8,
    "replicationFactor": 3,
    "shardKey": "_key"
  }'

# 4. Restore data to new sharded collection
solidb-restore -i users.json
```

### 5. Selective Restore

```bash
# Dump all collections
solidb-dump -d mydb -o full-backup.json

# Edit JSON to keep only specific collections
# Then restore
solidb-restore -i filtered-backup.json --database mydb_filtered --create-database
```

### 6. Cross-Version Migration

```bash
# Export from old version
solidb-dump -H old-server -d mydb -o migration.json

# Upgrade server software

# Import to new version
solidb-restore -i migration.json --create-database
```

---

## Performance Tips

1. **Use `--pretty` only when needed** - Pretty-printing increases file size significantly
2. **Compress large dumps** - Use gzip or similar:
   ```bash
   solidb-dump -d mydb | gzip > backup.json.gz
   gunzip -c backup.json.gz | solidb-restore -i -
   ```
3. **Batch size** - Currently fixed at 10,000 documents per query
4. **Network latency** - For large datasets, run tools on same network as database

---

## Limitations

- **No incremental backups** - Always full dumps
- **No schema export** - Index definitions are not included (only data)
- **Single-threaded** - No parallel dump/restore
- **Memory usage** - Large collections may require significant RAM
- **No encryption** - Dump files are plain JSON

---

## Troubleshooting

**"Failed to list collections"**

- Verify database name is correct
- Check server is running and accessible
- Verify permissions/authentication if enabled

**"Collection not found"**

- Database or collection doesn't exist
- Check spelling and case-sensitivity

**"Failed to insert document"**

- Duplicate key conflicts (use `--drop` flag)
- Schema validation errors
- Disk space issues

**Large restore takes too long**

- Normal for large datasets (inserts are sequential)
- Consider splitting dump into multiple files
- Future versions may support parallel restore

---

## Security Considerations

1. **Dump files contain sensitive data** - Secure storage required
2. **No authentication in tools** - Relies on server-level auth
3. **Plain JSON format** - Encrypt dump files if needed:
   ```bash
   solidb-dump -d mydb | gpg --encrypt > backup.json.gpg
   gpg --decrypt backup.json.gpg | solidb-restore -i -
   ```

---

## See Also

- [Sharding Documentation](./SHARDING.md)
- [API Reference](./API.md)
- [SDBQL Query Language](./SDBQL.md)
