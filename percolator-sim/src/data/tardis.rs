use crate::{DataSource, MarketEvent, POS_SCALE};
use std::path::Path;

pub struct TardisBookSource {
    reader: csv::Reader<std::fs::File>,
    pending_bids: Vec<(u64, u128)>,
    pending_asks: Vec<(u64, u128)>,
    pending_timestamp_ms: u64,
    has_pending: bool,
}

impl TardisBookSource {
    pub fn from_path(path: &Path) -> Result<Self, csv::Error> {
        let reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(path)?;
        Ok(Self {
            reader,
            pending_bids: Vec::new(),
            pending_asks: Vec::new(),
            pending_timestamp_ms: 0,
            has_pending: false,
        })
    }

    fn parse_timestamp(ts: &str) -> Option<u64> {
        let dt = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
        Some(dt.timestamp_millis() as u64)
    }
}

impl DataSource for TardisBookSource {
    fn next_event(&mut self) -> Option<MarketEvent> {
        loop {
            let mut record = csv::StringRecord::new();
            let has_more = self.reader.read_record(&mut record).ok()?;

            if !has_more {
                if self.has_pending {
                    self.has_pending = false;
                    return Some(MarketEvent::BookUpdate {
                        timestamp_ms: self.pending_timestamp_ms,
                        bids: std::mem::take(&mut self.pending_bids),
                        asks: std::mem::take(&mut self.pending_asks),
                    });
                }
                return None;
            }

            if record.len() < 8 {
                continue;
            }

            let ts_str = record.get(2)?;
            let side = record.get(5)?;
            let price_str = record.get(6)?;
            let amount_str = record.get(7)?;

            let ts = Self::parse_timestamp(ts_str)?;
            let price_f: f64 = price_str.parse().ok()?;
            let amount_f: f64 = amount_str.parse().ok()?;
            let price = (price_f * POS_SCALE as f64) as u64;
            let amount = (amount_f * POS_SCALE as f64) as u128;

            if self.has_pending && ts != self.pending_timestamp_ms {
                let event = MarketEvent::BookUpdate {
                    timestamp_ms: self.pending_timestamp_ms,
                    bids: std::mem::take(&mut self.pending_bids),
                    asks: std::mem::take(&mut self.pending_asks),
                };
                self.pending_timestamp_ms = ts;
                match side {
                    "bid" => self.pending_bids.push((price, amount)),
                    "ask" => self.pending_asks.push((price, amount)),
                    _ => {}
                }
                return Some(event);
            }

            self.has_pending = true;
            self.pending_timestamp_ms = ts;
            match side {
                "bid" => self.pending_bids.push((price, amount)),
                "ask" => self.pending_asks.push((price, amount)),
                _ => {}
            }
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
        writeln!(f, "exchange,symbol,timestamp,local_timestamp,is_snapshot,side,price,amount").unwrap();
        write!(f, "{}", rows).unwrap();
        f
    }

    #[test]
    fn parse_book_snapshot() {
        let csv = concat!(
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,30000.0,1.5\n",
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,29999.0,2.0\n",
            "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,ask,30001.0,1.0\n",
        );
        let f = make_csv(csv);
        let mut src = TardisBookSource::from_path(f.path()).unwrap();
        let event = src.next_event().unwrap();
        match event {
            MarketEvent::BookUpdate { bids, asks, .. } => {
                assert_eq!(bids.len(), 2);
                assert_eq!(asks.len(), 1);
            }
            _ => panic!("expected BookUpdate"),
        }
    }

    #[test]
    fn empty_after_all_consumed() {
        let csv = "binance,BTCUSDT,2022-05-09T00:00:00.000Z,2022-05-09T00:00:00.100Z,true,bid,30000.0,1.5\n";
        let f = make_csv(csv);
        let mut src = TardisBookSource::from_path(f.path()).unwrap();
        assert!(src.next_event().is_some());
        assert!(src.next_event().is_none());
    }
}
