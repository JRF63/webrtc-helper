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
    // Unique identifier for the track. Used in the `TrackLocal` implementation.
    fn id(&self) -> &str;

    // Group this track belongs to. Used in the `TrackLocal` implementation.
    fn stream_id(&self) -> &str;

    /// List of codecs that the encoder supports.
    fn supported_codecs(&self) -> &[Codec];

    /// Build an encoder given the codec parameters.
    // TODO: Maybe use Option<Box<dyn Encoder>> to allow for error reporting?
    fn build(
        self: Box<Self>,
        codec_params: &RTCRtpCodecParameters,
        context: &TrackLocalContext,
    ) -> Box<dyn Encoder>;

    /// Checks if the encoder supports the given codec parameters.
    fn is_codec_supported(&self, codec_params: &RTCRtpCodecParameters) -> bool {
        for supported_codec in self.supported_codecs() {
            if supported_codec.matches_parameters(codec_params) {
                return true;
            }
        }
        false
    }
}

pub trait Encoder: Send {
    // TODO: async or return a Stream
    fn packets(&mut self) -> Box<[Packet]>;

    // TODO: Unused. Probably "pull" MTU instead like with the bandwidth-estimate.
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
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
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
                                    // TODO: Random errors here
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
                })
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
