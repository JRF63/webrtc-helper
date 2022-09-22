use crate::{
    interceptor::configure_twcc_capturer,
    signaling::{Message, Signaler},
    Result,
};
use std::sync::Arc;
use tokio::sync::Notify;
use webrtc::{
    api::{interceptor_registry::configure_nack, media_engine::MediaEngine, APIBuilder},
    ice_transport::{
        ice_connection_state::RTCIceConnectionState, ice_gatherer_state::RTCIceGathererState, ice_server::RTCIceServer,
    },
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
    },
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::track_local_static_rtp::TrackLocalStaticRTP,
};

pub struct StreamingServerBuilder {
    tracks: Vec<(Arc<TrackLocalStaticRTP>, RTPCodecType)>,
    signaler: Option<Arc<dyn Signaler + Send + Sync>>,
    ice_servers: Vec<RTCIceServer>,
    mdns: bool
}

impl StreamingServerBuilder {
    pub fn new() -> Self {
        StreamingServerBuilder {
            tracks: Vec::new(),
            signaler: None,
            ice_servers: Vec::new(),
            mdns: false,
        }
    }

    pub async fn build(self) -> Result<StreamingServer> {
        const DYNAMIC_PAYLOAD_TYPE_START: u8 = 96u8;

        let mut media_engine = MediaEngine::default();
        for (payload_type, (track, codec_type)) in (DYNAMIC_PAYLOAD_TYPE_START..).zip(&self.tracks)
        {
            let codec = RTCRtpCodecParameters {
                capability: track.codec(),
                payload_type,
                ..Default::default()
            };
            media_engine.register_codec(codec, *codec_type)?;
        }

        let registry = configure_nack(Registry::new(), &mut media_engine);
        let (registry, twcc_data_receiver) = configure_twcc_capturer(registry, &mut media_engine)?;

        let api_builder = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let peer_connection = Arc::new(api_builder.new_peer_connection(RTCConfiguration {
            ice_servers: self.ice_servers,
            ..Default::default()
        }).await?);

        // for (track, _) in self.tracks {
        //     // TODO: tokio::spawn a handler
        //     let _rtp_sender = peer_connection.add_track(track as _).await?;
        // }

        let signaler = self.signaler.expect("Signaling channel not set");
        let streaming_server = StreamingServerInternal {
            peer_connection,
            signaler,
        };

        // let closure_clone = signaler.clone();
        // peer_connection
        //     .on_ice_candidate(Box::new(move |candidate| {
        //         let signaler = closure_clone.clone();
        //         Box::pin(async move {
        //             if let Some(candidate) = candidate {
        //                 signaler
        //                     .send(Message::IceCandidate(candidate))
        //                     .await
        //                     .expect("Peer A: Unable to send ICE candidate");
        //             }
        //         })
        //     }))
        //     .await;

        // peer_connection
        //     .on_ice_connection_state_change(Box::new(move |state| {
        //         Box::pin(async move {
        //             if state == RTCIceConnectionState::Failed {
        //                 // TODO: ICE restart
        //             }
        //         })
        //     }))
        //     .await;

        // let ice_gathering_complete = Arc::new(Notify::new());
        // let ice_gathering_complete_clone = ice_gathering_complete.clone();
        // peer_connection
        //     .on_ice_gathering_state_change(Box::new(move |state| {
        //         if state == RTCIceGathererState::Complete {
        //             ice_gathering_complete_clone.notify_one();
        //         }
        //         Box::pin(async {})
        //     }))
        //     .await;

        // let peer_connection_clone = peer_connection.clone();
        // let signaler_clone = signaler.clone();
        // peer_connection
        //     .on_negotiation_needed(Box::new(move || {
        //         let peer_connection = peer_connection_clone.clone();
        //         let signaler = signaler_clone.clone();
        //         Box::pin(async move {
        //             // TODO: Figure a way to bubble the errors up
        //             // let offer = peer_connection
        //             //     .create_offer(None)
        //             //     .await
        //             //     .expect("Cannot create offer");
        //             // signaler
        //             //     .send_sdp(offer.clone())
        //             //     .await
        //             //     .expect("Cannot send offer");
        //             // peer_connection
        //             //     .set_local_description(offer)
        //             //     .await
        //             //     .expect("Cannot set local description");
        //             // let answer = signaler.recv_sdp().await.expect("Cannot receive answer");
        //             // peer_connection
        //             //     .set_remote_description(answer)
        //             //     .await
        //             //     .expect("Cannot set remote description");
        //         })
        //     }))
        //     .await;

        // peer_connection
        //     .on_peer_connection_state_change(Box::new(move |state| {
        //         // let signaler = signaler_clone.clone();
        //         Box::pin(async move { if state == RTCPeerConnectionState::Failed {} })
        //     }))
        //     .await;

        // Ok(StreamingServer {
        //     peer_connection,
        //     signaler: signaler,
        //     ice_gathering_complete,
        // })
        todo!()
    }
}

struct StreamingServerInternal {
    peer_connection: Arc<RTCPeerConnection>,
    signaler: Arc<dyn Signaler + Send + Sync>,
}

impl StreamingServerInternal {
    async fn initiate_connection(&self) -> Result<()> {
        let offer = self.peer_connection.create_offer(None).await?;
        self.peer_connection
            .set_local_description(offer.clone())
            .await?;
        self.signaler
            .send(Message::Sdp(offer.clone()))
            .await
            .expect("Error sending offer");
        Ok(())
    }
}

/// Sends audio + video, receives inputs
pub struct StreamingServer {
    peer_connection: Arc<RTCPeerConnection>,
    signaler: Arc<dyn Signaler + Send + Sync>,
    ice_gathering_complete: Arc<Notify>,
    // bandwidth_estimate: ??
}

impl StreamingServer {
    pub fn builder() -> StreamingServerBuilder {
        StreamingServerBuilder::new()
    }
}
