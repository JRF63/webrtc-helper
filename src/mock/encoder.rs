use crate::{
    codecs::Codec,
    encoder::{Encoder, EncoderBuilder},
    util::data_rate::{DataRate, TwccBandwidthEstimate},
};
use bytes::Bytes;
use std::time::{Duration, Instant};
use webrtc::{
    rtp::{
        header::Header,
        packet::Packet,
        sequence::{new_random_sequencer, Sequencer},
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecParameters,
    track::track_local::TrackLocalContext,
};

pub struct MockEncoderBuilder {
    codecs: Vec<Codec>,
}

impl MockEncoderBuilder {
    pub fn new() -> Self {
        Self {
            codecs: vec![super::codec::mock_codec()],
        }
    }
}

impl EncoderBuilder for MockEncoderBuilder {
    fn id(&self) -> &str {
        "mock-video"
    }

    fn stream_id(&self) -> &str {
        "mock-webrtc"
    }

    fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }

    fn build(
        self: Box<Self>,
        codec_params: &RTCRtpCodecParameters,
        context: &TrackLocalContext,
        bandwidth_estimate: TwccBandwidthEstimate
    ) -> Box<dyn Encoder> {
        if self.is_codec_supported(codec_params) {
            let data_rate = *bandwidth_estimate.borrow();
            let encoder = MockEncoder {
                sequencer: Box::new(new_random_sequencer()),
                start: (Instant::now(), 0),
                clock_rate: 90000,
                bandwidth_estimate,
                data_rate,
                last_update: Instant::now(),
                rate_change_counter: 0,
                packets: dummy_packets(codec_params.payload_type, context.ssrc())
            };
            Box::new(encoder)
        } else {
            panic!("Codec not supported");
        }
    }
}

fn dummy_packets(payload_type: u8, ssrc: u32) -> Vec<Packet> {
    // For ~100 MBps
    const NUM_PACKETS: usize = 1_666_667;
    let mut packets = Vec::with_capacity(NUM_PACKETS);

    let data = [42u8; 1200 - 12];
    
    for _ in 0..NUM_PACKETS {
        let header = Header {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type,
            sequence_number: 0,
            timestamp: 0,
            ssrc,
            ..Default::default()
        };
        let payload = Bytes::copy_from_slice(&data);
        let packet = Packet { header, payload };
        packets.push(packet);
    }

    packets
}

pub struct MockEncoder {
    sequencer: Box<dyn Sequencer + Send + Sync>,
    start: (Instant, u32),
    clock_rate: u64,
    bandwidth_estimate: TwccBandwidthEstimate,
    data_rate: DataRate,
    last_update: Instant,
    rate_change_counter: u64,
    packets: Vec<Packet>,
}

impl Encoder for MockEncoder {
    fn packets(&mut self) -> &[Packet] {
        const FRAME_INTERVAL_60_FPS: Duration = Duration::from_micros(16_667);
        const MTU: usize = 1200;

        if let Ok(true) = self.bandwidth_estimate.has_changed() {
            self.data_rate = *self.bandwidth_estimate.borrow();
        }

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        if elapsed < FRAME_INTERVAL_60_FPS {
            return &[];
        }

        if self.rate_change_counter % 180 == 0 {
            let send_bitrate = self.data_rate.bytes_per_sec_f64();
            println!("<: {send_bitrate:.3}");
        }
        self.rate_change_counter = self.rate_change_counter.wrapping_add(1);

        self.last_update = now;

        let payload_total_bytes = self.data_rate.bytes_per_sec_f64() * elapsed.as_secs_f64();
        let num_packets = (payload_total_bytes as usize) / (MTU - 12);
        if num_packets == 0 {
            return &[];
        }

        let duration = self.last_update.duration_since(self.start.0).as_millis() as u64;
        let ticks = duration.wrapping_mul(self.clock_rate).wrapping_div(1000);
        let timestamp = self.start.1.wrapping_add(ticks as u32);

        for packet in &mut self.packets[..num_packets] {
            packet.header.sequence_number = self.sequencer.next_sequence_number();
            packet.header.timestamp = timestamp;
        }

        self.packets[num_packets - 1].header.marker = true;
        &self.packets[..num_packets]
    }
}
