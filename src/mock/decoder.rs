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

                    // let sleep = tokio::time::sleep(Duration::from_secs(5));
                    // tokio::pin!(sleep);
                    loop {
                        tokio::select! {
                            read_result = track.read(&mut buffer) => {
                                if let Ok((bytes, _)) = read_result {
                                    let duration = Instant::now().duration_since(start);
                                    let timestamp = duration.as_millis();
                                    data.push((bytes, timestamp as u64));
                                } else {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {
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

                    let elapsed = (end - start) as f64 / 1e3;
                    // bytes per sec
                    let average_bitrate = total_bytes as f64 / elapsed;
                    println!("Bitrate: {average_bitrate} Bps, Elapsed: {elapsed}");
                })
        });
    }
}
