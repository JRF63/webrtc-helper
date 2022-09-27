use std::{
    sync::{
        atomic::{AtomicI64, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use webrtc::rtcp::transport_feedbacks::transport_layer_cc::{
    RecvDelta, SymbolTypeTcc, TransportLayerCc,
};

const REFERENCE_TIME_WRAPAROUND: i64 = (1 << 24) * 64000;
const PROBABLE_WRAPAROUND_THRESHOLD: i64 = REFERENCE_TIME_WRAPAROUND / 2;

// Timestamp is always in the half-open domain [0, 1073741824000).
#[derive(Clone, Copy, PartialEq)]
pub struct TwccTime(i64);

impl TwccTime {
    /// Reinterpret a `Duration` as a `TwccTime` timestamp.
    pub const fn from_duration(timestamp: &Duration) -> TwccTime {
        let val = timestamp.as_micros() % (REFERENCE_TIME_WRAPAROUND as u128);
        TwccTime(val as i64)
    }

    /// Read the reference time of a TWCC RTCP packet.
    pub const fn extract_from_rtcp(rtcp: &TransportLayerCc) -> TwccTime {
        // The draft says the reference time should be a 24-bit *signed* integer but the reference
        // implementation treats it as an unsigned.
        let val = rtcp.reference_time as i64 * 64000;
        TwccTime(val)
    }

    /// Build a new `TwccTime` given a base time and a time delta.
    pub const fn from_recv_delta(base_time: TwccTime, recv_delta: &RecvDelta) -> TwccTime {
        let mut val = base_time.0;
        match recv_delta.type_tcc_packet {
            SymbolTypeTcc::PacketReceivedSmallDelta => {
                val += recv_delta.delta;
            }
            SymbolTypeTcc::PacketReceivedLargeDelta => {
                // Map to [-8192000, 8191750] microseconds
                // https://datatracker.ietf.org/doc/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.5
                val += recv_delta.delta - 8192000;
            }
            _ => (),
        }
        // Keep the timestamp inside [0, 1073741824000).
        if val < 0 {
            val += REFERENCE_TIME_WRAPAROUND;
        } else if val >= REFERENCE_TIME_WRAPAROUND {
            val -= REFERENCE_TIME_WRAPAROUND;
        }
        TwccTime(val)
    }

    /// Subtract `rhs` from `self` assuming they have close values. Large differences are assumed
    /// to be done over the wrap-around point.
    pub const fn small_delta_sub(self, rhs: TwccTime) -> i64 {
        let mut val = self.0 - rhs.0;
        if val < -PROBABLE_WRAPAROUND_THRESHOLD {
            val += REFERENCE_TIME_WRAPAROUND;
        } else if val > PROBABLE_WRAPAROUND_THRESHOLD {
            val -= REFERENCE_TIME_WRAPAROUND;
        }
        val
    }

    /// Compare this `TwccTime` to another assuming they have close values. Large differences are
    /// assumed to be done over the wrap-around point.
    const fn small_delta_cmp(&self, other: &TwccTime) -> std::cmp::Ordering {
        const MIN_I64: i64 = i64::MIN;
        const MAX_I64: i64 = i64::MAX;
        match self.small_delta_sub(*other) {
            0 => std::cmp::Ordering::Equal,
            1..=MAX_I64 => std::cmp::Ordering::Greater,
            MIN_I64..=-1 => std::cmp::Ordering::Less,
        }
    }
}

// Impl'ed for readability in the delay-based control.
impl PartialOrd for TwccTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.small_delta_cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtraction() {
        let mut timestamp = Duration::from_micros(1073741696000);
        let a = TwccTime::from_duration(&timestamp);
        let delta_1 = 64000;
        timestamp += Duration::from_micros(delta_1);
        let b = TwccTime::from_duration(&timestamp);
        assert_eq!(b.small_delta_sub(a), delta_1 as i64);

        // Wraps around
        let delta_2 = 640000;
        timestamp += Duration::from_micros(delta_2);
        let c = TwccTime::from_duration(&timestamp);
        assert!(b.0 > c.0);
        assert_eq!(c.small_delta_sub(b), delta_2 as i64);

        assert_eq!(a.small_delta_sub(a), 0);
        assert_eq!(b.small_delta_sub(a), -a.small_delta_sub(b));
    }

    #[test]
    fn ordering() {
        let mut timestamp = Duration::from_micros(1073741696000);
        let a = TwccTime::from_duration(&timestamp);
        timestamp += Duration::from_micros(64000);
        let b = TwccTime::from_duration(&timestamp);
        timestamp += Duration::from_micros(640000);
        let c = TwccTime::from_duration(&timestamp); // Wraps around
        assert!(b.0 > c.0);

        assert_eq!(a.small_delta_cmp(&a), std::cmp::Ordering::Equal);
        assert_eq!(b.small_delta_cmp(&a), std::cmp::Ordering::Greater);
        assert_eq!(c.small_delta_cmp(&b), std::cmp::Ordering::Greater);
        assert_eq!(c.small_delta_cmp(&a), std::cmp::Ordering::Greater);
        assert_eq!(a.small_delta_cmp(&b), std::cmp::Ordering::Less);
        assert_eq!(b.small_delta_cmp(&c), std::cmp::Ordering::Less);
        assert_eq!(a.small_delta_cmp(&c), std::cmp::Ordering::Less);

        let thirty_hours = Duration::from_secs(30 * 3600);
        let mut timestamp = Duration::from_micros(0);
        let mut prev = TwccTime::from_duration(&timestamp);
        for _ in 0..20 {
            timestamp += thirty_hours;
            let current = TwccTime::from_duration(&timestamp);
            assert_eq!(current.small_delta_cmp(&prev), std::cmp::Ordering::Greater);
            prev = current;
        }
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct TwccSendInfo(Arc<Box<[(AtomicI64, AtomicU64)]>>);

impl TwccSendInfo {
    // This allocates a relatively large ~1 MB fixed-size array
    pub fn new() -> TwccSendInfo {
        const ALLOC_SIZE: usize = (u16::MAX as usize) + 1;

        let mut vec = Vec::new();
        vec.reserve_exact(ALLOC_SIZE);

        for _ in 0..ALLOC_SIZE {
            vec.push(Default::default());
        }

        TwccSendInfo(Arc::new(vec.into_boxed_slice()))
    }

    fn get(&self, index: u16) -> &(AtomicI64, AtomicU64) {
        // SAFETY: The inner vec has (u16::MAX + 1) values so indices up to u16::MAX are valid
        unsafe { self.0.get_unchecked(index as usize) }
    }

    pub fn store(&self, index: u16, timestamp: TwccTime, packet_size: u64) {
        let (a, b) = self.get(index);
        a.store(timestamp.0, Ordering::Release);
        b.store(packet_size, Ordering::Release);
    }

    pub fn load(&self, index: u16) -> (TwccTime, u64) {
        let (a, b) = self.get(index);
        (
            TwccTime(a.load(Ordering::Acquire)),
            b.load(Ordering::Acquire),
        )
    }
}
