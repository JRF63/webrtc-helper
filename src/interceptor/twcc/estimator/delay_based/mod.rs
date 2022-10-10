mod history;
mod overuse_detector;
mod packet_group;

use self::{
    history::History,
    overuse_detector::{DelayDetector, NetworkCondition},
    packet_group::PacketGroup,
};
use super::TwccTime;
use std::{collections::VecDeque, time::Instant};

const BURST_TIME_US: i64 = 5000;

// Should be within 500 - 1000 ms if packets are grouped by 5 ms burst time
const WINDOW_SIZE: u32 = 100;

const ESTIMATOR_REACTION_TIME_MS: f32 = 100.0;

const STATE_NOISE_COVARIANCE: f32 = 10e-3;

const INITIAL_SYSTEM_ERROR_COVARIANCE: f32 = 0.1;

// Midway between the recommended value of 0.001 - 0.1
const CHI: f32 = 0.01;

const INITIAL_DELAY_THRESHOLD_US: f32 = 12500.0;

const OVERUSE_TIME_THRESHOLD_US: i64 = 10000;

const K_U: f32 = 0.01;

const K_D: f32 = 0.00018;

const DECREASE_RATE_FACTOR: f32 = 0.85;

// Exponential moving average smoothing factor
const ALPHA: f32 = 0.95;

struct IncomingBitrateEstimate {
    mean: f32,
    variance: f32,
    close_to_ave: bool,
}

impl IncomingBitrateEstimate {
    fn new() -> IncomingBitrateEstimate {
        IncomingBitrateEstimate {
            mean: 0.0,
            variance: 0.0,
            close_to_ave: false,
        }
    }

    fn update(&mut self, bytes_per_sec: f32) {
        let stddev = self.variance.sqrt();
        if (bytes_per_sec - self.mean).abs() < 3.0 * stddev {
            self.close_to_ave = true;
        } else {
            // Reset the average and go to multiplicative increase
            self.mean = bytes_per_sec;
            self.variance = 0.0;
            self.close_to_ave = false;
            return;
        }

        // Exponentially-weighted mean and variance calculation from:
        // https://web.archive.org/web/20181222175223/http://people.ds.cam.ac.uk/fanf2/hermes/doc/antiforgery/stats.pdf
        let diff = bytes_per_sec - self.mean;
        let incr = ALPHA * diff;
        self.mean = self.mean + incr;
        self.variance = (1.0 - ALPHA) * (self.variance + diff * incr);
    }
}

pub struct DelayBasedBandwidthEstimator {
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    history: History,
    incoming_bitrate_estimate: IncomingBitrateEstimate,
    delay_detector: Option<DelayDetector>,
    last_update: Option<Instant>,
    network_condition: NetworkCondition,
    rtt_ms: f32,
}

impl DelayBasedBandwidthEstimator {
    pub fn new() -> DelayBasedBandwidthEstimator {
        DelayBasedBandwidthEstimator {
            prev_group: None,
            curr_group: None,
            history: History::new(),
            incoming_bitrate_estimate: IncomingBitrateEstimate::new(),
            delay_detector: None,
            last_update: None,
            network_condition: NetworkCondition::Normal,
            rtt_ms: 0.0, // TODO
        }
    }

    pub fn process_packet(
        &mut self,
        departure_time: TwccTime,
        arrival_time: TwccTime,
        packet_size: u64,
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
            self.curr_group_completed(arrival_time);

            std::mem::swap(&mut self.prev_group, &mut self.curr_group);
            self.curr_group = Some(PacketGroup::new(departure_time, arrival_time, packet_size));
        }
    }

    pub fn update_rtt(&mut self, rtt_ms: f32) {
        self.rtt_ms = rtt_ms;
    }

    fn curr_group_completed(&mut self, arrival_time: TwccTime) {
        if let (Some(curr_group), Some(prev_group)) = (&self.curr_group, &self.prev_group) {
            // Inter-departure time should be >= 0 since we ignore reordered packets
            let interdeparture_time = curr_group.interdeparture_time(prev_group);
            let interarrival_time = curr_group.interarrival_time(prev_group);
            let intergroup_delay = interarrival_time - interdeparture_time;

            self.history.add_group(curr_group, interdeparture_time);

            if let Some(delay_detector) = &mut self.delay_detector {
                if let Some(&min_send_interval) = self.history.smallest_send_interval() {
                    self.network_condition = delay_detector.detect_network_condition(
                        intergroup_delay,
                        min_send_interval,
                        interarrival_time,
                        arrival_time,
                    );

                    match self.network_condition {
                        NetworkCondition::Overuse => {
                            if let Some(bytes_per_sec) =
                                self.history.received_bandwidth_bytes_per_sec()
                            {
                                self.incoming_bitrate_estimate.update(bytes_per_sec)
                            }
                        }
                        _ => (),
                    }
                }
            } else {
                self.delay_detector = Some(DelayDetector::new(intergroup_delay));
            }
        }
    }

    pub fn estimate(&mut self, current_bandwidth: f32, now: Instant) -> f32 {
        let mut bandwidth_estimate = match self.network_condition {
            overuse_detector::NetworkCondition::Underuse => {
                let time_since_last_update_ms = self.time_since_last_update(now);

                if self.incoming_bitrate_estimate.close_to_ave {
                    bandwidth_additive_increase(
                        current_bandwidth,
                        time_since_last_update_ms,
                        self.rtt_ms,
                        self.history.average_packet_size_bytes(),
                    )
                } else {
                    bandwidth_multiplicative_increase(current_bandwidth, time_since_last_update_ms)
                }
            }
            overuse_detector::NetworkCondition::Normal => current_bandwidth,
            overuse_detector::NetworkCondition::Overuse => bandwidth_decrease(current_bandwidth),
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
        let millis = self
            .last_update
            .map(|t| now.duration_since(t).as_millis() as f32)
            .unwrap_or((BURST_TIME_US / 1000) as f32);
        millis
    }
}

fn bandwidth_additive_increase(
    current_bandwidth: f32,
    time_since_last_update_ms: f32,
    rtt_ms: f32,
    ave_packet_size_bytes: f32,
) -> f32 {
    let response_time_ms = ESTIMATOR_REACTION_TIME_MS + rtt_ms;

    let alpha = 0.5 * f32::min(1.0, time_since_last_update_ms / response_time_ms);
    // Bandwidth is in bytes/s hence the 1000 in the congestion control draft was divided by 8
    current_bandwidth + f32::max(125.0, alpha * ave_packet_size_bytes)
}

fn bandwidth_multiplicative_increase(
    current_bandwidth: f32,
    time_since_last_update_ms: f32,
) -> f32 {
    let eta = 1.08f32.powf(f32::min(1.0, time_since_last_update_ms / 1000.0));
    current_bandwidth * eta
}

fn bandwidth_decrease(current_bandwidth: f32) -> f32 {
    current_bandwidth * DECREASE_RATE_FACTOR
}
