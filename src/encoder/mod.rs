mod track;

pub use self::track::EncoderTrackLocal;
use crate::{codecs::Codec, peer::IceConnectionState, util::data_rate::TwccBandwidthEstimate};
use tokio::sync::mpsc::{error::TryRecvError, Receiver};
use webrtc::{
    ice_transport::ice_connection_state::RTCIceConnectionState,
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
        bandwidth_estimate: TwccBandwidthEstimate,
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
    /// Return the `Packets` for the current frame. This function is allowed to block.
    fn packets(&mut self) -> &[Packet];

    fn start(
        mut self: Box<Self>,
        mut receiver: Receiver<TrackLocalEvent>,
        rtp_track: TrackLocalStaticRTP,
        mut ice_connection_state: IceConnectionState,
    ) where
        // TODO: Why 'static??
        Self: 'static,
    {
        // Spawn a dedicated thread for the encoder since its calls may block.
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    // Wait for connection before sending data
                    while *ice_connection_state.borrow() != RTCIceConnectionState::Connected {
                        if let Err(_) = ice_connection_state.changed().await {
                            // Sender closed
                            return;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));

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
                                // `Encoder::start` is called inside `EncoderTrackLocal::bind` with
                                // `Bind` already passed as the first event. This means that
                                // `rtp_track` will be `bind`ed beforehand in the first branch of
                                // this map and its `write_rtp` method should succeed.

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
