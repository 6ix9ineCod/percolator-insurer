use crate::{DataSource, MarketEvent, POS_SCALE};
use std::path::Path;

pub struct BinanceTradeSource {
    reader: csv::Reader<std::fs::File>,
}

impl BinanceTradeSource {
    pub fn from_path(path: &Path) -> Result<Self, csv::Error> {
        let reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)?;
        Ok(Self { reader })
    }
}

impl DataSource for BinanceTradeSource {
    fn next_event(&mut self) -> Option<MarketEvent> {
        let mut record = csv::StringRecord::new();
        if !self.reader.read_record(&mut record).ok()? {
            return None;
        }
        if record.len() < 7 {
            return None;
        }
        let price_str = record.get(1)?;
        let qty_str = record.get(2)?;
        let timestamp_str = record.get(5)?;
        let is_buyer_maker_str = record.get(6)?;

        let price_f: f64 = price_str.parse().ok()?;
        let qty_f: f64 = qty_str.parse().ok()?;
        let timestamp_ms: u64 = timestamp_str.parse().ok()?;
        let is_buyer_maker: bool = is_buyer_maker_str.parse().ok()?;

        let price = (price_f * POS_SCALE as f64) as u64;
        let qty = (qty_f * POS_SCALE as f64) as u128;
        let is_buy = !is_buyer_maker;

        Some(MarketEvent::Trade {
            timestamp_ms,
            price,
            qty,
            is_buy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarketEvent;
    use std::io::Write;

    fn make_csv(rows: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", rows).unwrap();
        f
    }

    #[test]
    fn parse_single_trade() {
        let csv = "123,50000.50,0.001,100,100,1700000000000,false,true\n";
        let f = make_csv(csv);
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        let event = src.next_event().unwrap();
        match event {
            MarketEvent::Trade { timestamp_ms, price, qty, is_buy } => {
                assert_eq!(timestamp_ms, 1700000000000);
                assert_eq!(price, 50000500000);
                assert!(qty > 0);
                assert_eq!(is_buy, true);
            }
            _ => panic!("expected Trade"),
        }
    }

    #[test]
    fn parse_multiple_trades() {
        let csv = "1,50000.0,1.0,1,1,1000,false,true\n2,50001.0,2.0,2,2,2000,true,true\n";
        let f = make_csv(csv);
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_none());
    }

    #[test]
    fn empty_file() {
        let f = make_csv("");
        let mut src = BinanceTradeSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_none());
    }
}
