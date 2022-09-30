//! https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-6

pub struct LossBasedBandwidthEstimator;

impl LossBasedBandwidthEstimator {
    pub fn new() -> LossBasedBandwidthEstimator {
        LossBasedBandwidthEstimator {}
    }

    pub fn estimate(&mut self, current_bandwidth: f32, received: u32, lost: u32) -> f32 {
        let total = received + lost;
        let fraction_lost = lost as f32 / total as f32;
        if fraction_lost < 0.02 {
            current_bandwidth * 1.05
        } else if fraction_lost > 0.10 {
            current_bandwidth * (1.0 - 0.5 * fraction_lost)
        } else {
            current_bandwidth
        }
    }
}