
use solidb::storage::StorageEngine;
use std::sync::Arc;

fn main() {
    let storage = StorageEngine::new("data").unwrap();
    if let Ok(db) = storage.get_database("_system") { // Assuming _system or default db
         // Check both _system and potential user dbs if name is ambiguous
         println!("Checking database _system...");
         check_coll(&db, "bib_new");
    }
    
    // Also check other databases?
    for db_name in storage.list_databases() {
        if db_name == "_system" { continue; }
        println!("Checking database {}...", db_name);
        if let Ok(db) = storage.get_database(&db_name) {
             check_coll(&db, "bib_new");
        }
    }
}

fn check_coll(db: &solidb::storage::database::Database, name: &str) {
    match db.get_collection(name) {
        Ok(c) => println!("Collection {} exists. Type: {}", name, c.get_type()),
        Err(_) => println!("Collection {} does not exist in {}", name, db.name),
    }
}
