use crate::geoip::GeoInfo;
use serde::Serialize;
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize)]
pub struct HopStats {
    pub hop_num: u8,
    pub ip: Option<IpAddr>,
    pub hostname: Option<String>,
    pub geo_info: Option<GeoInfo>,
    pub sent: u32,
    pub recv: u32,
    pub last_rtt: Option<f64>, // in ms
    pub rtts: Vec<f64>,        // history of RTTs in ms
}

impl HopStats {
    pub fn new(hop_num: u8) -> Self {
        Self {
            hop_num,
            ip: None,
            hostname: None,
            geo_info: None,
            sent: 0,
            recv: 0,
            last_rtt: None,
            rtts: Vec::new(),
        }
    }

    pub fn register_probe(&mut self, ip: Option<IpAddr>, rtt: Option<f64>) {
        self.sent += 1;
        if let Some(rtt_val) = rtt {
            self.recv += 1;
            self.last_rtt = Some(rtt_val);
            self.rtts.push(rtt_val);
            if self.rtts.len() > 100 {
                self.rtts.remove(0); // keep window size of 100 for live TUI/web
            }
        } else {
            self.last_rtt = None;
        }

        if ip.is_some() {
            self.ip = ip;
        }
    }

    pub fn loss_pct(&self) -> f64 {
        if self.sent == 0 {
            0.0
        } else {
            ((self.sent - self.recv) as f64 / self.sent as f64) * 100.0
        }
    }

    pub fn avg_rtt(&self) -> f64 {
        if self.rtts.is_empty() {
            0.0
        } else {
            self.rtts.iter().sum::<f64>() / self.rtts.len() as f64
        }
    }

    pub fn min_rtt(&self) -> f64 {
        if self.rtts.is_empty() {
            0.0
        } else {
            *self
                .rtts
                .iter()
                .min_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(&0.0)
        }
    }

    pub fn max_rtt(&self) -> f64 {
        if self.rtts.is_empty() {
            0.0
        } else {
            *self
                .rtts
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap_or(&0.0)
        }
    }

    pub fn jitter(&self) -> f64 {
        if self.rtts.len() < 2 {
            return 0.0;
        }
        let mut diff_sum = 0.0;
        for i in 1..self.rtts.len() {
            diff_sum += (self.rtts[i] - self.rtts[i - 1]).abs();
        }
        diff_sum / (self.rtts.len() - 1) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hop_with_rtts(rtts: &[f64]) -> HopStats {
        let mut hop = HopStats::new(1);
        for &r in rtts {
            hop.register_probe(None, Some(r));
        }
        hop
    }

    #[test]
    fn test_avg_rtt() {
        let hop = make_hop_with_rtts(&[10.0, 20.0, 30.0]);
        assert!((hop.avg_rtt() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn test_min_rtt() {
        let hop = make_hop_with_rtts(&[10.0, 5.0, 30.0]);
        assert!((hop.min_rtt() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_max_rtt() {
        let hop = make_hop_with_rtts(&[10.0, 5.0, 30.0]);
        assert!((hop.max_rtt() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn test_jitter() {
        // jitter = mean of absolute successive differences
        // |20-10| + |10-20| = 10 + 10 = 20; /2 = 10
        let hop = make_hop_with_rtts(&[10.0, 20.0, 10.0]);
        assert!((hop.jitter() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_loss_pct_no_loss() {
        let mut hop = HopStats::new(1);
        hop.register_probe(None, Some(5.0));
        hop.register_probe(None, Some(6.0));
        assert!((hop.loss_pct() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_loss_pct_with_drops() {
        let mut hop = HopStats::new(1);
        hop.register_probe(None, Some(5.0)); // received
        hop.register_probe(None, None); // dropped
        hop.register_probe(None, None); // dropped
        hop.register_probe(None, Some(6.0)); // received
                                             // 2 dropped out of 4 sent = 50%
        assert!((hop.loss_pct() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_empty_hop() {
        let hop = HopStats::new(1);
        assert_eq!(hop.avg_rtt(), 0.0);
        assert_eq!(hop.min_rtt(), 0.0);
        assert_eq!(hop.max_rtt(), 0.0);
        assert_eq!(hop.jitter(), 0.0);
        assert_eq!(hop.loss_pct(), 0.0);
    }

    #[test]
    fn test_rtt_window_size() {
        // rtts window should cap at 100
        let mut hop = HopStats::new(1);
        for i in 0..150 {
            hop.register_probe(None, Some(i as f64));
        }
        assert_eq!(hop.rtts.len(), 100);
    }
}
