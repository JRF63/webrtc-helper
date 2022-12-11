use webrtc::{rtp_transceiver::rtp_receiver::RTCRtpReceiver, track::track_remote::TrackRemote};

use crate::{codecs::Codec, decoder::DecoderBuilder};
use std::{
    collections::VecDeque,
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
                    const WINDOW_LEN: usize = 2000;
                    let mut data = VecDeque::new();

                    let start = Instant::now();

                    let mut i = 0;
                    let mut total_bytes = 0;
                    let mut buffer = vec![0; 1500];

                    loop {
                        tokio::select! {
                            read_result = track.read(&mut buffer) => {
                                if let Ok((bytes, _)) = read_result {
                                    total_bytes += bytes;
                                    i += 1;

                                    let duration = Instant::now().duration_since(start);
                                    let timestamp = duration.as_millis();

                                    data.push_back((bytes, timestamp as u64));

                                    if data.len() > WINDOW_LEN {
                                        let (bytes, _) = data.pop_front().unwrap();
                                        total_bytes -= bytes;
                                    }

                                    if i % WINDOW_LEN == 0 {
                                        let (_, start) = data.front().unwrap();
                                        let (_, end) = data.back().unwrap();

                                        // in seconds
                                        let elapsed = (end - start) as f64 / 1e3;
                                        let average_bitrate = total_bytes as f64 / elapsed;
                                        println!("   >: {average_bitrate:.4}");
                                    }
                                } else {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                break;
                            }
                        }
                    }
                })
        });
    }
}
