//! https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02#section-6

pub struct LossBasedControl {
    bandwidth_estimate: f32
}

impl LossBasedControl {
    pub fn new(init_bandwidth: f32) -> Self {
        Self { bandwidth_estimate: init_bandwidth }
    }

    pub fn update(&mut self, received: u32, lost: u32) {
        let total = received + lost;
        let fraction_lost = lost as f32 / total as f32;
        if fraction_lost < 0.02 {
            self.bandwidth_estimate *= 1.05;
        } else if fraction_lost > 0.10 {
            self.bandwidth_estimate *= 1.0 - 0.5 * fraction_lost;
        }
    }

    pub fn get_estimate(&self) -> f32 {
        self.bandwidth_estimate
    }
}