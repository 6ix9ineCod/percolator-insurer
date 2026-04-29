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
        loop {
            let mut record = csv::StringRecord::new();
            match self.reader.read_record(&mut record) {
                Ok(true) => {}
                _ => return None,
            }
            if record.len() < 7 {
                continue;
            }
            let price_f: f64 = match record.get(1).and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            let qty_f: f64 = match record.get(2).and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            let timestamp_ms: u64 = match record.get(5).and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };
            let is_buyer_maker: bool = match record.get(6).and_then(|s| s.parse().ok()) {
                Some(v) => v,
                None => continue,
            };

            let price = (price_f * POS_SCALE as f64) as u64;
            let qty = (qty_f * POS_SCALE as f64) as u128;
            let is_buy = !is_buyer_maker;

            return Some(MarketEvent::Trade {
                timestamp_ms,
                price,
                qty,
                is_buy,
            });
        }
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
