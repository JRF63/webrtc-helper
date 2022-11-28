use super::time::TwccTime;
use crate::util::data_rate::DataRate;
use std::sync::{
    atomic::{AtomicI64, AtomicU64, Ordering},
    Arc,
};

// To be able to index in the range [0, u16::MAX]
const TWCC_ARRAY_SIZE: usize = (u16::MAX as usize) + 1;

// Box<[T; N]> is used instead of Vec<T> or Box<[T]> to help the compiler to elide-out the bounds
// check when indexing with a u16.
#[derive(Clone)]
#[repr(transparent)]
pub struct TwccSendInfo(Arc<Box<[(AtomicI64, AtomicU64); TWCC_ARRAY_SIZE]>>);

impl TwccSendInfo {
    // This allocates a relatively large ~1 MB fixed-size array
    pub fn new() -> TwccSendInfo {
        let mut vec = Vec::new();
        vec.reserve_exact(TWCC_ARRAY_SIZE);

        for _ in 0..TWCC_ARRAY_SIZE {
            vec.push(Default::default());
        }

        let boxed_array = TryFrom::try_from(vec.into_boxed_slice()).unwrap();
        TwccSendInfo(Arc::new(boxed_array))
    }

    pub fn store(&self, index: u16, timestamp: TwccTime, packet_size: u64) {
        let (a, b) = &self.0[index as usize];
        a.store(timestamp.as_raw(), Ordering::Release);
        b.store(packet_size, Ordering::Release);
    }

    pub fn load(&self, index: u16) -> (TwccTime, u64) {
        let (a, b) = &self.0[index as usize];
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
        // 256 Kbps
        const INITIAL_BANDWIDTH: u64 = 256_000;

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
