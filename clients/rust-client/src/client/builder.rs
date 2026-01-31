use super::DriverError;
use super::HttpClient;
use super::SoliDBClient;

pub struct SoliDBClientBuilder {
    addr: String,
    auth: Option<AuthMethod>,
    timeout_ms: Option<u64>,
    pool_size: Option<usize>,
    transport: Transport,
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

#[derive(Clone, Copy, Default)]
pub enum Transport {
    #[default]
    Http,
    Tcp,
}

impl SoliDBClientBuilder {
    pub fn new(addr: &str) -> Self {
        let addr = addr.to_string();
        let is_http = addr.starts_with("http://") || addr.starts_with("https://");

        Self {
            addr,
            auth: None,
            timeout_ms: None,
            pool_size: None,
            transport: if is_http {
                Transport::Http
            } else {
                Transport::Tcp
            },
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

    pub fn use_http(mut self) -> Self {
        self.transport = Transport::Http;
        self
    }

    pub fn use_tcp(mut self) -> Self {
        self.transport = Transport::Tcp;
        self
    }

    pub async fn build_http(self) -> Result<HttpClient, DriverError> {
        let mut client = HttpClient::new(&self.addr);

        if let Some(database) = self.auth.as_ref().map(|a| match a {
            AuthMethod::UsernamePassword { database, .. } => database.clone(),
            AuthMethod::ApiKey { database, .. } => database.clone(),
        }) {
            client.set_database(&database);
        }

        if let Some(auth) = self.auth {
            match auth {
                AuthMethod::UsernamePassword {
                    database,
                    username,
                    password,
                } => {
                    client.login(&database, &username, &password).await?;
                }
                AuthMethod::ApiKey { database, api_key } => {
                    let mut http_client = HttpClient::new(&self.addr);
                    http_client.set_database(&database);
                    http_client.set_token(&api_key);
                    return Ok(http_client);
                }
            }
        }

        Ok(client)
    }

    pub async fn build(self) -> Result<SoliDBClient, DriverError> {
        match self.transport {
            Transport::Http => {
                let _ = self.build_http().await?;
                Err(DriverError::ProtocolError(
                    "Use build_http() for HTTP transport. For TCP, use .use_tcp() or a host:port address.".to_string(),
                ))
            }
            Transport::Tcp => {
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
    }

    pub async fn build_with_transport(self) -> Result<SoliDBClient, DriverError> {
        match self.transport {
            Transport::Http => {
                let _ = self.build_http().await?;
                Err(DriverError::ProtocolError(
                    "HTTP transport selected but build() was called. Use build_http() instead."
                        .to_string(),
                ))
            }
            Transport::Tcp => self.build().await,
        }
    }
}
