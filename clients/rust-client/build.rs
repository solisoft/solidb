fn main() {
    #[cfg(feature = "mobile")]
    {
        // Generate UniFFI bindings when mobile feature is enabled
        uniffi::generate_scaffolding("src/solidb_client.udl").unwrap();
        println!("cargo:rerun-if-changed=src/solidb_client.udl");
    }
}
