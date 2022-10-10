mod delay_based;
mod loss_based;

use webrtc::{
    rtcp::{
        receiver_report::ReceiverReport,
        transport_feedbacks::transport_layer_cc::{
            PacketStatusChunk, SymbolTypeTcc, TransportLayerCc,
        },
    },
    rtp::extension::abs_send_time_extension::unix2ntp,
};

use self::{delay_based::DelayBasedBandwidthEstimator, loss_based::LossBasedBandwidthEstimator};
use super::sync::{TwccBandwidthEstimate, TwccSendInfo, TwccTime};
use std::time::{Instant, SystemTime};

pub struct TwccBandwidthEstimator {
    estimate: TwccBandwidthEstimate,
    delay_based_estimator: DelayBasedBandwidthEstimator,
    loss_based_estimator: LossBasedBandwidthEstimator,
    received: u32,
    lost: u32,
}

impl TwccBandwidthEstimator {
    pub fn new(estimate: TwccBandwidthEstimate) -> TwccBandwidthEstimator {
        TwccBandwidthEstimator {
            estimate,
            delay_based_estimator: DelayBasedBandwidthEstimator::new(),
            loss_based_estimator: LossBasedBandwidthEstimator::new(),
            received: 0,
            lost: 0,
        }
    }

    pub fn estimate(&mut self, now: Instant) {
        let current_bandwidth = self.estimate.get_estimate() as f32;
        let a = self.delay_based_estimator.estimate(current_bandwidth, now);
        let b = self
            .loss_based_estimator
            .estimate(current_bandwidth, self.received, self.lost);
        self.estimate.set_estimate(f32::min(a, b) as u64);

        self.received = 0;
        self.lost = 0;
    }

    pub fn process_feedback(&mut self, tcc: &TransportLayerCc, send_info: &TwccSendInfo) {
        let mut sequence_number = tcc.base_sequence_number;
        let mut arrival_time = TwccTime::extract_from_rtcp(tcc);

        let mut recv_deltas_iter = tcc.recv_deltas.iter();

        let mut with_packet_status = |status: &SymbolTypeTcc| {
            match status {
                SymbolTypeTcc::PacketNotReceived => {
                    self.lost += 1;
                }
                SymbolTypeTcc::PacketReceivedWithoutDelta => {
                    self.received += 1;
                }
                _ => {
                    self.received += 1;
                    if let Some(recv_delta) = recv_deltas_iter.next() {
                        arrival_time = TwccTime::from_recv_delta(arrival_time, recv_delta);

                        let (departure_time, packet_size) = send_info.load(sequence_number);

                        self.delay_based_estimator.process_packet(
                            departure_time,
                            arrival_time,
                            packet_size,
                        );
                    }
                }
            }
            sequence_number = sequence_number.wrapping_add(1);
        };

        for chunk in tcc.packet_chunks.iter() {
            match chunk {
                PacketStatusChunk::RunLengthChunk(chunk) => {
                    for _ in 0..chunk.run_length {
                        with_packet_status(&chunk.packet_status_symbol);
                    }
                }
                PacketStatusChunk::StatusVectorChunk(chunk) => {
                    for status in chunk.symbol_list.iter() {
                        with_packet_status(status);
                    }
                }
            }
        }
    }

    pub fn update_rtt(&mut self, rr: &ReceiverReport) {
        let now = (unix2ntp(SystemTime::now()) >> 16) as u32;

        for recp in &rr.reports {
            let rtt_ms = calculate_rtt_ms(now, recp.delay, recp.last_sender_report);
            self.delay_based_estimator.update_rtt(rtt_ms);
        }
    }
}

fn calculate_rtt_ms(now: u32, delay: u32, last_sender_report: u32) -> f32 {
    let rtt = now - delay - last_sender_report;
    let rtt_seconds = rtt >> 16;
    let rtt_fraction = (rtt & (u16::MAX as u32)) as f32 / (u16::MAX as u32) as f32;
    rtt_seconds as f32 * 1000.0 + (rtt_fraction as f32) * 1000.0
}