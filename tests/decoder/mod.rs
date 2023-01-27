use webrtc::{
    ice_transport::ice_connection_state::RTCIceConnectionState,
    rtp_transceiver::rtp_receiver::RTCRtpReceiver, track::track_remote::TrackRemote,
};

use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use webrtc_helper::{codecs::Codec, decoder::DecoderBuilder, peer::IceConnectionState};

pub struct MockDecoderBuilder {
    codecs: Vec<Codec>,
}

impl MockDecoderBuilder {
    pub fn new() -> Self {
        Self {
            codecs: vec![super::codec::mock_codec()],
        }
    }
}

impl DecoderBuilder for MockDecoderBuilder {
    fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }

    fn build(
        self: Box<Self>,
        track: Arc<TrackRemote>,
        _rtp_receiver: Arc<RTCRtpReceiver>,
        ice_connection_state: IceConnectionState,
    ) {
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            handle.block_on(async move {
                let mut data = Vec::new();

                let start = Instant::now();

                let mut packet_bytes_accum = 0;
                let mut buffer = vec![0; 1500];

                let mut interval = tokio::time::interval(Duration::from_secs(3));
                interval.tick().await;

                while *ice_connection_state.borrow() == RTCIceConnectionState::Connected {
                    tokio::select! {
                        read_result = track.read(&mut buffer) => {
                            if let Ok((packet_bytes, _)) = read_result {
                                packet_bytes_accum += packet_bytes;

                                let duration = Instant::now().duration_since(start);
                                let timestamp = duration.as_millis();

                                data.push((packet_bytes, timestamp as u64));
                            } else {
                                break;
                            }
                        }
                        _ = interval.tick() => {
                            if let (Some((_, start)), Some((_, end))) = (data.first(), data.last()) {
                                let elapsed = (end - start) as f64 / 1e3; // in seconds
                                let average_bitrate = packet_bytes_accum as f64 / elapsed;
                                println!("   >: {average_bitrate:.4}");
                                packet_bytes_accum = 0;
                                data.clear();
                            }
                        }
                    }
                }
            })
        });
    }
}
