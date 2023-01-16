use webrtc::{
    api::media_engine::MediaEngine,
    rtp_transceiver::{
        rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
        RTCPFeedback,
    },
};

const MIME_TYPE_H264: &str = "video/H264";
const MIME_TYPE_OPUS: &str = "audio/opus";

// TODO H265:
// See https://www.rfc-editor.org/rfc/rfc7798#section-7.1
// const MIME_TYPE_H265: &str = "video/H265";

/// The type of a [Codec].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CodecType {
    Audio = 1,
    Video = 2,
}

impl Into<RTPCodecType> for CodecType {
    fn into(self) -> RTPCodecType {
        match self {
            CodecType::Audio => RTPCodecType::Audio,
            CodecType::Video => RTPCodecType::Video,
        }
    }
}

/// Abstraction of a media codec for registering in a [MediaEngine].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Codec {
    parameters: RTCRtpCodecParameters,
    codec_type: CodecType,
}

impl Codec {
    /// Create a new [Codec].
    pub fn new(parameters: RTCRtpCodecParameters, codec_type: CodecType) -> Codec {
        Codec {
            parameters,
            codec_type,
        }
    }

    /// Returns the type (audio/video) of the [Codec].
    pub fn codec_type(&self) -> CodecType {
        self.codec_type
    }

    /// Modifies the payload type of the [Codec].
    pub fn set_payload_type(&mut self, payload_type: u8) {
        self.parameters.payload_type = payload_type;
    }

    /// Checks if the [Codec] has the same [RTCRtpCodecCapability] as `codec_capability`.
    pub fn capability_matches(&self, codec_capability: &RTCRtpCodecCapability) -> bool {
        // All parameters except `rtcp_feedback` should match
        let codec_matches = |a: &RTCRtpCodecCapability, b: &RTCRtpCodecCapability| {
            a.mime_type == b.mime_type
                && a.clock_rate == b.clock_rate
                && a.channels == b.channels
                && a.sdp_fmtp_line == b.sdp_fmtp_line
        };

        codec_matches(&self.parameters.capability, codec_capability)
    }

    /// Create an [RFC4588][RFC4588] retransmission [Codec] from a base video [Codec]. Returns
    /// [None] if `base_codec` is of type [CodecType::Audio].
    ///
    /// [RFC4588]: https://www.rfc-editor.org/rfc/rfc4588
    pub fn retransmission(base_codec: &Codec) -> Option<Codec> {
        if base_codec.codec_type() == CodecType::Audio {
            return None;
        }

        let parameters = RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: "video/rtx".to_owned(),
                sdp_fmtp_line: format!("apt={}", base_codec.parameters.payload_type),
                rtcp_feedback: Vec::new(),
                ..base_codec.parameters.capability.clone()
            },
            ..Default::default()
        };

        Some(Codec::new(parameters, CodecType::Video))
    }

    /// Create an [RFC5109][RFC5109] [Codec].
    ///
    /// [RFC5109]: https://www.rfc-editor.org/rfc/rfc5109
    pub fn ulpfec() -> Codec {
        let parameters = RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: "video/ulpfec".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: Vec::new(),
            },
            ..Default::default()
        };
        Codec::new(parameters, CodecType::Video)
    }

    /// Create an Opus [Codec].
    pub fn opus() -> Codec {
        let parameters = RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                rtcp_feedback: Vec::new(),
            },
            payload_type: 0,
            ..Default::default()
        };
        Codec::new(parameters, CodecType::Audio)
    }

    /// Create an H.264 [Codec] with the given parameters as defined in [RFC6184][RFC6184].
    ///
    /// [RFC6184]: https://www.rfc-editor.org/rfc/rfc6184.html#section-8.1
    pub fn h264_custom(
        profile: H264Profile,
        level_idc: Option<u8>,
        sps_and_pps: Option<(&[u8], &[u8])>,
    ) -> Codec {
        // level_idc=0x1f (Level 3.1)
        // Hardcoded since level-asymmetry-allowed is enabled
        let level_idc = level_idc.unwrap_or(0x1f);

        // level-asymmetry-allowed=1 (Offerer can send at a higher level (bitrate) than negotiated)
        // packetization-mode=1 (Single NAL units, STAP-A's, and FU-A's only)
        let mut sdp_fmtp_line = format!(
            "level-asymmetry-allowed=1;\
            packetization-mode=1;\
            profile-level-id={}{level_idc:02x}",
            profile.profile_idc_iop()
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
        Codec::new(parameters, CodecType::Video)
    }

    /// H264 [Codec] with parameters that are guaranteed to be supported by most browsers.
    pub fn h264() -> Codec {
        // profile_idc=0x42 (Constrained Baseline)
        // profile_iop=0b11100000
        Codec::h264_custom(H264Profile::ConstrainedBaseline, None, None)
    }
}

