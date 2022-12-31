use tokio::sync::watch;

#[derive(Clone, Copy, PartialEq, Default)]
pub struct DataRate(f64);

impl DataRate {
    #[inline]
    pub fn from_bits_per_sec(bits_per_sec: u64) -> DataRate {
        DataRate(bits_per_sec as f64 / 8.0)
    }

    #[inline]
    pub fn from_bytes_per_sec_f64(bytes_per_sec: f64) -> DataRate {
        DataRate(bytes_per_sec)
    }

    #[inline]
    pub fn bits_per_sec(&self) -> u64 {
        self.0 as u64 * 8
    }

    #[inline]
    pub fn bytes_per_sec_f64(&self) -> f64 {
        self.0
    }
}

pub type TwccBandwidthEstimate = watch::Receiver<DataRate>;

pub type TwccBandwidthSender = watch::Sender<DataRate>;

/// Create a new channel for sending/receiving the bandwidth estimate.
pub(crate) fn twcc_bandwidth_estimate_channel(
) -> (watch::Sender<DataRate>, watch::Receiver<DataRate>) {
    const INIT_RATE: u64 = 1_000_000; // 1 Mbps
    watch::channel(DataRate::from_bits_per_sec(INIT_RATE))
}
