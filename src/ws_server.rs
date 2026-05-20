use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Serialize)]
pub enum ChainEvent {
    NewBlock {
        hash: String,
        height: u64,
        timestamp: u64,
        tx_count: usize,
    },
    Transaction {
        tx_hash: String,
        status: String,
        block_hash: String,
    },
    ContractEvent {
        contract_id: String,
        topic: String,
        data: String,
    },
}

pub fn create_event_bus() -> (broadcast::Sender<ChainEvent>, broadcast::Receiver<ChainEvent>) {
    broadcast::channel(1024)
}

pub fn start_ws_server(
    bind_addr: String,
    rx: broadcast::Receiver<ChainEvent>,
    is_running: Arc<Mutex<bool>>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("Failed to build WS tokio runtime: {}", e);
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Failed to bind WS server to {}: {}", bind_addr, e);
                    return;
                }
            };
            log::info!("WebSocket server listening on ws://{}", bind_addr);

            let mut shutdown_rx = {
                let running = is_running.lock().unwrap();
                let (tx, rx) = tokio::sync::watch::channel(*running);
                drop(running);
                // spawn a task that flips the watch when is_running becomes false
                let is_running_clone = Arc::clone(&is_running);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let r = is_running_clone.lock().unwrap();
                        if !*r {
                            let _ = tx.send(false);
                        }
                        drop(r);
                    }
                });
                rx
            };

            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        let (stream, peer_addr) = match accept_result {
                            Ok(v) => v,
                            Err(e) => {
                                log::error!("WS accept error: {}", e);
                                continue;
                            }
                        };
                        log::debug!("WS connection from {}", peer_addr);
                        let rx = rx.resubscribe();
                        tokio::spawn(async move {
                            if let Err(e) = handle_ws(stream, rx).await {
                                log::debug!("WS client {} disconnected: {}", peer_addr, e);
                            }
                        });
                    }
                    _ = shutdown_rx.changed() => {
                        if !*shutdown_rx.borrow() {
                            log::info!("WS server shutting down");
                            break;
                        }
                    }
                }
            }
        });
    });
}

async fn handle_ws(
    stream: tokio::net::TcpStream,
    mut rx: broadcast::Receiver<ChainEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::StreamExt;
    use tokio_tungstenite::accept_async;

    let ws_stream = accept_async(stream).await?;
    let (mut write, _) = ws_stream.split();

    loop {
        match rx.recv().await {
            Ok(event) => {
                let msg = serde_json::to_string(&event)?;
                let frame = tokio_tungstenite::tungstenite::Message::Text(msg);
                use futures_util::SinkExt;
                if write.send(frame).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
        }
    }
    Ok(())
}
