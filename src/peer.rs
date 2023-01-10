use crate::{
    codecs::{Codec, MediaEngineExt},
    decoder::DecoderBuilder,
    encoder::{EncoderBuilder, EncoderTrackLocal},
    interceptor::configure_custom_twcc,
    signaling::{Message, Signaler},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{watch, Mutex, Notify};
use webrtc::{
    api::{
        interceptor_registry::{configure_nack, configure_rtcp_reports},
        media_engine::MediaEngine,
        setting_engine::SettingEngine,
        APIBuilder,
    },
    ice::mdns::MulticastDnsMode,
    ice_transport::{ice_connection_state::RTCIceConnectionState, ice_server::RTCIceServer},
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, offer_answer_options::RTCOfferOptions,
        sdp::sdp_type::RTCSdpType, signaling_state::RTCSignalingState, RTCPeerConnection,
    },
    rtp_transceiver::rtp_receiver::RTCRtpReceiver,
    track::track_remote::TrackRemote,
};

/// Used for querying `RTCIceConnectionState` in the encoders/decoders.
pub type IceConnectionState = watch::Receiver<RTCIceConnectionState>;

/// Determines if the peer will offer or wait for an SDP.
///
/// The role of each peer needs to be specified at the start since the `webrtc` crate does not
/// support any form of rollback and cannot use ["perfect negotiation"][PN].
///
/// [PN]: https://developer.mozilla.org/en-US/docs/Web/API/WebRTC_API/Perfect_negotiation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Offerer,
    Answerer,
}

pub struct WebRtcBuilder<S>
where
    S: Signaler + 'static,
{
    signaler: S,
    role: Role,
    encoders: Vec<Box<dyn EncoderBuilder>>,
    decoders: Vec<Box<dyn DecoderBuilder>>,
    ice_servers: Vec<RTCIceServer>,
}

