use super::{EncoderBuilder, TrackLocalEvent};
use crate::{peer::IceConnectionState, util::data_rate::TwccBandwidthEstimate};
use async_trait::async_trait;
use std::any::Any;
use tokio::sync::{
    mpsc::{channel, Sender},
    Mutex,
};
use webrtc::{
    error::Result,
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalContext,
    },
    Error,
};

const CHANNEL_BUFFER_SIZE: usize = 4;

enum TrackLocalData {
    Builder(Box<dyn EncoderBuilder>),
    Sender((RTCRtpCodecParameters, Sender<TrackLocalEvent>)),
}

pub struct EncoderTrackLocal {
    data: Mutex<TrackLocalData>,
    ice_connection_state: IceConnectionState,
    bandwidth_estimate: TwccBandwidthEstimate,
    id: String,
    stream_id: String,
    kind: RTPCodecType,
}

#[async_trait]
impl TrackLocal for EncoderTrackLocal {
    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let mut data = self.data.lock().await;

        match &mut *data {
            TrackLocalData::Builder(builder) => {
                for codec in t.codec_parameters() {
                    if builder.is_codec_supported(codec) {
                        let (tx, rx) = channel(CHANNEL_BUFFER_SIZE);

                        let rtp_track = TrackLocalStaticRTP::new(
                            codec.capability.clone(),
                            self.id.clone(),
                            self.stream_id.clone(),
                        );

                        let send_success = tx.send(TrackLocalEvent::Bind(t.clone())).await.is_ok();

                        if !send_success {
                            return Err(Error::ErrUnsupportedCodec);
                        }

                        let mut sender = TrackLocalData::Sender((codec.clone(), tx));

                        std::mem::swap(&mut *data, &mut sender);

                        if let TrackLocalData::Builder(builder) = sender {
                            let encoder = builder.build(codec, t, self.bandwidth_estimate.clone());
                            encoder.start(rx, rtp_track, self.ice_connection_state.clone());
                        }

                        return Ok(codec.clone());
                    }
                }
                Err(Error::ErrUnsupportedCodec)
            }
            TrackLocalData::Sender((codec, sender)) => {
                match sender.send(TrackLocalEvent::Bind(t.clone())).await {
                    Ok(_) => Ok(codec.clone()),
                    Err(_) => Err(Error::ErrUnsupportedCodec),
                }
            }
        }
    }

    async fn unbind(&self, t: &TrackLocalContext) -> Result<()> {
        let mut data = self.data.lock().await;
        if let TrackLocalData::Sender((_, sender)) = &mut *data {
            if sender
                .send(TrackLocalEvent::Unbind(t.clone()))
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        Err(Error::ErrUnbindFailed)
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

impl EncoderTrackLocal {
    pub fn new(
        encoder_builder: Box<dyn EncoderBuilder>,
        ice_connection_state: IceConnectionState,
        bandwidth_estimate: TwccBandwidthEstimate,
    ) -> Option<EncoderTrackLocal> {
        let codecs = encoder_builder.supported_codecs();

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

        let id = encoder_builder.id().to_owned();
        let stream_id = encoder_builder.stream_id().to_owned();

        Some(EncoderTrackLocal {
            data: Mutex::new(TrackLocalData::Builder(encoder_builder)),
            ice_connection_state,
            bandwidth_estimate,
            id,
            stream_id,
            kind,
        })
    }
}
