use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::{info, error};
use super::log::ReplicationLog;
use crate::cluster::transport::{Transport, ClusterMessage};

pub struct ReplicationWorker {
    log: Arc<ReplicationLog>,
    transport: Arc<dyn Transport>,
    cluster_manager: Arc<crate::cluster::manager::ClusterManager>,
}

impl ReplicationWorker {
    pub fn new(
        log: Arc<ReplicationLog>, 
        transport: Arc<dyn Transport>,
        cluster_manager: Arc<crate::cluster::manager::ClusterManager>,
    ) -> Self {
        Self {
            log,
            transport,
            cluster_manager,
        }
    }

    pub async fn start(self) {
        let mut tick = interval(Duration::from_millis(100));
        let mut tick_count: u64 = 0;
        loop {
            tick.tick().await;
            tick_count += 1;
            
            // Get current max sequence from log
            let current_seq = self.log.current_sequence();
            
            // Iterate over active peers
            let active_nodes = self.cluster_manager.state().active_nodes();
            
            for peer in &active_nodes {
                if peer.id == self.cluster_manager.local_node_id() {
                    continue;
                }
                
                // Get last sequence we sent to this peer from OUR log
                let peer_last_seq = self.cluster_manager.state().get_sent_to_peer(&peer.id);
                
                // If peer is behind, replicate batch
                if peer_last_seq < current_seq {
                    // Read entries from log (from peer_last_seq + 1)
                     let entries = self.log.get_entries_after(peer_last_seq, 10000);
                     if !entries.is_empty() {
                             let last_entry_seq = entries.last().map(|e| e.sequence).unwrap_or(0);
                             tracing::info!("Replicating {} entries to peer {} (seq {} -> {})", 
                                   entries.len(), peer.id, peer_last_seq, last_entry_seq);
                             
                             let msg = ClusterMessage::Replication(
                                 crate::replication::protocol::ReplicationMessage::SyncResponse {
                                     entries,
                                     current_sequence: current_seq,
                                 }
                             );
                             
                             if let Ok(_) = self.transport.send(&peer.address, msg).await {
                                 // Update what we sent to this peer from our local log
                                 self.cluster_manager.state().update_sent_to_peer(&peer.id, last_entry_seq);
                             }
                         }

                }
            }
            
            // Send Heartbeat every 10 ticks (1 second)
            if tick_count % 10 == 0 {
                let heartbeat = ClusterMessage::Heartbeat { 
                    from: self.cluster_manager.local_node_id(),
                    sequence: current_seq 
                };
                
                // Broadcast heartbeat (iterate peers and send)
                for peer in active_nodes {
                     if peer.id != self.cluster_manager.local_node_id() {
                         tracing::debug!("Sending heartbeat to {} (seq {})", peer.id, current_seq);
                         let _ = self.transport.send(&peer.address, heartbeat.clone()).await;
                     }
                }
            }
        }
    }
    

}