impl<S> WebRtcBuilder<S>
where
    S: Signaler + 'static,
{
    pub fn new(signaler: S, role: Role) -> Self {
        WebRtcBuilder {
            signaler,
            role,
            encoders: Vec::new(),
            decoders: Vec::new(),
            ice_servers: Vec::new(),
        }
    }

    pub fn with_encoder(&mut self, encoder: Box<dyn EncoderBuilder>) -> &mut Self {
        self.encoders.push(encoder);
        self
    }

    pub fn with_decoder(&mut self, decoder: Box<dyn DecoderBuilder>) -> &mut Self {
        self.decoders.push(decoder);
        self
    }

    pub fn with_ice_server(&mut self, ice_server: RTCIceServer) -> &mut Self {
        self.ice_servers.push(ice_server);
        self
    }

    pub async fn build(self) -> webrtc::error::Result<Arc<WebRtcPeer<S>>> {
        let mut media_engine = MediaEngine::default();
        {
            let mut codecs = Vec::new();
            for encoder in self.encoders.iter() {
                codecs.extend_from_slice(encoder.supported_codecs());
            }
            for decoder in self.decoders.iter() {
                codecs.extend_from_slice(decoder.supported_codecs());
            }

            Self::register_codecs(codecs, &mut media_engine)?;
        }

        let registry = configure_nack(Registry::new(), &mut media_engine);
        let registry = configure_rtcp_reports(registry);
        let (registry, bandwidth_estimate) = configure_custom_twcc(registry, &mut media_engine)?;

        let mut setting_engine = SettingEngine::default();

        // Leave mDNS disabled on debug builds because webrtc-rs does not handle it properly when
        // communicating with another webrtc-rs instance
        #[cfg(debug_assertions)]
        setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::Unspecified);

        // Enabling mDNS hides local IP addresses
        #[cfg(not(debug_assertions))]
        setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::QueryAndGather);

        let api_builder = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        let peer = Arc::new(WebRtcPeer {
            pc: api_builder
                .new_peer_connection(RTCConfiguration {
                    ice_servers: self.ice_servers,
                    ..Default::default()
                })
                .await?,
            signaler: self.signaler,
            closed: Notify::new(),
        });

        match self.role {
            Role::Offerer => {
                let weak_ref = Arc::downgrade(&peer);
                peer.pc.on_negotiation_needed(Box::new(move || {
                    let peer = weak_ref.clone();
                    Box::pin(async move {
                        if let Some(peer) = peer.upgrade() {
                            if let Err(e) = peer.start_negotiation(false).await {
                                panic!("{e}");
                            }
                        }
                    })
                }));
            }
            Role::Answerer => (),
        }

        let weak_ref = Arc::downgrade(&peer);
        peer.pc.on_ice_candidate(Box::new(move |candidate| {
            let peer = weak_ref.clone();
            Box::pin(async move {
                if let (Some(peer), Some(candidate)) = (peer.upgrade(), candidate) {
                    if let Ok(json) = candidate.to_json() {
                        let _ = peer.signaler.send(Message::IceCandidate(json)).await;
                    }
                }
            })
        }));

        let (ice_tx, ice_rx) = watch::channel(RTCIceConnectionState::default());
        let weak_ref = Arc::downgrade(&peer);
        peer.pc
            .on_ice_connection_state_change(Box::new(move |state| {
                let _ = ice_tx.send(state);
                let peer = weak_ref.clone();
                Box::pin(async move {
                    if state == RTCIceConnectionState::Failed {
                        match self.role {
                            Role::Offerer => {
                                if let Some(peer) = peer.upgrade() {
                                    // TODO: Test ICE restart
                                    if let Err(e) = peer.start_negotiation(true).await {
                                        panic!("{e}");
                                    }
                                }
                            }
                            Role::Answerer => (), // Offerer should be the one to initiate ICE restart
                        }
                    }
                })
            }));

        let peer_clone = peer.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::handle_signaler_message(peer_clone).await {
                panic!("{e}");
            }
        });

        let decoders = Arc::new(Mutex::new(self.decoders));
        peer.pc.on_track(Box::new(
            move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                let (Some(track), Some(receiver)) = (track, receiver) else {
                        return Box::pin(async move {});
                    };

                let decoders = decoders.clone();

                Box::pin(async move {
                    let codec = track.codec().await;
                    let mut decoders = decoders.lock().await;
                    let mut matched_index = None;
                    for (index, decoder) in decoders.iter().enumerate() {
                        if decoder.is_codec_supported(&codec.capability) {
                            matched_index = Some(index);
                        }
                    }
                    if let Some(index) = matched_index {
                        let decoder = decoders.swap_remove(index);
                        decoder.build(track, receiver);
                    }
                })
            },
        ));

        for encoder_builder in self.encoders {
            let track =
                EncoderTrackLocal::new(encoder_builder, ice_rx.clone(), bandwidth_estimate.clone())
                    .await;
            let track = Arc::new(track);
            track.add_as_transceiver(&peer.pc).await?;
        }

        Ok(peer)
    }

    fn register_codecs(
        codecs: Vec<Codec>,
        media_engine: &mut MediaEngine,
    ) -> Result<(), webrtc::Error> {
        const DYNAMIC_PAYLOAD_TYPE_START: u8 = 96u8;

        let mut payload_id = Some(DYNAMIC_PAYLOAD_TYPE_START);

        for mut codec in codecs {
            if let Some(payload_type) = payload_id {
                codec.set_payload_type(payload_type);
                media_engine.register_custom_codec(codec.clone())?;
                payload_id = payload_type.checked_add(1);

                // Register for retransmission
                if let Some(mut retransmission) = Codec::retransmission(&codec) {
                    if let Some(payload_type) = payload_id {
                        retransmission.set_payload_type(payload_type);
                        media_engine.register_custom_codec(retransmission)?;
                        payload_id = payload_type.checked_add(1);
                    } else {
                        panic!("Not enough payload type for video retransmission");
                    }
                }
            } else {
                panic!("Registered too many codecs");
            }
        }

        if let Some(payload_type) = payload_id {
            // Needed for playback of non-constrained-baseline H264 for some reason
            let mut ulpfec = Codec::ulpfec();
            ulpfec.set_payload_type(payload_type);
            media_engine.register_custom_codec(ulpfec)?;
        } else {
            panic!("Not enough payload type for ULPFEC");
        }

        Ok(())
    }

    // Implements the impolite peer of "perfect negotiation".
    async fn handle_signaler_message(peer: Arc<WebRtcPeer<S>>) -> Result<(), webrtc::Error> {
        loop {
            if let Ok(msg) = peer.signaler.recv().await {
                match msg {
                    Message::Sdp(sdp) => {
                        let sdp_type = sdp.sdp_type;

                        if sdp_type == RTCSdpType::Offer
                            && peer.pc.signaling_state() != RTCSignalingState::Stable
                        {
                            continue;
                        }

                        peer.pc.set_remote_description(sdp).await?;
                        if sdp_type == RTCSdpType::Offer {
                            let answer = peer.pc.create_answer(None).await?;
                            peer.pc.set_local_description(answer.clone()).await?;
                            let _ = peer.signaler.send(Message::Sdp(answer)).await;
                        }
                    }
                    Message::IceCandidate(candidate) => {
                        peer.pc.add_ice_candidate(candidate).await?;
                    }
                    Message::Bye => {
                        peer.close().await;
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct WebRtcPeer<S: Signaler + 'static> {
    pc: RTCPeerConnection,
    signaler: S,
    closed: Notify,
}

impl<S: Signaler + 'static> WebRtcPeer<S> {
    pub fn builder(signaler: S, role: Role) -> WebRtcBuilder<S> {
        WebRtcBuilder::new(signaler, role)
    }

    pub async fn close(&self) {
        let _ = self.signaler.send(Message::Bye).await;
        self.closed.notify_waiters();
    }

    pub async fn is_closed(&self) {
        self.closed.notified().await;
    }

    pub async fn start_negotiation(&self, ice_restart: bool) -> Result<(), webrtc::Error> {
        let options = if ice_restart {
            Some(RTCOfferOptions {
                voice_activity_detection: false, // Seems unused
                ice_restart: true,
            })
        } else {
            None
        };

        let offer = self.pc.create_offer(options).await?;
        self.pc.set_local_description(offer.clone()).await?;
        self.signaler
            .send(Message::Sdp(offer))
            .await
            .map_err(|_| webrtc::Error::ErrUnknownType)?;
        Ok(())
    }
}
