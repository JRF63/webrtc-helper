mod track;

pub use self::track::EncoderTrackLocal;
use crate::{codecs::Codec, interceptor::twcc::TwccBandwidthEstimate, util::data_rate::DataRate};
use tokio::sync::mpsc::{error::TryRecvError, Receiver};
use webrtc::{
    rtp::packet::Packet,
    rtp_transceiver::rtp_codec::RTCRtpCodecParameters,
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalContext,
        TrackLocalWriter,
    },
};

pub enum TrackLocalEvent {
    Bind(TrackLocalContext),
    Unbind(TrackLocalContext),
}

pub trait EncoderBuilder: Send {
    fn id(&self) -> &str;

    fn stream_id(&self) -> &str;

    fn supported_codecs(&self) -> &[Codec];

    fn build(self: Box<Self>, codec: &RTCRtpCodecParameters) -> Box<dyn Encoder>;

    fn is_codec_supported(&self, codec: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.matches_parameters(codec) {
                return true;
            }
        }
        false
    }
}

pub trait Encoder: Send {
    fn packets(&mut self) -> Box<[Packet]>;

    // TODO: Unused
    fn set_mtu(&mut self, mtu: usize);

    fn set_data_rate(&mut self, data_rate: DataRate);

    fn start(
        mut self: Box<Self>,
        mut receiver: Receiver<TrackLocalEvent>,
        rtp_track: TrackLocalStaticRTP,
        bandwidth_estimate: TwccBandwidthEstimate,
    ) where
        // TODO: Why 'static??
        Self: 'static,
    {
        tokio::spawn(async move {
            let mut data_rate = DataRate::default();

            // TODO: Check if the calls to `packets` and `set_data_rate` passes through a v-table.
            loop {
                match receiver.try_recv() {
                    Ok(event) => {
                        // TODO: log error
                        if process_track_local_event(&rtp_track, event).await.is_err() {
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        // Encode
                        let new_data_rate = bandwidth_estimate.get_estimate();
                        if new_data_rate != data_rate {
                            data_rate = new_data_rate;
                            self.set_data_rate(data_rate);
                        }

                        for packet in self.packets().iter() {
                            if let Err(_err) = rtp_track.write_rtp(packet).await {
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
        });
    }
}

async fn process_track_local_event(
    rtp_track: &TrackLocalStaticRTP,
    event: TrackLocalEvent,
) -> webrtc::error::Result<()> {
    match event {
        TrackLocalEvent::Bind(t) => {
            rtp_track.bind(&t).await?;
        }
        TrackLocalEvent::Unbind(t) => {
            rtp_track.unbind(&t).await?;
        }
    }
    Ok(())
}
