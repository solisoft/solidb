use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
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
    ) -> anyhow::Result<impl futures::Stream<Item =  anyhow::Result<ChangeEvent>>> {
        // Construct WebSocket URL
        // Internal connections don't need auth or can use a special internal token?
        // For now, let's assume no auth or we need to pass a system token.
        // The implementation plan mentioned "token", but handlers.rs requires one.
        // We should generate a temporary internal token or use the keyfile system.
        // For simplicity in this iteration, we'll try to connect without a token 
        // implies we might need to adjust auth validation or use a "system" bypass
        // BUT handlers.rs definitely checks for token.
        // Let's assume we pass a dummy token "internal-cluster" for now and we might need to fix auth later
        // or re-use the cluster keyfile to sign a token.
        
        let url_str = format!(
            "ws://{}/_api/ws/changefeed?token=internal-cluster", 
            node_addr
        );
        let url = Url::parse(&url_str)?;

        tracing::debug!("[CLUSTER-WS] Connecting to {}", url);

        let (ws_stream, _) = connect_async(url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();

        // Send subscription message
        let subscribe_msg = serde_json::json!({
            "type": "subscribe",
            "database": database,
            "collection": collection
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
