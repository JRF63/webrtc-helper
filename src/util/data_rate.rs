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
        (self.0 * 8.0) as u64
    }

    #[inline]
    pub fn bytes_per_sec_f64(&self) -> f64 {
        self.0
    }

    #[inline]
    pub(crate) fn as_blob(&self) -> u64 {
        u64::from_ne_bytes(self.0.to_ne_bytes())
    }

    #[inline]
    pub(crate) fn from_blob(blob: u64) -> DataRate {
        DataRate(f64::from_ne_bytes(blob.to_ne_bytes()))
    }
}
