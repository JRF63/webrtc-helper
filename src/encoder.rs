use crate::{codecs::Codec, interceptor::twcc::TwccBandwidthEstimate};
use tokio::sync::mpsc::{error::TryRecvError, Receiver};
use webrtc::{
    rtp::packet::Packet,
    rtp_transceiver::rtp_codec::RTCRtpCodecParameters,
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalContext,
        TrackLocalWriter,
    },
};

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

pub enum TrackLocalEvent {
    Bind(TrackLocalContext),
    Unbind(TrackLocalContext),
}

pub trait Encoder: Send {
    fn packets(&mut self) -> Box<[Packet]>;

    fn set_mtu(&mut self, mtu: usize);
}

pub trait EncoderBuilder: Send {
    fn supported_codecs(&self) -> &[Codec];

    fn build(
        self: Box<Self>,
        codec: &RTCRtpCodecParameters,
        rtp_track: TrackLocalStaticRTP,
        bandwidth_estimate: TwccBandwidthEstimate,
        receiver: Receiver<TrackLocalEvent>,
    );

    fn start_encoder(
        self: Box<Self>,
        codec: &RTCRtpCodecParameters,
        rtp_track: TrackLocalStaticRTP,
        bandwidth_estimate: TwccBandwidthEstimate,
        receiver: Receiver<TrackLocalEvent>,
    ) where
        Self: Sized,
    {
        let encoder_builder = *self;
    }

    fn is_codec_supported(&self, codec: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.matches_parameters(codec) {
                return true;
            }
        }
        false
    }
}
