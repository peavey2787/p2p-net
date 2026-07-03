use p2p_net::{start_node, NodeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = NodeConfig {
        listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        ..NodeConfig::default()
    };
    let handle = start_node(cfg).await?;

    let mut messages = handle.subscribe("example/general").await?;
    handle
        .broadcast("example/general", b"hello mesh".to_vec())
        .await?;
    let peers = handle.get_peers().await?;
    println!("connected peers: {}", peers.len());

    tokio::select! {
        maybe_message = messages.recv() => {
            if let Ok(message) = maybe_message {
                println!("received {} bytes on {}", message.payload.len(), message.topic);
            }
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
    }

    handle.shutdown().await;
    Ok(())
}
