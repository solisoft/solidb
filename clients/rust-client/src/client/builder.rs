use super::DriverError;
use super::SoliDBClient;

pub struct SoliDBClientBuilder {
    addr: String,
    auth: Option<AuthMethod>,
    timeout_ms: Option<u64>,
    pool_size: Option<usize>,
}

pub enum AuthMethod {
    UsernamePassword {
        database: String,
        username: String,
        password: String,
    },
    ApiKey {
        database: String,
        api_key: String,
    },
}

impl SoliDBClientBuilder {
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
            auth: None,
            timeout_ms: None,
            pool_size: None,
        }
    }

    pub fn auth(mut self, database: &str, username: &str, password: &str) -> Self {
        self.auth = Some(AuthMethod::UsernamePassword {
            database: database.to_string(),
            username: username.to_string(),
            password: password.to_string(),
        });
        self
    }

    pub fn auth_with_api_key(mut self, database: &str, api_key: &str) -> Self {
        self.auth = Some(AuthMethod::ApiKey {
            database: database.to_string(),
            api_key: api_key.to_string(),
        });
        self
    }

    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = Some(size);
        self
    }

    pub async fn build(self) -> Result<SoliDBClient, DriverError> {
        let pool_size = self.pool_size.unwrap_or(4);
        let mut client = SoliDBClient::connect_with_pool(&self.addr, pool_size).await?;

        if let Some(auth) = self.auth {
            match auth {
                AuthMethod::UsernamePassword {
                    database,
                    username,
                    password,
                } => {
                    client.auth(&database, &username, &password).await?;
                }
                AuthMethod::ApiKey { database, api_key } => {
                    client.auth_with_api_key(&database, &api_key).await?;
                }
            }
        }

        Ok(client)
    }
}
