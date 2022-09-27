use super::TwccTime;
use std::collections::VecDeque;

const BURST_TIME_US: i64 = 5000;
const DECREASE_RATE_FACTOR: f32 = 0.85;
// Should be within 500 - 1000 ms if packets are grouped by 5 ms burst time
const WINDOW_SIZE: usize = 100;
const ESTIMATOR_REACTION_TIME_MS: f32 = 100.0;

#[derive(Clone, Copy)]
struct PacketGroup {
    earliest_departure_time_us: TwccTime,
    departure_time_us: TwccTime,
    arrival_time_us: TwccTime,
    size_bytes: u64,
    num_packets: u64,
}

impl PacketGroup {
    fn new(
        departure_time_us: TwccTime,
        arrival_time_us: TwccTime,
        packet_size: u64,
    ) -> PacketGroup {
        PacketGroup {
            earliest_departure_time_us: departure_time_us,
            departure_time_us,
            arrival_time_us,
            size_bytes: packet_size,
            num_packets: 1,
        }
    }

    fn belongs_to_group(&self, departure_time_us: TwccTime, arrival_time_us: TwccTime) -> bool {
        if departure_time_us.small_delta_sub(self.earliest_departure_time_us) > BURST_TIME_US {
            return true;
        }

        let inter_arrival_time = arrival_time_us.small_delta_sub(self.arrival_time_us);
        let inter_departure_time = departure_time_us.small_delta_sub(self.departure_time_us);
        let inter_group_delay = inter_arrival_time - inter_departure_time;
        if inter_arrival_time < BURST_TIME_US && inter_group_delay < 0 {
            return true;
        }

        false
    }
}

struct WindowData {
    id: u32,
    arrival_time_us: TwccTime,
    size_bytes: u64,
    num_packets: u64,
}

struct InterdepartureTimeData {
    id: u32,
    inter_depature_time: i64,
}

#[derive(Default)]
struct History {
    data: VecDeque<WindowData>,
    ascending_minima: VecDeque<InterdepartureTimeData>,
    total_packet_size_bytes: u64,
    num_packets: u64,
}

impl History {
    fn add_group(&mut self, curr_group: &PacketGroup, prev_group: &PacketGroup) {
        let window_data = WindowData {
            id: self
                .data
                .back()
                .map(|x| x.id.wrapping_add(1))
                .unwrap_or_default(),
            arrival_time_us: curr_group.arrival_time_us,
            size_bytes: curr_group.size_bytes,
            num_packets: curr_group.num_packets,
        };
        let idt_data = InterdepartureTimeData {
            id: window_data.id,
            inter_depature_time: curr_group
                .departure_time_us
                .small_delta_sub(prev_group.departure_time_us),
        };

        self.total_packet_size_bytes += window_data.size_bytes;
        self.num_packets += window_data.num_packets;
        self.data.push_back(window_data);

        if self.data.len() < WINDOW_SIZE {
            // Temporarily stores the inter-departure times
            self.ascending_minima.push_back(idt_data);
        } else if self.data.len() == WINDOW_SIZE {
            self.build_ascending_minima();
        } else {
            let to_remove = self.data.pop_front().unwrap();
            self.total_packet_size_bytes -= to_remove.size_bytes;
            self.num_packets -= to_remove.num_packets;
            self.maintain_ascending_minima(to_remove.id, idt_data);
        }
    }

    fn build_ascending_minima(&mut self) {
        let mut start = 0;
        let mut tmp = VecDeque::new();

        while let Some((index, minimum)) = self
            .ascending_minima
            .iter()
            .enumerate()
            .skip(start)
            .reduce(|accum, item| {
                if item.1.inter_depature_time < accum.1.inter_depature_time {
                    item
                } else {
                    accum
                }
            })
        {
            tmp.push_back(InterdepartureTimeData {
                id: minimum.id,
                inter_depature_time: minimum.inter_depature_time,
            });
            start = index + 1;
        }
        self.ascending_minima = tmp;
    }

    fn maintain_ascending_minima(&mut self, id_to_remove: u32, item: InterdepartureTimeData) {
        while self.ascending_minima.back().unwrap().inter_depature_time > item.inter_depature_time {
            self.ascending_minima.pop_back();
        }
        self.ascending_minima.push_back(item);
        if self.ascending_minima.front().unwrap().id == id_to_remove {
            self.ascending_minima.pop_front();
        }
    }

    fn average_packet_size_bytes(&self) -> f32 {
        self.total_packet_size_bytes as f32 / self.num_packets as f32
    }

    fn received_bandwidth_bytes_per_sec(&self) -> Option<f32> {
        let timespan = self
            .data
            .back()?
            .arrival_time_us
            .small_delta_sub(self.data.front()?.arrival_time_us);
        Some(self.total_packet_size_bytes as f32 / timespan as f32)
    }
}

pub struct DelayBasedBandwidthEstimator {
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    history: History,
    bandwidth_estimate: f32,
    time_since_last_update_ms: f32,
    rtt_ms: f32,
}

impl DelayBasedBandwidthEstimator {
    pub fn process_feedback(
        &mut self,
        departure_time_us: TwccTime,
        arrival_time_us: TwccTime,
        packet_size: u64,
    ) -> Option<f32> {
        if let Some(curr_group) = &mut self.curr_group {
            // Ignore reordered packets
            if departure_time_us >= curr_group.earliest_departure_time_us {
                if curr_group.belongs_to_group(departure_time_us, arrival_time_us) {
                    curr_group.size_bytes += packet_size;
                    curr_group.num_packets += 1;

                    if departure_time_us > curr_group.departure_time_us {
                        curr_group.departure_time_us = departure_time_us;
                    }
                    if arrival_time_us > curr_group.arrival_time_us {
                        curr_group.arrival_time_us = arrival_time_us;
                    }
                } else {
                    if let Some(prev_group) = &self.prev_group {
                        self.history.add_group(curr_group, prev_group);

                        // TODO: estimate

                        if let Some(received_bandwidth) =
                            self.history.received_bandwidth_bytes_per_sec()
                        {
                            if self.bandwidth_estimate >= 1.5 * received_bandwidth {
                                self.bandwidth_estimate = received_bandwidth;
                            }
                        }
                    }

                    self.prev_group = Some(*curr_group);
                    self.curr_group = Some(PacketGroup::new(
                        departure_time_us,
                        arrival_time_us,
                        packet_size,
                    ));
                }
            }
        } else {
            self.curr_group = Some(PacketGroup::new(
                departure_time_us,
                arrival_time_us,
                packet_size,
            ));
        }

        None
    }

    fn multiplicative_increase(&mut self) {
        let eta = 1.08f32.powf(1.0f32.min(self.time_since_last_update_ms / 1000.0));
        self.bandwidth_estimate *= eta;
    }

    fn additive_increase(&mut self) {
        let response_time_ms = ESTIMATOR_REACTION_TIME_MS + self.rtt_ms;
        let alpha = 0.5 * 1.0f32.min(self.time_since_last_update_ms / response_time_ms);
        self.bandwidth_estimate += 125f32.max(alpha * self.history.average_packet_size_bytes());
    }

    fn decrease(&mut self) {
        self.bandwidth_estimate *= DECREASE_RATE_FACTOR;
    }
}