/// RTCP feedbacks that can be handled either by this crate or natively by webrtc-rs.
pub(crate) fn supported_video_rtcp_feedbacks() -> Vec<RTCPFeedback> {
    vec![RTCPFeedback {
        typ: "ccm".to_owned(),
        parameter: "fir".to_owned(),
    }]
}

/// Helper trait for adding methods to [MediaEngine].
pub(crate) trait MediaEngineExt {
    /// Register the [Codec] with a dynamically chose payload type.
    fn register_custom_codec(&mut self, codec: Codec) -> Result<(), webrtc::Error>;
}

impl MediaEngineExt for MediaEngine {
    fn register_custom_codec(&mut self, codec: Codec) -> Result<(), webrtc::Error> {
        self.register_codec(codec.parameters, codec.codec_type.into())?;
        Ok(())
    }
}

/// H.264 codec profile
#[non_exhaustive]
pub enum H264Profile {
    ConstrainedBaseline,
    Baseline,
    Main,
    Extended,
    High,
    ProgressiveHigh,
    ConstrainedHigh,
    High10,
    High422,
    High444,
    High10Intra,
    High422Intra,
    High444Intra,
    Cavlc444Intra,
    StereoHigh,
}

impl H264Profile {
    const IDC_BASELINE: u8 = 0x42;
    const IDC_MAIN: u8 = 0x4D;
    const IDC_EXTENDED: u8 = 0x58;
    const IDC_HIGH: u8 = 0x64;
    const IDC_HIGH_10: u8 = 0x6E;
    const IDC_HIGH_422: u8 = 0x7A;
    const IDC_HIGH_444: u8 = 0xF4;
    const IDC_CAVLC_444: u8 = 0x44;
    const IDC_STEREO_HIGH: u8 = 0x80;

    /// Parse the `H264Profile` as a partial profile-level-id.
    pub fn profile_idc_iop(self) -> String {
        // https://developer.mozilla.org/en-US/docs/Web/Media/Formats/codecs_parameter
        let (profile_idc, profile_iop): (u8, u8) = match self {
            H264Profile::ConstrainedBaseline => (H264Profile::IDC_BASELINE, 0xE0),
            H264Profile::Baseline => (H264Profile::IDC_BASELINE, 0),
            H264Profile::Main => (H264Profile::IDC_MAIN, 0),
            H264Profile::Extended => (H264Profile::IDC_EXTENDED, 0),
            H264Profile::High => (H264Profile::IDC_HIGH, 0),
            H264Profile::ProgressiveHigh => (H264Profile::IDC_HIGH, 0x08),
            H264Profile::ConstrainedHigh => (H264Profile::IDC_HIGH, 0x0C),
            H264Profile::High10 => (H264Profile::IDC_HIGH_10, 0),
            H264Profile::High422 => (H264Profile::IDC_HIGH_422, 0),
            H264Profile::High444 => (H264Profile::IDC_HIGH_444, 0),
            H264Profile::High10Intra => (H264Profile::IDC_HIGH_10, 0x10),
            H264Profile::High422Intra => (H264Profile::IDC_HIGH_422, 0x10),
            H264Profile::High444Intra => (H264Profile::IDC_HIGH_444, 0x10),
            H264Profile::Cavlc444Intra => (H264Profile::IDC_CAVLC_444, 0),
            H264Profile::StereoHigh => (H264Profile::IDC_STEREO_HIGH, 0),
        };
        format!("{profile_idc:02x}{profile_iop:02x}")
    }

