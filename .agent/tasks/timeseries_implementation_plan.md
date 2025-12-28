# Implementation Plan - Time Series Capabilities

We will implement native Time Series capabilities in SoliDB to support efficient storage, querying, and retention of time-based data.

## 1. Storage Layer (`src/storage/collection.rs`)

We will enhance the `Collection` struct to support a new `timeseries` type.

- **Collection Type**: Allow setting `collection_type` to `"timeseries"`.
- **Validation**: When `collection_type` is `"timeseries"`, ensure documents are immutable (no updates allowed, only inserts and deletes).
- **Key Generation**: Ensure keys are strictly time-ordered (UUIDv7 is already the default, which is perfect).
- **Efficient Retention**: Implement `delete_range_by_time` to efficiently prune old data using RocksDB's `delete_range_cf`.
    - Function: `prune_older_than(timestamp_ms: u64)`
    - This will construct a start key (min) and an end key (timestamp derived from UUIDv7) and issue a range delete.

## 2. SDBQL Layer (`src/sdbql/executor.rs`)

We will add time-series specific aggregation functions.

- **New Function**: `TIME_BUCKET(timestamp, interval)`
    - **Description**: Buckets a timestamp into fixed-width intervals.
    - **Arguments**:
        - `timestamp`: The timestamp value (either ISO8601 string or numeric timestamp).
        - `interval`: string like `'5m'`, `'1h'`, `'1d'`.
    - **Returns**: The start time of the bucket.
    - **Usage**: Used in `GROUP BY` clauses for downsampling.

## 3. Server/API Layer (`src/server/routes.rs`)

- **Prune Endpoint**: Expose the pruning functionality via API.
    - `POST /api/v1/collection/{name}/prune`
    - Body: `{ "older_than": "2024-01-01T00:00:00Z" }`

## 4. Documentation

- Update `architecture.etlua` to mention Time Series capabilities.
- Document `TIME_BUCKET` function.

---

## Execution Steps

1.  **Modify `src/storage/collection.rs`**: Add `prune_older_than` and `timeseries` type validation.
2.  **Modify `src/sdbql/executor.rs`**: Implement `TIME_BUCKET` function.
3.  **Modify `src/server/routes.rs`**: Add route for pruning.
