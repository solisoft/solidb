use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{
    derive_bloom_filter_key_count, get_bloom_filter_target_fp_rate, tokenize, CuckooFilter,
    Index, IndexStats, IndexType,
};
use fastbloom::BloomFilter;
use rocksdb::{Direction, IteratorMode, WriteBatch};
// use std::collections::HashSet; 
use hex;
use std::hash::DefaultHasher;

impl Collection {
    // ... (existing code up to get_index_for_field) ...

    // Note: Inserting new methods after index_lookup_eq_limit or nearby

    /// Lookup documents where field > value
    pub fn index_lookup_gt(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, true)
    }

    /// Lookup documents where field >= value
    pub fn index_lookup_gte(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, true)
    }

    /// Lookup documents where field < value
    pub fn index_lookup_lt(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, false, false)
    }

    /// Lookup documents where field <= value
    pub fn index_lookup_lte(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        self.index_range_scan(field, value, true, false)
    }

    /// Generic range scan helper
    fn index_range_scan(
        &self,
        field: &str,
        value: &Value,
        inclusive: bool,
        forward: bool,
    ) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;
        let index_name = &index.name;

        // Encode value to preserve order
        let value_key = crate::storage::codec::encode_key(value);
        let value_str = hex::encode(value_key);
        
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix_base = format!("{}{}:", IDX_PREFIX, index_name);
        
        // Target key to seek to
        let target_key = format!("{}{}:", prefix_base, value_str);

        let mut doc_keys = Vec::new();

        if forward {
             // scan > or >= value
             let mode = IteratorMode::From(target_key.as_bytes(), Direction::Forward);
             let mut iter = db.iterator_cf(cf, mode);
             
             // Check first element for equality handling
             if let Some(Ok((k, v))) = iter.next() {
                 if k.starts_with(prefix_base.as_bytes()) {
                     let key_str = String::from_utf8_lossy(&k);
                     // Check if exact match to target_key (meaning value equality)
                     // Note: target_key includes the colon separator "idx:name:val:"? 
                     // Wait, index format is "idx:name:val:doc_key"? or "idx:name:val"?
                     // Looking at create_index (not shown but inferred), it usually is "idx:name:val:doc_key" -> ""
                     // or "idx:name:val" -> "doc_key" (if unique)
                     // or "idx:name:val:doc_key" -> empty (if non-unique)
                     
                     // In index_lookup_eq: prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
                     // So key is "idx:name:val:..."
                     
                     let is_exact = k.starts_with(target_key.as_bytes());
                     
                     if is_exact {
                         if inclusive {
                             let val_str = String::from_utf8_lossy(&v);
                             doc_keys.push(Self::doc_key(&val_str));
                         }
                         // If not inclusive (gt), we skip this one AND all others with same prefix (same val)
                         // But simple next() only skips one. We need to skip ALL equal values.
                     } else {
                         // Greater than target
                         let val_str = String::from_utf8_lossy(&v);
                         doc_keys.push(Self::doc_key(&val_str));
                     }
                 }
             }

             // Continue iteration
             // This logic is tricky for "skip all equal".
             // Simpler: iterate all from seek, filter based on prefix constraint
             // But we want everything > val.
             // "Everything" means until end of index prefix "idx:name:"?
             
             // Let's refine:
             // We need to iterate all keys starting with "idx:name:" that are > (or >=) target_key
         } else {
             // scan < or <= value
             // Seek to target_key, then reverse?
             let mode = IteratorMode::From(target_key.as_bytes(), Direction::Reverse);
             // ...
         }
         
         // Simplified approach using string comparison on keys if manageable, or simple loop
         // Since we don't have the full iteration logic ready and to avoid bugs, 
         // let's try to reuse index_sorted but with a filter?
         // index_sorted iterates ALL. That's slow if we want a range.
         
         // Let's rewrite reusing IteratorMode efficiently.
         
        Some(Vec::new()) // Placeholder to fix compilation first, will implement body in next step with multi_replace
    }
    
    // ... 
}
