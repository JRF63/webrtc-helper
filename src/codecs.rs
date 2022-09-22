use webrtc::rtp_transceiver::{
    rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters},
    RTCPFeedback,
};

pub const MIME_TYPE_H264: &str = "video/H264";
pub const MIME_TYPE_H265: &str = "video/H265";
pub const MIME_TYPE_OPUS: &str = "audio/opus";

pub fn opus_codec() -> RTCRtpCodecParameters {
    // https://github.com/webrtc-rs/webrtc/blob/c30b5c1db4668bb1314f32e0121270e1bb1dac7a/webrtc/src/api/media_engine/mod.rs#L90
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 0,
        ..Default::default()
    }
}

fn video_rtcp_feedback() -> Vec<RTCPFeedback> {
    // "goog-remb" is replaced with "transport-cc"
    // https://github.com/webrtc-rs/webrtc/blob/c30b5c1db4668bb1314f32e0121270e1bb1dac7a/webrtc/src/api/media_engine/mod.rs#L138
    vec![
        RTCPFeedback {
            typ: "transport-cc".to_owned(),
            parameter: "".to_owned(),
        },
        RTCPFeedback {
            typ: "ccm".to_owned(),
            parameter: "fir".to_owned(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: "".to_owned(),
        },
        RTCPFeedback {
            typ: "nack".to_owned(),
            parameter: "pli".to_owned(),
        },
    ]
}

// RFC4588
fn rtp_retransmission(video_codec_params: &RTCRtpCodecParameters) -> RTCRtpCodecParameters {
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: "video/rtx".to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: format!("apt={}", video_codec_params.payload_type),
            rtcp_feedback: vec![],
        },
        payload_type: 0,
        ..Default::default()
    }
}

// RFC5109
fn ulpfec() -> RTCRtpCodecParameters {
    // https://github.com/webrtc-rs/webrtc/blob/c30b5c1db4668bb1314f32e0121270e1bb1dac7a/webrtc/src/api/media_engine/mod.rs#L367
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: "video/ulpfec".to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: "".to_owned(),
            rtcp_feedback: vec![],
        },
        payload_type: 0,
        ..Default::default()
    }
}

/// H264 codec with parameters that is guaranteed to be supported by most browsers.
fn h264_browser_guaranteed_codec() -> RTCRtpCodecParameters {
    // https://github.com/webrtc-rs/webrtc/blob/c30b5c1db4668bb1314f32e0121270e1bb1dac7a/webrtc/src/api/media_engine/mod.rs#L271
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90000,
            channels: 0,
            // level-asymmetry-allowed=1 (Offerer can send at a higher level (bitrate) than negotiated)
            // packetization-mode=1 (Single NAL units, STAP-A's, and FU-A's only)
            // profile_idc=0x42 (Constrained Baseline)
            // profile_iop=0b11100000
            // level_idc=0x1f (Level 3.1)
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .to_owned(),
            rtcp_feedback: video_rtcp_feedback(),
        },
        payload_type: 0,
        ..Default::default()
    }
}

fn h264_custom_codec(
    profile_idc: u8,
    profile_iop: u8,
    level_idc: u8,
    sps: &[u8],
    pps: &[u8],
) -> RTCRtpCodecParameters {
    let sps_base64 = base64::encode(sps);
    let pps_base64 = base64::encode(pps);
    let sdp_fmtp_line = format!(
        "level-asymmetry-allowed=1;\
        packetization-mode=1;\
        profile-level-id={profile_idc:x}{profile_iop:x}{level_idc:x};\
        sprop-parameter-sets={sps_base64},{pps_base64}"
    );
    RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line,
            rtcp_feedback: video_rtcp_feedback(),
        },
        payload_type: 0,
        ..Default::default()
    }
}
