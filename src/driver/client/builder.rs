use super::SoliDBClient;
use crate::driver::protocol::DriverError;

/// Builder for creating a SoliDBClient with additional options
pub struct SoliDBClientBuilder {
    addr: String,
    auth: Option<(String, String, String)>, // (database, username, password)
    #[allow(dead_code)]
    timeout_ms: Option<u64>,
}

impl SoliDBClientBuilder {
    /// Create a new builder
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
            auth: None,
            timeout_ms: None,
        }
    }

    /// Set authentication credentials
    pub fn auth(mut self, database: &str, username: &str, password: &str) -> Self {
        self.auth = Some((
            database.to_string(),
            username.to_string(),
            password.to_string(),
        ));
        self
    }

    /// Set connection timeout in milliseconds
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Build the client
    pub async fn build(self) -> Result<SoliDBClient, DriverError> {
        let mut client = SoliDBClient::connect(&self.addr).await?;

        if let Some((database, username, password)) = self.auth {
            client.auth(&database, &username, &password).await?;
        }

        Ok(client)
    }
}
