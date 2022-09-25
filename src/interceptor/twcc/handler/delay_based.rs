use super::TwccDataMap;
use std::time::{Instant, Duration};
use webrtc::rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;

#[derive(Clone, Copy)]
struct PacketGroup {
    group_idx: i64,
    departure_time_us: i64,
    arrival_time_us: i64,
}

pub struct DelayBasedControl {
    map: TwccDataMap,
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    bandwidth_estimate: f32,
    time_since_last_update_ms: f32,
    rtt_ms: f32,
}

const WRAP_AROUND_PERIOD: i64 = 1073741824000;
const MAX_GROUP_INDEX: i64 = 214748352; // (2 ^ 24) * 64000
const BURST_TIME_US: i64 = 5000;
const BITRATE_TIME_WINDOW: f32 = 0.5;
// Decrease rate factor
const BETA: f32 = 0.85;

impl DelayBasedControl {
    // Doesn't properly handle wrap around
    pub fn update(&mut self, sequence_number: u16, arrival_time_us: i64) {
        let group_idx = arrival_time_us / BURST_TIME_US;
        match &mut self.curr_group {
            Some(curr_group) => {
                if group_idx != curr_group.group_idx {
                    self.prev_group = Some(*curr_group);
                    curr_group.group_idx = group_idx;
                    curr_group.departure_time_us = self.map[sequence_number].load();
                    curr_group.arrival_time_us = arrival_time_us;
                }
            }
            None => {

            }
        }
    }

    pub fn get_estimate(&self) -> f32 {
        self.bandwidth_estimate
    }

    fn multiplicative_increase(&mut self) {
        let eta = 1.08f32.powf(1.0f32.min(self.time_since_last_update_ms / 1000.0));
        self.bandwidth_estimate *= eta;
    }

    fn additive_increase(&mut self) {
        let response_time_ms = 100.0 + self.rtt_ms;
        let alpha = 0.5 * 1.0f32.min(self.time_since_last_update_ms / response_time_ms);

        // TODO: Use a better estimate
        let expected_packet_size_bits = {
            let bits_per_frame = self.bandwidth_estimate / 60.0;
            let packets_per_frame = (bits_per_frame / (1200.0 * 8.0)).ceil();
            let avg_packet_size_bits = bits_per_frame / packets_per_frame;
            avg_packet_size_bits
        };

        self.bandwidth_estimate += 1000f32.max(alpha * expected_packet_size_bits);
    }

    fn decrease(&mut self) {
        self.bandwidth_estimate *= BETA;
    }
}
