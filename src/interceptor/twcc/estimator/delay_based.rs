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

const DECREASE_RATE_FACTOR: f32 = 0.85;

#[derive(Clone)]
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

    fn inter_arrival_time(&self, other: &PacketGroup) -> i64 {
        self.arrival_time_us.small_delta_sub(other.arrival_time_us)
    }

    fn inter_departure_time(&self, other: &PacketGroup) -> i64 {
        self.departure_time_us
            .small_delta_sub(other.departure_time_us)
    }

    fn inter_group_delay(&self, other: &PacketGroup) -> i64 {
        self.inter_arrival_time(other) - self.inter_departure_time(other)
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

struct History {
    data: VecDeque<WindowData>,
    ascending_minima: VecDeque<InterdepartureTimeData>,
    total_packet_size_bytes: u64,
    num_packets: u64,
}

impl History {
    fn add_group(&mut self, curr_group: &PacketGroup, inter_depature_time: i64) {
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
            inter_depature_time,
        };

        self.total_packet_size_bytes += window_data.size_bytes;
        self.num_packets += window_data.num_packets;
        self.data.push_back(window_data);

        if self.data.len() < WINDOW_SIZE {
            // Temporarily stores the inter-departure times until len() == WINDOW_SIZE
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

    /// Used for computing f_max in the arrival-time filter
    fn smallest_send_interval(&self) -> Option<f32> {
        if self.data.len() < WINDOW_SIZE {
            self.ascending_minima
                .iter()
                .map(|x| x.inter_depature_time)
                .min()
        } else {
            self.ascending_minima
                .front()
                .map(|front| front.inter_depature_time)
        }
        .map(|min| min as f32)
    }
}

struct ArrivalTimeFilter {
    m_hat: f32,
    e: f32,
    var_v_hat: f32,
}

impl ArrivalTimeFilter {
    fn new(inter_group_delay: i64) -> ArrivalTimeFilter {
        ArrivalTimeFilter {
            m_hat: inter_group_delay as f32,
            e: INITIAL_SYSTEM_ERROR_COVARIANCE,
            var_v_hat: 0.0,
        }
    }

    fn update(&mut self, inter_group_delay: i64, interval: f32) {
        // This is different than in the draft since the interval used here is in microseconds
        let alpha = (1.0 - CHI).powf(30.0 * interval / 1e6);

        let q = STATE_NOISE_COVARIANCE;

        // m_hat, inter_group_delay, and z are in microseconds
        let z = inter_group_delay as f32 - self.m_hat;
        let z2 = z * z;

        self.var_v_hat = 1.0f32.max(alpha * self.var_v_hat + (1.0 - alpha) * z2);
        let k = (self.e + q) / (self.var_v_hat + (self.e + q));
        self.m_hat = self.m_hat + z * k;
        self.e = (1.0 - k) * (self.e + q);
    }
}

pub struct DelayBasedBandwidthEstimator {
    prev_group: Option<PacketGroup>,
    curr_group: Option<PacketGroup>,
    arrival_time_filter: Option<ArrivalTimeFilter>,
    history: History,
    bandwidth_estimate: f32,
    last_update: Option<Instant>,
    rtt_ms: f32,
}

impl DelayBasedBandwidthEstimator {
    pub fn process_feedback(
        &mut self,
        departure_time_us: TwccTime,
        arrival_time_us: TwccTime,
        packet_size: u64,
    ) -> Option<f32> {
        let mut new_packet_group = false;

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
                    new_packet_group = true;
                }
            }
        } else {
            new_packet_group = true;
        }

        if new_packet_group {
            self.estimate_bandwidth();

            std::mem::swap(&mut self.prev_group, &mut self.curr_group);
            self.curr_group = Some(PacketGroup::new(
                departure_time_us,
                arrival_time_us,
                packet_size,
            ));
        }

        None
    }

    fn estimate_bandwidth(&mut self) {
        if let (Some(curr_group), Some(prev_group)) = (&self.curr_group, &self.prev_group) {
            let inter_departure_time = curr_group.inter_departure_time(prev_group);
            self.history.add_group(curr_group, inter_departure_time);

            let inter_group_delay = curr_group.inter_group_delay(prev_group);
            if let Some(arrival_time_filter) = &mut self.arrival_time_filter {
                if let Some(interval) = self.history.smallest_send_interval() {
                    arrival_time_filter.update(inter_group_delay, interval);
                }
            } else {
                self.arrival_time_filter = Some(ArrivalTimeFilter::new(inter_group_delay));
            }

            // TODO: estimate bandwidth

            if let Some(received_bandwidth) = self.history.received_bandwidth_bytes_per_sec() {
                if self.bandwidth_estimate >= 1.5 * received_bandwidth {
                    self.bandwidth_estimate = received_bandwidth;
                }
            }
        }
    }

    fn time_since_last_update(&mut self) -> f32 {
        let now = Instant::now();
        let time_since_last_update_ms = self
            .last_update
            .map(|t| now.duration_since(t).as_millis() as f32)
            .unwrap_or((BURST_TIME_US / 1000) as f32);
        self.last_update = Some(now);
        time_since_last_update_ms
    }

    fn multiplicative_increase(&mut self) {
        let eta = 1.08f32.powf(1.0f32.min(self.time_since_last_update() / 1000.0));
        self.bandwidth_estimate *= eta;
    }

    fn additive_increase(&mut self) {
        let response_time_ms = ESTIMATOR_REACTION_TIME_MS + self.rtt_ms;

        let alpha = 0.5 * f32::min(1.0, self.time_since_last_update() / response_time_ms);
        // Bandwidth is in bytes hence the 1000 in the congestion control draft was divided by 8
        self.bandwidth_estimate +=
            f32::max(125.0, alpha * self.history.average_packet_size_bytes());
    }

    fn decrease(&mut self) {
        self.bandwidth_estimate *= DECREASE_RATE_FACTOR;
    }
}
