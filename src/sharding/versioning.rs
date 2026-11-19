//! Document versioning of a sharded collection (card #1002): its history
//! lives in its physical shards (`<name>_s<n>`), which follow the logical
//! collection's flag on every node. The flag itself is replicated on the
//! logical collection (`Operation::SetVersioning`, card #998); each node
//! applies it to the shards it holds, when it changes and when a shard is
//! created there.

use crate::storage::Database;

/// The logical collection of a physical shard name: `orders` for
/// `orders_s3`.
pub fn logical_of(physical: &str) -> Option<&str> {
    let (logical, n) = physical.rsplit_once("_s")?;
    (!logical.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .then_some(logical)
}

/// Give the shards of `logical` this node holds the versioning of
/// `logical`. Nothing when it is not sharded.
pub fn sync_shard_versioning(db: &Database, logical: &str) {
    let Ok(collection) = db.get_collection(logical) else {
        return;
    };
    let Some(config) = collection.get_shard_config().filter(|c| c.num_shards > 0) else {
        return;
    };
    let on = collection.is_versioned();
    for shard in 0..config.num_shards {
        let Ok(physical) = db.get_collection(&format!("{logical}_s{shard}")) else {
            continue;
        };
        let applied = match (on, physical.is_versioned()) {
            (true, false) => physical.enable_versioning(),
            (false, true) => physical.disable_versioning(),
            _ => Ok(()),
        };
        if let Err(e) = applied {
            tracing::warn!("versioning of shard {logical}_s{shard} not set: {e}");
        }
    }
}

/// After the shard `physical` was created on this node: the versioning of
/// its logical collection.
pub fn follow_logical(db: &Database, physical: &str) {
    if let Some(logical) = logical_of(physical) {
        sync_shard_versioning(db, logical);
    }
}

#[cfg(test)]
mod tests {
    use super::logical_of;

    #[test]
    fn a_shard_name_gives_its_collection() {
        assert_eq!(logical_of("orders_s3"), Some("orders"));
        assert_eq!(logical_of("my_stuff_s12"), Some("my_stuff"));
        assert_eq!(logical_of("orders"), None);
        assert_eq!(logical_of("orders_sx"), None);
        assert_eq!(logical_of("_s1"), None);
    }
}
