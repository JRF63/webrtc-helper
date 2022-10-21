use crate::{codecs::Codec, encoder::Encoder, interceptor::twcc::TwccBandwidthEstimate};
use async_trait::async_trait;
use std::any::Any;
use tokio::sync::{
    mpsc::{channel, error::TryRecvError, Receiver, Sender},
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

enum TrackLocalEvent {
    Bind(TrackLocalContext),
    Unbind(TrackLocalContext),
}

struct Meow<E>
where
    E: Encoder,
{
    receiver: Receiver<TrackLocalEvent>,
    rtp_track: TrackLocalStaticRTP,
    encoder: E,
    // bandwidth_estimate: TwccBandwidthEstimate,
}

impl<E> Meow<E>
where
    E: Encoder,
{
    async fn encoding_loop(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    match event {
                        TrackLocalEvent::Bind(t) => {
                            if self.rtp_track.bind(&t).await.is_err() {
                                // TODO: log error
                                break;
                            }
                        }
                        TrackLocalEvent::Unbind(t) => {
                            if self.rtp_track.unbind(&t).await.is_err() {
                                // TODO: log error
                                break;
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    // Encode
                    todo!()
                }
                Err(TryRecvError::Disconnected) => {
                    // Sender closed; exit out of loop
                    break;
                }
            }
        }
    }
}

pub struct CustomTrackLocal {
    codecs: Box<[Codec]>,
    pair: Mutex<Option<(RTCRtpCodecParameters, Sender<TrackLocalEvent>)>>,
    id: String,
    stream_id: String,
    kind: RTPCodecType,
}

#[async_trait]
impl TrackLocal for CustomTrackLocal {
    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let mut pair = self.pair.lock().await;
        if let Some((chosen_codec, sender)) = &*pair {
            match sender.send(TrackLocalEvent::Bind(t.clone())).await {
                Ok(_) => Ok(chosen_codec.clone()),
                Err(_) => Err(Error::ErrUnsupportedCodec),
            }
        } else {
            for codec in t.codec_parameters() {
                for supported_codec in self.supported_codecs().iter() {
                    if supported_codec.matches_parameters(codec) {
                        let (tx, rx) = channel(CHANNEL_BUFFER_SIZE);



                        *pair = Some((codec.clone(), tx));
                        if let Some((chosen_codec, sender)) = &*pair {
                            if sender.send(TrackLocalEvent::Bind(t.clone())).await.is_ok() {
                                return Ok(chosen_codec.clone());
                            }
                        }
                    }
                }
            }
            Err(Error::ErrUnsupportedCodec)
        }
    }

    async fn unbind(&self, t: &TrackLocalContext) -> Result<()> {
        let pair = self.pair.lock().await;
        if let Some((_, sender)) = &*pair {
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
            pair: Mutex::new(None),
            id,
            stream_id,
            kind,
        })
    }

    pub fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }
}
