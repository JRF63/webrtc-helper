use crate::{
    codecs::Codec,
    encoder::{Encoder, EncoderBuilder},
    interceptor::twcc::TwccBandwidthEstimate,
};
use async_trait::async_trait;
use std::{any::Any, ops::DerefMut};
use tokio::sync::{
    mpsc::{channel, error::TryRecvError, Receiver, Sender},
    Mutex,
};
use webrtc::{
    error::Result,
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalContext,
        TrackLocalWriter,
    },
    Error,
};

const CHANNEL_BUFFER_SIZE: usize = 4;

enum TrackLocalEvent {
    Bind(TrackLocalContext),
    Unbind(TrackLocalContext),
}

struct PacketWriter<T>
where
    T: Encoder,
{
    receiver: Receiver<TrackLocalEvent>,
    rtp_track: TrackLocalStaticRTP,
    encoder: T,
}

impl<T> PacketWriter<T>
where
    T: Encoder,
{
    async fn start(mut self) {
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
                    for packet in self.encoder.packets().iter() {
                        if let Err(_err) = self.rtp_track.write_rtp(packet).await {
                            // TODO: log error
                        }
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    // Sender closed; exit out of loop
                    break;
                }
            }
        }
    }
}

enum TrackLocalData<T: EncoderBuilder> {
    Builder(T),
    Sender((RTCRtpCodecParameters, Sender<TrackLocalEvent>)),
}

pub struct EncoderTrack<T>
where
    T: EncoderBuilder,
{
    data: Mutex<TrackLocalData<T>>,
    bandwidth_estimate: TwccBandwidthEstimate,
    codecs: Box<[Codec]>,
    id: String,
    stream_id: String,
    kind: RTPCodecType,
}

#[async_trait]
impl<T> TrackLocal for EncoderTrack<T>
where
    T: EncoderBuilder + Send + Sync + 'static,
{
    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        let mut data = self.data.lock().await;

        match &*data {
            TrackLocalData::Builder(_) => {
                for codec in t.codec_parameters() {
                    for supported_codec in self.supported_codecs().iter() {
                        if supported_codec.matches_parameters(codec) {
                            let (tx, rx) = channel(CHANNEL_BUFFER_SIZE);

                            let rtp_track = TrackLocalStaticRTP::new(
                                codec.capability.clone(),
                                self.id.clone(),
                                self.stream_id.clone(),
                            );

                            let send_success =
                                tx.send(TrackLocalEvent::Bind(t.clone())).await.is_ok();

                            if !send_success {
                                return Err(Error::ErrUnsupportedCodec);
                            }

                            let mut sender = TrackLocalData::Sender((codec.clone(), tx));

                            std::mem::swap(data.deref_mut(), &mut sender);

                            if let TrackLocalData::Builder(builder) = sender {
                                let encoder = builder.build(codec, self.bandwidth_estimate.clone());
                                tokio::spawn(async move {
                                    let writer = PacketWriter {
                                        receiver: rx,
                                        rtp_track,
                                        encoder,
                                    };
                                    writer.start().await;
                                });
                            }

                            return Ok(codec.clone());
                        }
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
        let data = self.data.lock().await;
        if let TrackLocalData::Sender((_, sender)) = &*data {
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

impl<T> EncoderTrack<T>
where
    T: EncoderBuilder,
{
    pub fn new(
        encoder_builder: T,
        id: String,
        stream_id: String,
        bandwidth_estimate: TwccBandwidthEstimate,
    ) -> Option<EncoderTrack<T>> {
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

        let codecs = codecs.to_vec().into_boxed_slice();
        Some(EncoderTrack {
            data: Mutex::new(TrackLocalData::Builder(encoder_builder)),
            bandwidth_estimate,
            codecs,
            id,
            stream_id,
            kind,
        })
    }

    pub fn supported_codecs(&self) -> &[Codec] {
        &self.codecs
    }
}
