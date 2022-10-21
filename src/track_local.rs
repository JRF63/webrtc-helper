use crate::Codec;
use async_trait::async_trait;
use std::any::Any;
use tokio::sync::Mutex;
use webrtc::{
    error::Result,
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalContext,
    },
    Error,
};

pub struct CustomTrackLocal {
    codecs: Box<[Codec]>,
    id: String,
    stream_id: String,
    kind: RTPCodecType,
    rtp_track: Mutex<Option<TrackLocalStaticRTP>>,
}

#[async_trait]
impl TrackLocal for CustomTrackLocal {
    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let mut rtp_track = self.rtp_track.lock().await;
        if let Some(rtp_track) = &*rtp_track {
            rtp_track.bind(t).await
        } else {
            for codec in t.codec_parameters() {
                for supported_codec in self.supported_codecs().iter() {
                    if supported_codec.matches_parameters(codec) {
                        *rtp_track = Some(TrackLocalStaticRTP::new(
                            codec.capability.clone(),
                            self.id.clone(),
                            self.stream_id.clone(),
                        ));
                        if let Some(rtp_track) = &*rtp_track {
                            return rtp_track.bind(t).await;
                        }
                    }
                }
            }
            Err(Error::ErrUnsupportedCodec)
        }
    }

    async fn unbind(&self, t: &TrackLocalContext) -> Result<()> {
        let rtp_track = self.rtp_track.lock().await;
        if let Some(rtp_track) = &*rtp_track {
            rtp_track.unbind(t).await
        } else {
            Err(Error::ErrUnbindFailed)
        }
    }

    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn stream_id(&self) -> &str {
        self.stream_id.as_str()
    }

    fn kind(&self) -> RTPCodecType {
        self.kind
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CustomTrackLocal {
    pub fn new(codecs: &[Codec], id: String, stream_id: String) -> Option<CustomTrackLocal> {
        let codecs: Vec<_> = codecs.iter().cloned().collect();

        let mut audio = 0;
        let mut video = 0;
        for codec in codecs.iter() {
            match codec.kind() {
                RTPCodecType::Unspecified => return None,
                RTPCodecType::Audio => audio += 1,
                RTPCodecType::Video => video += 1,
            }
        }

        let kind = match (audio, video) {
            (0, 0) => return None,
            (_, 0) => RTPCodecType::Audio,
            (0, _) => RTPCodecType::Video,
            _ => return None,
        };

        Some(CustomTrackLocal {
            codecs: codecs.into_boxed_slice(),
            id,
            stream_id,
            kind,
            rtp_track: Mutex::new(None),
        })
    }

    pub fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }
}
