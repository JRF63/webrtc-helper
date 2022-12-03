use crate::{
    codecs::Codec,
    encoder::{Encoder, EncoderBuilder},
    util::data_rate::DataRate,
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
    ) -> Box<dyn Encoder> {
        if self.is_codec_supported(codec_params) {
            const RTP_OUTBOUND_MTU: usize = 1200;

            let encoder = MockEncoder {
                mtu: RTP_OUTBOUND_MTU,
                start: (Instant::now(), 0),
                clock_rate: 90000,
                time: Instant::now(),
                payload_type: codec_params.payload_type,
                sequencer: Box::new(new_random_sequencer()),
                ssrc: context.ssrc(),
                data_rate: DataRate::default(),
            };
            Box::new(encoder)
        } else {
            panic!("Codec not supported");
        }
    }
}

const FRAME_INTERVAL_60_FPS: Duration = Duration::from_micros(16_667);

pub struct MockEncoder {
    mtu: usize,
    start: (Instant, u32),
    clock_rate: u64,
    time: Instant,
    payload_type: u8,
    sequencer: Box<dyn Sequencer + Send + Sync>,
    ssrc: u32,
    data_rate: DataRate,
}

fn dummy_payloads(mtu: usize, data_rate: DataRate) -> Vec<Bytes> {
    let frame_interval = FRAME_INTERVAL_60_FPS.as_micros() as f64;
    let bytes_per_micros = data_rate.bytes_per_sec_f64() * 1e6;
    let bytes_per_interval = (bytes_per_micros / frame_interval) as u64;

    let data = vec![42u8; mtu];
    let mut payloads = Vec::new();
    let mut remaining_bytes = bytes_per_interval;
    loop {
        payloads.push(Bytes::copy_from_slice(&data));
        if let Some(r) = remaining_bytes.checked_sub(mtu as u64) {
            remaining_bytes = r;
        } else {
            break;
        }
    }

    if remaining_bytes > 0 {
        payloads.push(Bytes::copy_from_slice(&data[..(remaining_bytes as usize)]));
    }

    payloads
}

impl Encoder for MockEncoder {
    fn packets(&mut self) -> Box<[Packet]> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.time);
        if elapsed >= FRAME_INTERVAL_60_FPS {
            self.time = now;
            let duration = self.time.duration_since(self.start.0).as_millis() as u64;
            let ticks = duration.wrapping_mul(self.clock_rate).wrapping_div(1000);
            let timestamp = self.start.1.wrapping_add(ticks as u32);

            let mut payloads = dummy_payloads(self.mtu - 12, self.data_rate);
            let payloads_len = payloads.len();
            let mut packets = Vec::with_capacity(payloads_len);

            for payload in payloads.drain(..(payloads_len - 1)) {
                let header = Header {
                    version: 2,
                    padding: false,
                    extension: false,
                    marker: false,
                    payload_type: self.payload_type,
                    sequence_number: self.sequencer.next_sequence_number(),
                    timestamp,
                    ssrc: self.ssrc,
                    ..Default::default()
                };
                let packet = Packet { header, payload };
                packets.push(packet);
            }

            {
                let payload = payloads.pop().unwrap();
                let header = Header {
                    version: 2,
                    padding: false,
                    extension: false,
                    marker: true,
                    payload_type: self.payload_type,
                    sequence_number: self.sequencer.next_sequence_number(),
                    timestamp,
                    ssrc: self.ssrc,
                    ..Default::default()
                };
                let packet = Packet { header, payload };
                packets.push(packet);
            }

            packets.into_boxed_slice()
        } else {
            Box::new([])
        }
    }

    fn set_mtu(&mut self, mtu: usize) {
        self.mtu = mtu;
    }

    fn set_data_rate(&mut self, data_rate: DataRate) {
        self.data_rate = data_rate;
    }
}
