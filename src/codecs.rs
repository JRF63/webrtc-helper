use webrtc::{
    api::media_engine::MediaEngine,
    error::Result,
    rtp_transceiver::{
        rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
        RTCPFeedback,
    },
};

const MIME_TYPE_H264: &str = "video/H264";
const MIME_TYPE_OPUS: &str = "audio/opus";
// TODO:
// const MIME_TYPE_H265: &str = "video/H265";

#[derive(Clone)]
pub struct Codec {
    parameters: RTCRtpCodecParameters,
    kind: RTPCodecType,
}

impl Codec {
    pub(crate) fn set_payload_type(&mut self, payload_type: u8) {
        self.parameters.payload_type = payload_type;
    }

    /// Configure the media engine to use the codec and returns the number of codecs registered.
    pub(crate) fn register_to_media_engine(&self, media_engine: &mut MediaEngine) -> Result<u8> {
        media_engine.register_codec(self.parameters.clone(), self.kind)?;
        if self.kind == RTPCodecType::Video {
            let rfc4588_payload_type = self.parameters.payload_type.checked_add(1);
            if let Some(payload_type) = rfc4588_payload_type {
                let rfc4588_params = RTCRtpCodecParameters {
                    capability: RTCRtpCodecCapability {
                        mime_type: "video/rtx".to_owned(),
                        clock_rate: 90000,
                        channels: 0,
                        sdp_fmtp_line: format!("apt={}", self.parameters.payload_type),
                        rtcp_feedback: vec![],
                    },
                    payload_type,
                    ..Default::default()
                };
                media_engine.register_codec(rfc4588_params, RTPCodecType::Video)?;
                Ok(2)
            } else {
                // u8 overflowed
                panic!("Registered too many codecs");
            }
        } else {
            Ok(1)
        }
    }

    pub(crate) fn kind(&self) -> RTPCodecType {
        self.kind
    }
    
    pub(crate) fn matches_parameters(
        &self,
        parameters: &RTCRtpCodecParameters,
    ) -> bool {
        self.parameters.capability == parameters.capability
    }

    pub fn opus() -> Codec {
        let parameters = RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 0,
            ..Default::default()
        };
        let kind = RTPCodecType::Audio;
        Codec { parameters, kind }
    }

    pub fn h264_custom(
        profile_idc: u8,
        profile_iop: u8,
        sps_and_pps: Option<(&[u8], &[u8])>,
    ) -> Codec {
        // level_idc=0x1f (Level 3.1)
        // Hardcoded since level-asymmetry-allowed is enabled
        let level_idc = 0x1f;

        // level-asymmetry-allowed=1 (Offerer can send at a higher level (bitrate) than negotiated)
        // packetization-mode=1 (Single NAL units, STAP-A's, and FU-A's only)
        let mut sdp_fmtp_line = format!(
            "level-asymmetry-allowed=1;\
            packetization-mode=1;\
            profile-level-id={profile_idc:x}{profile_iop:x}{level_idc:x}"
        );
        if let Some((sps, pps)) = sps_and_pps {
            let sps_base64 = base64::encode(sps);
            let pps_base64 = base64::encode(pps);
            sdp_fmtp_line.push_str(&format!(";sprop-parameter-sets={sps_base64},{pps_base64}"))
        }
        let parameters = RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line,
                rtcp_feedback: supported_video_rtcp_feedbacks(),
            },
            payload_type: 0,
            ..Default::default()
        };
        let kind = RTPCodecType::Video;
        Codec { parameters, kind }
    }

    /// H264 codec with parameters that are guaranteed to be supported by most browsers.
    pub fn h264() -> Codec {
        // profile_idc=0x42 (Constrained Baseline)
        // profile_iop=0b11100000
        Codec::h264_custom(0x42, 0b11100000, None)
    }
}

/// RTCP feedbacks that can be handled either by this crate or natively by webrtc-rs.
fn supported_video_rtcp_feedbacks() -> Vec<RTCPFeedback> {
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

// TODO:
// fn ulpfec() -> RTCRtpCodecParameters {
//     // https://github.com/webrtc-rs/webrtc/blob/c30b5c1db4668bb1314f32e0121270e1bb1dac7a/webrtc/src/api/media_engine/mod.rs#L367
//     RTCRtpCodecParameters {
//         capability: RTCRtpCodecCapability {
//             mime_type: "video/ulpfec".to_owned(),
//             clock_rate: 90000,
//             channels: 0,
//             sdp_fmtp_line: "".to_owned(),
//             rtcp_feedback: vec![],
//         },
//         payload_type: 0,
//         ..Default::default()
//     }
// }
