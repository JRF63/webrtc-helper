mod delay_based;
mod loss_based;

use self::{delay_based::DelayBasedBandwidthEstimator, loss_based::LossBasedBandwidthEstimator};
use super::sync::{TwccBandwidthEstimate, TwccTime};
use std::time::Instant;

pub struct TwccBandwidthEstimator {
    estimate: TwccBandwidthEstimate,
    delay_based_estimator: DelayBasedBandwidthEstimator,
    loss_based_estimator: LossBasedBandwidthEstimator,
}

impl TwccBandwidthEstimator {
    pub fn new(estimate: TwccBandwidthEstimate) -> TwccBandwidthEstimator {
        TwccBandwidthEstimator {
            estimate,
            delay_based_estimator: DelayBasedBandwidthEstimator::new(),
            loss_based_estimator: LossBasedBandwidthEstimator::new(),
        }
    }

    pub fn estimate(&mut self, received: u32, lost: u32, now: Instant) {
        let current_bandwidth = self.estimate.get_estimate() as f32;
        let a = self.delay_based_estimator.estimate(current_bandwidth, now);
        let b = self
            .loss_based_estimator
            .estimate(current_bandwidth, received, lost);
        self.estimate.set_estimate(f32::min(a, b) as u64);
    }

    pub fn process_packet_feedback(
        &mut self,
        departure_time: TwccTime,
        arrival_time: TwccTime,
        packet_size: u64,
        now: Instant,
    ) {
        self.delay_based_estimator.process_packet_feedback(
            departure_time,
            arrival_time,
            packet_size,
            now,
        );
    }
}
