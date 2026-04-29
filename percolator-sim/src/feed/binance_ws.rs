use crate::{MarketEvent, POS_SCALE};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use futures_util::StreamExt;
use tokio::sync::mpsc;

pub async fn connect_binance_trades(
    symbol: &str,
    tx: mpsc::Sender<MarketEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("wss://fstream.binance.com/ws/{}@aggTrade", symbol.to_lowercase());
    let (ws_stream, _) = connect_async(&url).await?;
    let (_, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Ping(_)) => continue,
            Ok(_) => continue,
            Err(_) => break,
        };

        let v: serde_json::Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let price_f: f64 = v["p"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let qty_f: f64 = v["q"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let timestamp_ms = v["T"].as_u64().unwrap_or(0);
        let is_buyer_maker = v["m"].as_bool().unwrap_or(false);

        let event = MarketEvent::Trade {
            timestamp_ms,
            price: (price_f * POS_SCALE as f64) as u64,
            qty: (qty_f * POS_SCALE as f64) as u128,
            is_buy: !is_buyer_maker,
        };

        if tx.send(event).await.is_err() {
            break;
        }
    }

    Ok(())
}
