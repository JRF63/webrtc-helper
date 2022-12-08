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
    ) -> Box<dyn Encoder> {
        if self.is_codec_supported(codec_params) {
            const RTP_OUTBOUND_MTU: usize = 1200;

            let encoder = MockEncoder {
                payload_type: codec_params.payload_type,
                ssrc: context.ssrc(),
                sequencer: Box::new(new_random_sequencer()),
                start: (Instant::now(), 0),
                clock_rate: 90000,
                last_update: Instant::now(),
                mtu: RTP_OUTBOUND_MTU,
                data_rate: DataRate::default(),
                rate_change_counter: 0,
            };
            Box::new(encoder)
        } else {
            panic!("Codec not supported");
        }
    }
}

pub struct MockEncoder {
    payload_type: u8,
    ssrc: u32,
    sequencer: Box<dyn Sequencer + Send + Sync>,
    start: (Instant, u32),
    clock_rate: u64,
    last_update: Instant,
    mtu: usize,
    data_rate: DataRate,
    rate_change_counter: u64,
}

impl Encoder for MockEncoder {
    fn packets(&mut self) -> Box<[Packet]> {
        const FRAME_INTERVAL_60_FPS: Duration = Duration::from_micros(16_667);

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        if elapsed < FRAME_INTERVAL_60_FPS {
            return Box::new([]);
        }

        self.last_update = now;

        let payload_total_bytes = self.data_rate.bytes_per_sec_f64() * elapsed.as_secs_f64();
        let payloads = dummy_payloads(self.mtu - 12, payload_total_bytes.floor() as u64);

        let duration = self.last_update.duration_since(self.start.0).as_millis() as u64;
        let ticks = duration.wrapping_mul(self.clock_rate).wrapping_div(1000);
        let timestamp = self.start.1.wrapping_add(ticks as u32);

        let mut packets = Vec::with_capacity(payloads.len());

        for payload in payloads {
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

        let mut last_packet = packets.last_mut().unwrap();
        last_packet.header.marker = true;

        packets.into_boxed_slice()
    }

    fn set_mtu(&mut self, mtu: usize) {
        self.mtu = mtu;
    }

    fn set_data_rate(&mut self, data_rate: DataRate) {
        self.rate_change_counter += 1;
        self.data_rate = data_rate;
        if self.rate_change_counter % 20 == 0 {
            let bitrate = self.data_rate.bytes_per_sec_f64();
            println!("<: {bitrate:.3}");
        }
    }
}

fn dummy_payloads(mtu: usize, payload_total_bytes: u64) -> Vec<Bytes> {
    let data = vec![42u8; mtu];
    let mut payloads = Vec::new();
    let mut remaining_bytes = payload_total_bytes;
    loop {
        if let Some(r) = remaining_bytes.checked_sub(mtu as u64) {
            payloads.push(Bytes::copy_from_slice(&data));
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
