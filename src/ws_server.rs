use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const CHANNEL_BLOCKS: &str = "blocks";
const CHANNEL_TRANSACTIONS: &str = "transactions";
const CHANNEL_MEMPOOL: &str = "mempool";

#[derive(Clone, Debug, Serialize)]
pub enum ChainEvent {
    NewBlock { hash: String, height: u64, timestamp: u64, tx_count: usize },
    Transaction { tx_hash: String, status: String, block_hash: String },
    Mempool { tx_hash: String, mempool_size: usize },
    ContractEvent { contract_id: String, topic: String, data: String },
}

impl ChainEvent {
    fn channel(&self) -> &'static str {
        match self {
            Self::NewBlock { .. } => CHANNEL_BLOCKS,
            Self::Transaction { .. } | Self::ContractEvent { .. } => CHANNEL_TRANSACTIONS,
            Self::Mempool { .. } => CHANNEL_MEMPOOL,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WsControlMessage {
    #[serde(rename = "type")]
    message_type: String,
    channel: String,
}

#[derive(Debug, Serialize)]
struct WsStatusMessage<'a> {
    #[serde(rename = "type")]
    message_type: &'a str,
    channel: &'a str,
}

#[derive(Debug, Serialize)]
struct WsErrorMessage<'a> {
    #[serde(rename = "type")]
    message_type: &'a str,
    error: &'a str,
}

#[derive(Debug, Serialize)]
struct WsEventMessage<'a> {
    #[serde(rename = "type")]
    message_type: &'a str,
    channel: &'a str,
    event: &'a ChainEvent,
}

fn parse_channel(channel: &str) -> Option<&'static str> {
    match channel {
        CHANNEL_BLOCKS => Some(CHANNEL_BLOCKS),
        CHANNEL_TRANSACTIONS => Some(CHANNEL_TRANSACTIONS),
        CHANNEL_MEMPOOL => Some(CHANNEL_MEMPOOL),
        _ => None,
    }
}

fn to_text_frame<T: Serialize>(
    payload: &T,
) -> Result<tokio_tungstenite::tungstenite::Message, serde_json::Error> {
    let text = serde_json::to_string(payload)?;
    Ok(tokio_tungstenite::tungstenite::Message::Text(text))
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
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
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
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::accept_async;

    let ws_stream = accept_async(stream).await?;
    let (mut write, mut read) = ws_stream.split();
    let mut subscriptions: HashSet<&'static str> = HashSet::new();

    loop {
        tokio::select! {
            inbound = read.next() => {
                match inbound {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        let parsed: Result<WsControlMessage, _> = serde_json::from_str(text.as_ref());
                        match parsed {
                            Ok(msg) => {
                                match msg.message_type.as_str() {
                                    "subscribe" => {
                                        if let Some(channel) = parse_channel(&msg.channel) {
                                            subscriptions.insert(channel);
                                            let frame = to_text_frame(&WsStatusMessage { message_type: "subscribed", channel })?;
                                            if write.send(frame).await.is_err() {
                                                break;
                                            }
                                        } else {
                                            let frame = to_text_frame(&WsErrorMessage {
                                                message_type: "error",
                                                error: "unknown channel",
                                            })?;
                                            if write.send(frame).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    "unsubscribe" => {
                                        if let Some(channel) = parse_channel(&msg.channel) {
                                            subscriptions.remove(channel);
                                            let frame = to_text_frame(&WsStatusMessage { message_type: "unsubscribed", channel })?;
                                            if write.send(frame).await.is_err() {
                                                break;
                                            }
                                        } else {
                                            let frame = to_text_frame(&WsErrorMessage {
                                                message_type: "error",
                                                error: "unknown channel",
                                            })?;
                                            if write.send(frame).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    _ => {
                                        let frame = to_text_frame(&WsErrorMessage {
                                            message_type: "error",
                                            error: "unsupported message type",
                                        })?;
                                        if write.send(frame).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(_) => {
                                let frame = to_text_frame(&WsErrorMessage {
                                    message_type: "error",
                                    error: "invalid JSON message",
                                })?;
                                if write.send(frame).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload))) => {
                        if write
                            .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            event_result = rx.recv() => {
                match event_result {
                    Ok(event) => {
                        let channel = event.channel();
                        if !subscriptions.contains(channel) {
                            continue;
                        }
                        let frame = to_text_frame(&WsEventMessage {
                            message_type: "event",
                            channel,
                            event: &event,
                        })?;
                        if write.send(frame).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_channel_parser_accepts_supported_channels() {
        assert_eq!(parse_channel("blocks"), Some("blocks"));
        assert_eq!(parse_channel("transactions"), Some("transactions"));
        assert_eq!(parse_channel("mempool"), Some("mempool"));
        assert_eq!(parse_channel("unknown"), None);
    }

    #[tokio::test]
    async fn websocket_subscribe_and_unsubscribe_filters_events() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::time::{timeout, Duration};
        use tokio_tungstenite::connect_async;

        let tcp_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let bind_addr = tcp_listener.local_addr().unwrap();
        drop(tcp_listener);

        let (tx, rx) = create_event_bus();
        let running = Arc::new(Mutex::new(true));
        start_ws_server(bind_addr.to_string(), rx, Arc::clone(&running));
        tokio::time::sleep(Duration::from_millis(150)).await;

        let url = format!("ws://{}", bind_addr);
        let (mut ws, _) = connect_async(url).await.unwrap();

        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"subscribe","channel":"blocks"}"#.to_string(),
        ))
        .await
        .unwrap();

        let _ack = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("subscribe ack timeout")
            .expect("ws closed")
            .expect("subscribe ack error");

        tx.send(ChainEvent::NewBlock {
            hash: "abcd".to_string(),
            height: 7,
            timestamp: 123456,
            tx_count: 2,
        })
        .unwrap();

        let event_msg = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("event timeout")
            .expect("ws closed")
            .expect("event error");
        let event_text = match event_msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            other => panic!("unexpected websocket frame: {:?}", other),
        };
        let event_json: serde_json::Value = serde_json::from_str(event_text.as_ref()).unwrap();
        assert_eq!(event_json["type"], "event");
        assert_eq!(event_json["channel"], "blocks");

        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"unsubscribe","channel":"blocks"}"#.to_string(),
        ))
        .await
        .unwrap();

        let _unsub_ack = timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("unsubscribe ack timeout")
            .expect("ws closed")
            .expect("unsubscribe ack error");

        tx.send(ChainEvent::NewBlock {
            hash: "ef01".to_string(),
            height: 8,
            timestamp: 123457,
            tx_count: 1,
        })
        .unwrap();

        let no_event = timeout(Duration::from_millis(250), ws.next()).await;
        assert!(no_event.is_err(), "event received after unsubscribe");

        *running.lock().unwrap() = false;
    }
}
