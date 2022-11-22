use crate::codecs::Codec;
use std::sync::Arc;
use webrtc::{
    rtp_transceiver::{rtp_codec::RTCRtpCodecParameters, rtp_receiver::RTCRtpReceiver},
    track::track_remote::TrackRemote,
};

pub trait Decoder: Send {
    fn supported_codecs(&self) -> &[Codec];

    fn build(self: Box<Self>, track: Arc<TrackRemote>, receiver: Arc<RTCRtpReceiver>);

    fn is_codec_supported(&self, codec: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.matches_parameters(codec) {
                return true;
            }
        }
        false
    }
}
