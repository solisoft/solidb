use crate::storage::collection::ChangeEvent;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message},
};
use url::Url;

/// Client for connecting to other nodes' WebSocket changefeeds
pub struct ClusterWebsocketClient;

impl ClusterWebsocketClient {
    /// Connect to a remote node's changefeed and return a stream of ChangeEvents
    ///
    /// # Arguments
    /// * `node_addr` - Address of the remote node
    /// * `database` - Database name
    /// * `collection` - Collection name
    /// * `local_only` - Whether to get only local changes
    /// * `cluster_secret` - Cluster secret from keyfile for authentication
    pub async fn connect(
        node_addr: &str,
        database: &str,
        collection: &str,
        local_only: bool,
        cluster_secret: &str,
    ) -> anyhow::Result<impl futures::Stream<Item = anyhow::Result<ChangeEvent>>> {
        // Construct WebSocket URL with cluster-internal authentication

        if cluster_secret.is_empty() {
            return Err(anyhow::anyhow!(
                "Cluster secret not configured - cannot connect to cluster WebSocket"
            ));
        }

        // wss:// when the cluster scheme is https (audit L1): the header
        // below carries the raw cluster secret.
        let url_str =
            super::http::peer_ws_url(node_addr, "/_api/ws/changefeed?token=cluster-internal");
        let url = Url::parse(&url_str)?;

        tracing::debug!(
            "[CLUSTER-WS] Connecting to {} (local_only={})",
            url,
            local_only
        );

        // Connect with cluster secret header for authentication
        let mut request = IntoClientRequest::into_client_request(url.as_str())?;
        request.headers_mut().insert(
            "X-Cluster-Secret",
            cluster_secret
                .parse()
                .map_err(|_| anyhow::anyhow!("cluster secret is not a valid header value"))?,
        );

        let (ws_stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(10), connect_async(request))
                .await
                .map_err(|_| anyhow::anyhow!("WebSocket connect to {} timed out", node_addr))??;
        let (mut write, mut read) = ws_stream.split();

        // Send subscription message
        let subscribe_msg = serde_json::json!({
            "type": "subscribe",
            "database": database,
            "collection": collection,
            "local": local_only
        });

        write
            .send(Message::Text(subscribe_msg.to_string().into()))
            .await?;

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
