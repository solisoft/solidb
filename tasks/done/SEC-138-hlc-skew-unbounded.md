# SEC-138: HLC accepts unbounded clock skew from peers

## Status
- **Severity**: HIGH
- **Category**: Cluster Integrity / DoS
- **Project**: soli/db
- **File**: `src/cluster/hlc.rs`
- **Lines**: 61-90, 195-228 (`HlcGenerator::receive`)

## Description
`HlcGenerator::receive` adopts any remote `physical_time` larger than the local clock. A single hostile or buggy peer sending `physical_time = u64::MAX - 1` permanently wedges every node's clock, breaking ordering on all future writes.

## Exploit Scenario
A compromised peer (or an attacker who exploits SEC-137 to forge a node identity) sends a message with `physical_time = u64::MAX - 1`. All receiving nodes update their clocks. Future writes appear to be from the year 5×10^8 — replication ordering is permanently broken cluster-wide.

## Recommendation
- Reject (or clamp) remote HLC physical times exceeding `now_ms + MAX_SKEW` (e.g. 60 s).
- Log peers that send out-of-bounds timestamps and circuit-break the connection.

## References
- Related: SEC-080, SEC-110, SEC-136.
