use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::{protocol::Message, client::IntoClientRequest}};
use url::Url;
use crate::storage::collection::ChangeEvent;

/// Client for connecting to other nodes' WebSocket changefeeds
pub struct ClusterWebsocketClient;

impl ClusterWebsocketClient {
    /// Connect to a remote node's changefeed and return a stream of ChangeEvents
    pub async fn connect(
        node_addr: &str,
        database: &str,
        collection: &str,
        local_only: bool,
    ) -> anyhow::Result<impl futures::Stream<Item =  anyhow::Result<ChangeEvent>>> {
        // Construct WebSocket URL with cluster-internal authentication

        // Get cluster secret for authentication
        let cluster_secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
        if cluster_secret.is_empty() {
            return Err(anyhow::anyhow!("SOLIDB_CLUSTER_SECRET not set - cannot connect to cluster WebSocket"));
        }

        let url_str = format!(
            "ws://{}/_api/ws/changefeed?token=cluster-internal",
            node_addr
        );
        let url = Url::parse(&url_str)?;

        tracing::debug!("[CLUSTER-WS] Connecting to {} (local_only={})", url, local_only);

        // Connect with cluster secret header for authentication
        let mut request = IntoClientRequest::into_client_request(url.as_str())?;
        request.headers_mut().insert(
            "X-Cluster-Secret",
            cluster_secret.parse().unwrap(),
        );

        let (ws_stream, _) = connect_async(request).await?;
        let (mut write, mut read) = ws_stream.split();

        // Send subscription message
        let subscribe_msg = serde_json::json!({
            "type": "subscribe",
            "database": database,
            "collection": collection,
            "local": local_only
        });

        write.send(Message::Text(subscribe_msg.to_string().into())).await?;

        // Return a stream that parses messages
        let stream = async_stream::try_stream! {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        // Skip "subscribed" confirmation or errors for now, just try to parse event
                        if let Ok(event) = serde_json::from_str::<ChangeEvent>(&text) {
                            yield event;
                        } else {
                            // Might be a control message like {"type": "subscribed"}
                             tracing::trace!("[CLUSTER-WS] Received non-event message: {}", text);
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(e) => Err(anyhow::anyhow!("WebSocket error: {}", e))?,
                    _ => {}
                }
            }
        };

        Ok(stream)
    }
}
