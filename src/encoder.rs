use crate::{codecs::Codec, interceptor::twcc::TwccBandwidthEstimate};
use webrtc::{rtp::packet::Packet, rtp_transceiver::rtp_codec::RTCRtpCodecParameters};

pub trait Encoder: Sized + Send {
    fn packets(&mut self) -> Box<[Packet]>;

    fn set_mtu(&mut self, mtu: usize);
}

pub trait EncoderBuilder {
    type EncoderType: Encoder;

    fn supported_codecs(&self) -> &[Codec];

    fn build(
        self,
        codec: &RTCRtpCodecParameters,
        bandwidth_estimate: TwccBandwidthEstimate,
    ) -> Self::EncoderType;

    fn is_codec_supported(&self, codec: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.matches_parameters(codec) {
                return true;
            }
        }
        false
    }
}
