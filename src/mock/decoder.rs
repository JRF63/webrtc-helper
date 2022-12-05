use webrtc::{rtp_transceiver::rtp_receiver::RTCRtpReceiver, track::track_remote::TrackRemote};

use crate::{codecs::Codec, decoder::DecoderBuilder};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
    fn supported_codecs(&self) -> &[crate::codecs::Codec] {
        &self.codecs
    }

    fn build(self: Box<Self>, track: Arc<TrackRemote>, _rtp_receiver: Arc<RTCRtpReceiver>) {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let start = Instant::now();
                    let mut data = Vec::new();

                    let mut buffer = vec![0; 1500];

                    let sleep = tokio::time::sleep(Duration::from_secs(25));
                    tokio::pin!(sleep);
                    loop {
                        tokio::select! {
                            read_result = track.read(&mut buffer) => {
                                if let Ok((bytes, _)) = read_result {
                                    let duration = Instant::now().duration_since(start);
                                    let timestamp = duration.as_millis() / 5000;
                                    data.push((bytes, timestamp as u64));
                                } else {
                                    break;
                                }
                            }
                            _ = &mut sleep => {
                                break;
                            }
                        }
                    }

                    let (_, start) = data.first().unwrap();
                    let (_, end) = data.last().unwrap();

                    let mut total_bytes = 0;
                    for (bytes, _) in &data {
                        total_bytes += bytes;
                    }

                    // bytes per millisecond
                    let average_bitrate = 1000.0 * total_bytes as f64 / (end - start) as f64;
                    let bitrate_mbps = average_bitrate / 1e6;
                    println!("Bitrate: {bitrate_mbps} MBps");
                })
        });
    }
}
