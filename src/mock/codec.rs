use crate::codecs::{supported_video_rtcp_feedbacks, Codec, CodecType};
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters};

pub(crate) fn mock_codec() -> Codec {
    let parameters = RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: "video/mock".to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: supported_video_rtcp_feedbacks(),
        },
        payload_type: 0,
        ..Default::default()
    };
    Codec::new(parameters, CodecType::Video)
}
