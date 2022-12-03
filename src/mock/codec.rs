use crate::codecs::{supported_video_rtcp_feedbacks, Codec};
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};

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
    let kind = RTPCodecType::Video;
    Codec::new(parameters, kind)
}
