use super::TwccTime;

#[derive(Clone, Copy)]
struct PacketGroup {
    earliest_departure_time_us: TwccTime,
    departure_time_us: TwccTime,
    arrival_time_us: TwccTime,
}

pub struct DelayBasedControl {
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    bandwidth_estimate: f32,
    time_since_last_update_ms: f32,
    rtt_ms: f32,
}

const BURST_TIME_US: i64 = 5000;
const BITRATE_TIME_WINDOW: f32 = 0.5;
// Decrease rate factor
const BETA: f32 = 0.85;

fn belongs_to_current_group(
    curr_group: &PacketGroup,
    departure_time_us: TwccTime,
    arrival_time_us: TwccTime,
) -> bool {
    if departure_time_us.small_delta_sub(curr_group.earliest_departure_time_us) > BURST_TIME_US {
        return true;
    }

    let inter_arrival_time = arrival_time_us.small_delta_sub(curr_group.arrival_time_us);
    let inter_departure_time = departure_time_us.small_delta_sub(curr_group.departure_time_us);
    let inter_group_delay = inter_arrival_time - inter_departure_time;
    if inter_arrival_time < BURST_TIME_US && inter_group_delay < 0 {
        return true;
    }

    false
}

impl DelayBasedControl {
    pub fn add_recv_delta(
        &mut self,
        departure_time_us: TwccTime,
        arrival_time_us: TwccTime,
    ) -> Option<f32> {
        if let Some(curr_group) = &mut self.curr_group {
            if departure_time_us < curr_group.earliest_departure_time_us {
                // TODO reordered
            }

            if belongs_to_current_group(curr_group, departure_time_us, arrival_time_us) {
                if departure_time_us > curr_group.departure_time_us {
                    curr_group.departure_time_us = departure_time_us;
                }
                if arrival_time_us > curr_group.arrival_time_us {
                    curr_group.arrival_time_us = arrival_time_us;
                }
            } else {
                // TODO new group
                if let Some(prev_group) = &self.prev_group {}

                self.prev_group = Some(*curr_group);
                self.curr_group = Some(PacketGroup {
                    earliest_departure_time_us: departure_time_us,
                    departure_time_us,
                    arrival_time_us,
                });
            }
        } else {
            self.curr_group = Some(PacketGroup {
                earliest_departure_time_us: departure_time_us,
                departure_time_us,
                arrival_time_us,
            });
        }

        None
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
