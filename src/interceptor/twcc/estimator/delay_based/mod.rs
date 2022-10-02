mod history;
mod overuse_detector;
mod packet_group;

use self::{
    history::History,
    overuse_detector::{DelayDetector, NetworkState},
    packet_group::PacketGroup,
};
use super::TwccTime;
use std::{collections::VecDeque, time::Instant};

const BURST_TIME_US: i64 = 5000;

// Should be within 500 - 1000 ms if packets are grouped by 5 ms burst time
const WINDOW_SIZE: usize = 100;

const ESTIMATOR_REACTION_TIME_MS: f32 = 100.0;

const STATE_NOISE_COVARIANCE: f32 = 10e-3;

const INITIAL_SYSTEM_ERROR_COVARIANCE: f32 = 0.1;

// Midway between the recommended value of 0.001 - 0.1
const CHI: f32 = 0.01;

const INITIAL_DELAY_THRESHOLD_US: f32 = 12500.0;

const OVERUSE_TIME_THRESHOLD_MS: u128 = 10;

const K_U: f32 = 0.01;

const K_D: f32 = 0.00018;

const DECREASE_RATE_FACTOR: f32 = 0.85;

pub struct DelayBasedBandwidthEstimator {
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    history: History,
    delay_detector: Option<DelayDetector>,
    last_update: Option<Instant>,
    network_state: NetworkState,
    rtt_ms: f32,
}

impl DelayBasedBandwidthEstimator {
    pub fn new() -> DelayBasedBandwidthEstimator {
        DelayBasedBandwidthEstimator {
            prev_group: None,
            curr_group: None,
            history: History::new(),
            delay_detector: None,
            last_update: None,
            network_state: NetworkState::Normal,
            rtt_ms: 0.0, // TODO
        }
    }

    pub fn process_packet(
        &mut self,
        departure_time: TwccTime,
        arrival_time: TwccTime,
        packet_size: u64,
        now: Instant,
    ) {
        let mut new_packet_group = false;

        if let Some(curr_group) = &mut self.curr_group {
            // Ignore reordered packets
            if departure_time >= curr_group.earliest_departure_time_us {
                if curr_group.belongs_to_group(departure_time, arrival_time) {
                    curr_group.add_packet(departure_time, arrival_time, packet_size);
                } else {
                    new_packet_group = true;
                }
            }
        } else {
            new_packet_group = true;
        }

        if new_packet_group {
            self.curr_group_completed(now);

            std::mem::swap(&mut self.prev_group, &mut self.curr_group);
            self.curr_group = Some(PacketGroup::new(departure_time, arrival_time, packet_size));
        }
    }

    fn curr_group_completed(&mut self, now: Instant) -> Option<f32> {
        if let (Some(curr_group), Some(prev_group)) = (&self.curr_group, &self.prev_group) {
            let interdeparture_time = curr_group.interdeparture_time(prev_group);
            self.history.add_group(curr_group, interdeparture_time);

            let interarrival_time = curr_group.interarrival_time(prev_group);
            let intergroup_delay = interarrival_time - interdeparture_time;

            if let Some(delay_detector) = &mut self.delay_detector {
                if let Some(min_send_interval) = self.history.smallest_send_interval() {
                    self.network_state = delay_detector.update(
                        intergroup_delay,
                        min_send_interval,
                        interarrival_time,
                        now,
                    );
                }
            } else {
                self.delay_detector = Some(DelayDetector::new(intergroup_delay));
            }
        }
        None
    }

    pub fn estimate(&mut self, current_bandwidth: f32, now: Instant) -> f32 {
        let mut bandwidth_estimate = match self.network_state {
            overuse_detector::NetworkState::Underuse => self.decrease(current_bandwidth),
            overuse_detector::NetworkState::Normal => current_bandwidth,
            overuse_detector::NetworkState::Overuse => {
                // TODO: ave and stddev
                let estimate_converged = false;

                if estimate_converged {
                    self.additive_increase(current_bandwidth, now)
                } else {
                    self.multiplicative_increase(current_bandwidth, now)
                }
            }
        };
        self.last_update = Some(now);

        if let Some(received_bandwidth) = self.history.received_bandwidth_bytes_per_sec() {
            if bandwidth_estimate >= 1.5 * received_bandwidth {
                bandwidth_estimate = received_bandwidth;
            }
        }

        return bandwidth_estimate;
    }

    fn time_since_last_update(&self, now: Instant) -> f32 {
        let time_since_last_update_ms = self
            .last_update
            .map(|t| now.duration_since(t).as_millis() as f32)
            .unwrap_or((BURST_TIME_US / 1000) as f32);
        time_since_last_update_ms
    }

    fn multiplicative_increase(&self, current_bandwidth: f32, now: Instant) -> f32 {
        let eta = 1.08f32.powf(f32::min(1.0, self.time_since_last_update(now) / 1000.0));
        current_bandwidth * eta
    }

    fn additive_increase(&self, current_bandwidth: f32, now: Instant) -> f32 {
        let response_time_ms = ESTIMATOR_REACTION_TIME_MS + self.rtt_ms;

        let alpha = 0.5 * f32::min(1.0, self.time_since_last_update(now) / response_time_ms);
        // Bandwidth is in bytes/s hence the 1000 in the congestion control draft was divided by 8
        current_bandwidth + f32::max(125.0, alpha * self.history.average_packet_size_bytes())
    }

    fn decrease(&self, current_bandwidth: f32) -> f32 {
        current_bandwidth * DECREASE_RATE_FACTOR
    }
}
