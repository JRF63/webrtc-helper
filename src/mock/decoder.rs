use webrtc::{rtp_transceiver::rtp_receiver::RTCRtpReceiver, track::track_remote::TrackRemote};

use crate::{codecs::Codec, decoder::DecoderBuilder};
use std::sync::Arc;

pub struct MockDecoderBuilder {
    codecs: Vec<Codec>,
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
                    let mut buffer = vec![0; 1500];
                    while let Ok(_) = track.read(&mut buffer).await {}
                })
        });
    }
}