    /// Try to convert the `str` to a `H264Profile`.
    pub fn from_str(src: &str) -> Result<H264Profile, ()> {
        let bytes = src.as_bytes();
        let idc_str = std::str::from_utf8(&bytes[..2]).map_err(|_| ())?;
        let iop_str = std::str::from_utf8(&bytes[2..]).map_err(|_| ())?;
        let idc = u8::from_str_radix(idc_str, 16).map_err(|_| ())?;
        let iop = u8::from_str_radix(iop_str, 16).map_err(|_| ())?;

        // Table 5 of RFC6184.
        //
        //   Profile     profile_idc        profile-iop
        //               (hexadecimal)      (binary)

        //   CB          42 (B)             x1xx0000
        //      same as: 4D (M)             1xxx0000
        //      same as: 58 (E)             11xx0000
        //   B           42 (B)             x0xx0000
        //      same as: 58 (E)             10xx0000
        //   M           4D (M)             0x0x0000
        //   E           58                 00xx0000
        //   H           64                 00000000
        //   H10         6E                 00000000
        //   H42         7A                 00000000
        //   H44         F4                 00000000
        //   H10I        6E                 00010000
        //   H42I        7A                 00010000
        //   H44I        F4                 00010000
        //   C44I        2C                 00010000

        const BITS_ON_LAST_HALF_MASK: u8 = 0b00001111;

        // FIXME: This is ugly
        match idc {
            H264Profile::IDC_BASELINE => {
                const CONSTRAINED_BASELINE_MASK: u8 = 0b01000000;

                if iop & BITS_ON_LAST_HALF_MASK != 0 {
                    return Err(());
                }

                if iop & CONSTRAINED_BASELINE_MASK != 0 {
                    return Ok(H264Profile::ConstrainedBaseline);
                }

                return Ok(H264Profile::Baseline);
            }
            H264Profile::IDC_MAIN => {
                const CONSTRAINED_BASELINE_MASK: u8 = 0b10000000;
                const VALID_MAIN_MASK: u8 = 0b01010000;

                if iop & BITS_ON_LAST_HALF_MASK != 0 {
                    return Err(());
                }

                if iop & CONSTRAINED_BASELINE_MASK != 0 {
                    return Ok(H264Profile::ConstrainedBaseline);
                }

                if iop & !VALID_MAIN_MASK != 0 {
                    return Err(());
                }

                return Ok(H264Profile::Main);
            }
            H264Profile::IDC_EXTENDED => {
                const BASELINE_MASK: u8 = 0b10000000;
                const CONSTRAINED_BASELINE_MASK: u8 = 0b11000000;
                const VALID_EXTENDED_MASK: u8 = 0b00110000;

                if iop & BITS_ON_LAST_HALF_MASK != 0 {
                    return Err(());
                }

                if iop & CONSTRAINED_BASELINE_MASK != 0 {
                    return Ok(H264Profile::ConstrainedBaseline);
                }

                if iop & BASELINE_MASK != 0 {
                    return Ok(H264Profile::Baseline);
                }

                if iop & !VALID_EXTENDED_MASK != 0 {
                    return Err(());
                }

                return Ok(H264Profile::Extended);
            }
            H264Profile::IDC_HIGH => match iop {
                0 => return Ok(H264Profile::High),
                0b00001000 => return Ok(H264Profile::ProgressiveHigh),
                0b00001100 => return Ok(H264Profile::ConstrainedHigh),
                _ => (),
            },
            H264Profile::IDC_HIGH_10 => match iop {
                0 => return Ok(H264Profile::High10),
                0b00010000 => return Ok(H264Profile::High10Intra),
                _ => (),
            },
            H264Profile::IDC_HIGH_422 => match iop {
                0 => return Ok(H264Profile::High422),
                0b00010000 => return Ok(H264Profile::High422Intra),
                _ => (),
            },
            H264Profile::IDC_HIGH_444 => match iop {
                0 => return Ok(H264Profile::High444),
                0b00010000 => return Ok(H264Profile::High444Intra),
                _ => (),
            },
            H264Profile::IDC_CAVLC_444 => match iop {
                0b00010000 => return Ok(H264Profile::Cavlc444Intra),
                _ => (),
            },
            _ => (),
        }
        Err(())
    }
}
