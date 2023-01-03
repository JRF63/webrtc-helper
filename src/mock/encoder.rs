use crate::{
    codecs::{Codec, CodecType},
    encoder::EncoderBuilder,
    peer::IceConnectionState,
    util::data_rate::{DataRate, TwccBandwidthEstimate},
};
use bytes::Bytes;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use webrtc::{
    ice_transport::ice_connection_state::RTCIceConnectionState,
    rtcp::{
        self,
        payload_feedbacks::{
            full_intra_request::FullIntraRequest, picture_loss_indication::PictureLossIndication,
        },
    },
    rtp::{
        header::Header,
        packet::Packet,
        sequence::{new_random_sequencer, Sequencer},
    },
    rtp_transceiver::{rtp_codec::RTCRtpCodecCapability, RTCRtpTransceiver},
    track::track_local::{track_local_static_rtp::TrackLocalStaticRTP, TrackLocalWriter},
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

    fn codec_type(&self) -> CodecType {
        CodecType::Video
    }

    fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }

    fn build(
        self: Box<Self>,
        rtp_track: Arc<TrackLocalStaticRTP>,
        transceiver: Arc<RTCRtpTransceiver>,
        mut ice_connection_state: IceConnectionState,
        bandwidth_estimate: TwccBandwidthEstimate,
        codec_capability: RTCRtpCodecCapability,
        ssrc: u32,
        payload_type: u8,
    ) {
        if !self.is_codec_supported(&codec_capability) {
            panic!("Codec not supported");
        }

        let handle = tokio::runtime::Handle::current();
        handle.spawn(async move {
            if let Some(sender) = transceiver.sender().await {
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1500];
                    while let Ok((n, _)) = sender.read(&mut buf).await {
                        let mut raw_data = &buf[..n];
                        if let Ok(packets) = rtcp::packet::unmarshal(&mut raw_data) {
                            for packet in packets {
                                let packet = packet.as_any();
                                if let Some(_pli) = packet.downcast_ref::<PictureLossIndication>() {
                                    // Unused
                                } else if let Some(_fir) = packet.downcast_ref::<FullIntraRequest>()
                                {
                                    // Unused
                                }
                            }
                        }
                    }
                });
            }
        });

        let stopped = Arc::new(AtomicBool::new(false));
        let stopper = stopped.clone();
        let bandwidth_clone = bandwidth_estimate.clone();
        let mut ice_connection_state_clone = ice_connection_state.clone();

        handle.spawn(async move {
            // Wait for connection before logging bandwidth
            while *ice_connection_state_clone.borrow() != RTCIceConnectionState::Connected {
                if let Err(_) = ice_connection_state_clone.changed().await {
                    return;
                }
            }

            let mut interval = tokio::time::interval(Duration::from_secs(3));

            while !stopped.load(Ordering::Acquire) {
                interval.tick().await;
                let send_bitrate = bandwidth_clone.borrow().bytes_per_sec_f64();
                println!("<: {send_bitrate:.3}");
            }
        });

        std::thread::spawn(move || {
            handle.block_on(async move {
                // Wait for connection before sending data
                while *ice_connection_state.borrow() != RTCIceConnectionState::Connected {
                    if let Err(_) = ice_connection_state.changed().await {
                        // Sender closed
                        stopper.store(true, Ordering::Release);
                        return;
                    }
                }

                const MTU: usize = 1200;
                const FRAME_INTERVAL_60FPS: Duration = Duration::from_nanos(16_666_667);

                let mut interval = tokio::time::interval(FRAME_INTERVAL_60FPS);
                let mut encoder = MockEncoder::new(bandwidth_estimate, ssrc, payload_type);

                while *ice_connection_state.borrow() == RTCIceConnectionState::Connected {
                    interval.tick().await;
                    for packet in encoder.packets(MTU, FRAME_INTERVAL_60FPS) {
                        if let Err(e) = rtp_track.write_rtp(packet).await {
                            panic!("{e}")
                        }
                    }
                }

                stopper.store(true, Ordering::Release);
            });
        });
    }
}

fn dummy_packets(ssrc: u32, payload_type: u8) -> Vec<Packet> {
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
    packets: Vec<Packet>,
}

impl MockEncoder {
    fn new(bandwidth_estimate: TwccBandwidthEstimate, ssrc: u32, payload_type: u8) -> MockEncoder {
        let data_rate = *bandwidth_estimate.borrow();
        MockEncoder {
            sequencer: Box::new(new_random_sequencer()),
            start: (Instant::now(), 0),
            clock_rate: 90000,
            bandwidth_estimate,
            data_rate,
            packets: dummy_packets(ssrc, payload_type),
        }
    }

    fn packets(&mut self, mtu: usize, frame_interval: Duration) -> &[Packet] {
        if let Ok(true) = self.bandwidth_estimate.has_changed() {
            self.data_rate = *self.bandwidth_estimate.borrow();
        }

        let payload_total_bytes = self.data_rate.bytes_per_sec_f64() * frame_interval.as_secs_f64();
        let num_packets = (payload_total_bytes as usize) / (mtu - 12);
        if num_packets == 0 {
            return &[];
        }

        let timestamp = {
            let duration = Instant::now().duration_since(self.start.0).as_micros() as u64;
            // ticks is unitless:
            // duration [us] * clock_rate [1/s] / 1_000_000 [us/s]
            let ticks = duration
                .wrapping_mul(self.clock_rate)
                .wrapping_div(1_000_000);
            self.start.1.wrapping_add(ticks as u32)
        };

        for packet in &mut self.packets[..num_packets] {
            packet.header.sequence_number = self.sequencer.next_sequence_number();
            packet.header.timestamp = timestamp;
        }

        self.packets[num_packets - 1].header.marker = true;
        &self.packets[..num_packets]
    }
}
