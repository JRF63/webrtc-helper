use crate::{signaling::SignalingChannel, Result};
use std::sync::Arc;
use tokio::sync::Notify;
use webrtc::{
    api::{
        interceptor_registry::{
            configure_nack, configure_twcc_receiver_only,
        },
        media_engine::MediaEngine,
        APIBuilder,
    },
    ice_transport::{
        ice_connection_state::RTCIceConnectionState, ice_gatherer_state::RTCIceGathererState,
    },
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
    },
    rtp_transceiver::{
        rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
        rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCRtpTransceiverInit,
    },
};

pub struct StreamingClientBuilder {
    codecs: Vec<(RTCRtpCodecParameters, RTPCodecType)>,
    signaling_channel: Option<Arc<dyn SignalingChannel + Send + Sync>>,
}

impl StreamingClientBuilder {
    pub fn new() -> Self {
        StreamingClientBuilder {
            codecs: Vec::new(),
            signaling_channel: None,
        }
    }

    pub async fn build(self) -> Result<StreamingClient> {
        const DYNAMIC_PAYLOAD_TYPE_START: u8 = 96u8;

        let mut media_engine = MediaEngine::default();
        for (payload_type, (mut codec, codec_type)) in
            (DYNAMIC_PAYLOAD_TYPE_START..).zip(self.codecs)
        {
            codec.payload_type = payload_type;
            media_engine.register_codec(codec, codec_type)?;
        }

        let mut registry = Registry::new();
        registry = configure_nack(registry, &mut media_engine);
        registry = configure_twcc_receiver_only(registry, &mut media_engine)?;

        let api_builder = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let config = RTCConfiguration::default();
        let peer_connection = Arc::new(api_builder.new_peer_connection(config).await?);

        let signaling_channel = self.signaling_channel.expect("Signaling channel not set");

        let signaling_channel_clone = signaling_channel.clone();
        peer_connection
            .on_ice_candidate(Box::new(move |candidate| {
                #[cfg(debug_assertions)]
                println!("Peer B: Found ICE candidate:\n{:?}", &candidate);

                let signaling_channel = signaling_channel_clone.clone();
                Box::pin(async move {
                    if let Some(candidate) = candidate {
                        signaling_channel
                            .send_ice_candidate(candidate)
                            .await
                            .expect("Peer B: Unable to send ICE candidate");
                    }
                })
            }))
            .await;

        let signaling_channel_clone = signaling_channel.clone();
        peer_connection
            .on_ice_connection_state_change(Box::new(move |state| {
                let signaling_channel = signaling_channel_clone.clone();
                Box::pin(async move {
                    if state == RTCIceConnectionState::Failed {
                        signaling_channel.signal_closed().await;
                    }
                })
            }))
            .await;

        let ice_gathering_complete = Arc::new(Notify::new());
        let ice_gathering_complete_clone = ice_gathering_complete.clone();
        peer_connection
            .on_ice_gathering_state_change(Box::new(move |state| {
                if state == RTCIceGathererState::Complete {
                    ice_gathering_complete_clone.notify_one();
                }
                Box::pin(async {})
            }))
            .await;

        let peer_connection_clone = peer_connection.clone();
        let signaling_channel_clone = signaling_channel.clone();
        peer_connection
            .on_negotiation_needed(Box::new(move || {
                let peer_connection = peer_connection_clone.clone();
                let signaling_channel = signaling_channel_clone.clone();
                Box::pin(async move {
                    // TODO: Figure a way to bubble the errors up
                    let offer = signaling_channel
                        .recv_sdp()
                        .await
                        .expect("Cannot receive offer");
                    peer_connection
                        .set_remote_description(offer)
                        .await
                        .expect("Cannot set remote description");
                    let answer = peer_connection
                        .create_answer(None)
                        .await
                        .expect("Cannot create answer");
                    peer_connection
                        .set_local_description(answer)
                        .await
                        .expect("Cannot set local description");
                })
            }))
            .await;

        let signaling_channel_clone = signaling_channel.clone();
        peer_connection
            .on_peer_connection_state_change(Box::new(move |state| {
                let signaling_channel = signaling_channel_clone.clone();
                Box::pin(async move {
                    if state == RTCPeerConnectionState::Failed {
                        signaling_channel.signal_closed().await;
                    }
                })
            }))
            .await;

        // Exactly one each of audio and video receivers
        {
            peer_connection
                .add_transceiver_from_kind(
                    RTPCodecType::Video,
                    &[RTCRtpTransceiverInit {
                        direction: RTCRtpTransceiverDirection::Recvonly,
                        send_encodings: Vec::new(),
                    }],
                )
                .await?;
            peer_connection
                .add_transceiver_from_kind(
                    RTPCodecType::Audio,
                    &[RTCRtpTransceiverInit {
                        direction: RTCRtpTransceiverDirection::Recvonly,
                        send_encodings: Vec::new(),
                    }],
                )
                .await?;
        }

        // TODO:
        // tokio::spawn(async move {
        //     while let Some(candidate) = ice_rx.recv().await {
        //         let candidate = candidate
        //             .to_json()
        //             .await
        //             .expect("Peer B: `to_json` of `RTCIceCandidate` failed");
        //         pc.add_ice_candidate(candidate)
        //             .await
        //             .expect("Peer B: Unable to add ICE candidate");
        //     }
        // });

        Ok(StreamingClient {
            peer_connection,
            signaling_channel,
            ice_gathering_complete,
        })
    }
}

/// Receives audio + video, sends inputs
pub struct StreamingClient {
    peer_connection: Arc<RTCPeerConnection>,
    signaling_channel: Arc<dyn SignalingChannel + Send + Sync>,
    ice_gathering_complete: Arc<Notify>,
}

impl StreamingClient {
    pub fn builder() -> StreamingClientBuilder {
        StreamingClientBuilder::new()
    }

    pub async fn do_signaling(&self) -> Result<()> {
        let offer = self.peer_connection.create_offer(None).await?;
        self.signaling_channel
            .send_sdp(offer.clone())
            .await
            .expect("Cannot send offer");
        self.peer_connection.set_local_description(offer).await?;
        let answer = self
            .signaling_channel
            .recv_sdp()
            .await
            .expect("Cannot receive answer");
        self.peer_connection.set_remote_description(answer).await?;

        self.ice_gathering_complete.notified().await;
        Ok(())
    }
}
