use super::time::TwccTime;
use crate::util::data_rate::DataRate;
use std::sync::{
    atomic::{AtomicI64, AtomicU64, Ordering},
    Arc,
};

/// Exact sized needed to be able to index in the range [0, u16::MAX]
const TWCC_ARRAY_SIZE: usize = (u16::MAX as usize) + 1;

/// TWCC data structure for storing the timestamp and size of each packet sent.
///
// Box<[T; N]> is used instead of Vec<T> or Box<[T]> to help the compiler to elide-out the bounds
// check when indexing with a u16. `TwccSendInfo` requires approx. ~1 MB of heap memory.
#[derive(Clone)]
#[repr(transparent)]
pub struct TwccSendInfo(Arc<Box<[(AtomicI64, AtomicU64); TWCC_ARRAY_SIZE]>>);

impl TwccSendInfo {
    /// Create a new `TwccSendInfo`.
    pub fn new() -> TwccSendInfo {
        let mut vec = Vec::new();
        vec.reserve_exact(TWCC_ARRAY_SIZE);

        for _ in 0..TWCC_ARRAY_SIZE {
            vec.push(Default::default());
        }

        let boxed_array = TryFrom::try_from(vec.into_boxed_slice()).unwrap();
        TwccSendInfo(Arc::new(boxed_array))
    }

    /// Stores the timestamp and packet size of the packet.
    pub fn store_send_info(&self, seq_num: u16, timestamp: TwccTime, packet_size: u64) {
        let (a, b) = &self.0[seq_num as usize];
        a.store(timestamp.as_raw(), Ordering::Release);
        b.store(packet_size, Ordering::Release);
    }

    /// Load the timestamp and packet size for the packet with the given sequence number.
    pub fn load_send_info(&self, seq_num: u16) -> (TwccTime, u64) {
        let (a, b) = &self.0[seq_num as usize];
        (
            TwccTime::from_raw(a.load(Ordering::Acquire)),
            b.load(Ordering::Acquire),
        )
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct TwccBandwidthEstimate(Arc<AtomicU64>);

impl TwccBandwidthEstimate {
    pub fn new() -> TwccBandwidthEstimate {
        // 1 Mbps
        const INITIAL_BANDWIDTH: u64 = 50_000_000;

        TwccBandwidthEstimate(Arc::new(AtomicU64::new(
            DataRate::from_bits_per_sec(INITIAL_BANDWIDTH).as_blob(),
        )))
    }

    pub(crate) fn set_estimate(&self, bandwidth: DataRate) {
        self.0.store(bandwidth.as_blob(), Ordering::Release);
    }

    pub fn get_estimate(&self) -> DataRate {
        DataRate::from_blob(self.0.load(Ordering::Acquire))
    }
}
