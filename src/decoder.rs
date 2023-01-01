use crate::codecs::Codec;
use std::sync::Arc;
use webrtc::{
    rtp_transceiver::{rtp_codec::RTCRtpCodecParameters, rtp_receiver::RTCRtpReceiver},
    track::track_remote::TrackRemote,
};

pub trait DecoderBuilder: Send {
    fn supported_codecs(&self) -> &[Codec];

    fn build(self: Box<Self>, track: Arc<TrackRemote>, rtp_receiver: Arc<RTCRtpReceiver>);

    fn is_codec_supported(&self, codec: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.capability_matches(codec) {
                return true;
            }
        }
        false
    }
}
