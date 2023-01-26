mod sample_sender;
mod profile;
mod util;

pub use self::{profile::H264Profile, sample_sender::H264SampleSender};
use super::{supported_video_rtcp_feedbacks, Codec, CodecType, MIME_TYPE_H264};
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters};

/// Type representing a specific H.264 codec configuration.
pub struct H264Codec {
    profile: H264Profile,
    level_idc: Option<u8>,
    sps_and_pps: Option<(Vec<u8>, Vec<u8>)>,
}

impl Into<Codec> for H264Codec {
    fn into(self) -> Codec {
        // level_idc=0x1f (Level 3.1)
        // Not important for senders since level-asymmetry-allowed is enabled
        let level_idc = self.level_idc.unwrap_or(0x1f);

        // level-asymmetry-allowed=1 (Offerer can send at a higher level (bitrate) than negotiated)
        // packetization-mode=1 (Single NAL units, STAP-A's, and FU-A's only)
        let mut sdp_fmtp_line = format!(
            "level-asymmetry-allowed=1;\
            packetization-mode=1;\
            profile-level-id={}{level_idc:02x}",
            self.profile.profile_idc_iop()
        );
        if let Some((sps, pps)) = self.sps_and_pps {
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
        Codec::new(parameters, CodecType::Video)
    }
}

impl H264Codec {
    /// Create a `H264Codec` with the given profile.
    pub fn new(profile: H264Profile) -> H264Codec {
        H264Codec {
            profile,
            level_idc: None,
            sps_and_pps: None,
        }
    }

    /// `H264Codec` with parameters that are guaranteed to be supported by most browsers.
    pub fn constrained_baseline() -> H264Codec {
        H264Codec::new(H264Profile::ConstrainedBaseline)
    }

    /// Configure the `H264Codec` to use the given codec level.
    pub fn with_level(mut self, level_idc: u8) -> H264Codec {
        self.level_idc = Some(level_idc);
        self
    }

    /// Configure the `H264Codec` to use the passed SPS/PPS parameters.
    pub fn with_parameter_sets(mut self, sps: &[u8], pps: &[u8]) -> H264Codec {
        self.sps_and_pps = Some((sps.to_vec(), pps.to_vec()));
        self
    }

    /// Read the (width, height) of the video stream from the SPS/PPS parameter sets.
    pub fn get_resolution(nal: &[u8]) -> Option<(usize, usize)> {
        util::parse_parameter_sets_for_resolution(nal)
    }
}
