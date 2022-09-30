use super::*;

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

pub struct History {
    data: VecDeque<WindowData>,
    ascending_minima: VecDeque<InterdepartureTimeData>,
    total_packet_size_bytes: u64,
    num_packets: u64,
}

impl History {
    pub fn new() -> History {
        History {
            data: VecDeque::new(),
            ascending_minima: VecDeque::new(),
            total_packet_size_bytes: 0,
            num_packets: 0,
        }
    }

    pub fn add_group(&mut self, curr_group: &PacketGroup, inter_depature_time: i64) {
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

    pub fn average_packet_size_bytes(&self) -> f32 {
        self.total_packet_size_bytes as f32 / self.num_packets as f32
    }

    pub fn received_bandwidth_bytes_per_sec(&self) -> Option<f32> {
        let timespan = self
            .data
            .back()?
            .arrival_time_us
            .small_delta_sub(self.data.front()?.arrival_time_us);
        Some(self.total_packet_size_bytes as f32 / timespan as f32)
    }

    /// Used for computing f_max in the arrival-time filter
    pub fn smallest_send_interval(&self) -> Option<i64> {
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
    }
}
