use crate::{
    codecs::Codec,
    interceptor::{configure_custom_twcc, twcc::TwccBandwidthEstimate},
    signaling::{Message, Signaler},
    CustomTrackLocal,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use webrtc::{
    api::{
        interceptor_registry::{configure_nack, configure_rtcp_reports},
        media_engine::MediaEngine,
        setting_engine::SettingEngine,
        APIBuilder,
    },
    ice::mdns::MulticastDnsMode,
    ice_transport::ice_server::RTCIceServer,
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, sdp::sdp_type::RTCSdpType, RTCPeerConnection,
    },
};

pub enum Role {
    Offerer,
    Answerer,
}

pub struct WebRtcBuilder<S: Signaler + Send + Sync + 'static> {
    signaler: S,
    role: Role,
    receivable_codecs: Vec<Codec>,
    sendable_tracks: Vec<CustomTrackLocal>,
    ice_servers: Vec<RTCIceServer>,
    mdns: bool,
}

impl<S: Signaler + Send + Sync + 'static> WebRtcBuilder<S> {
    pub fn new(signaler: S, role: Role) -> Self {
        WebRtcBuilder {
            signaler,
            role,
            receivable_codecs: Vec::new(),
            sendable_tracks: Vec::new(),
            ice_servers: Vec::new(),
            mdns: false,
        }
    }

    pub async fn build(mut self) -> webrtc::error::Result<Arc<WebRtc<S>>> {
        let mut media_engine = MediaEngine::default();
        {
            const DYNAMIC_PAYLOAD_TYPE_START: u8 = 96u8;

            let mut payload_id = Some(DYNAMIC_PAYLOAD_TYPE_START);

            let mut codecs = Vec::new();
            codecs.append(&mut self.receivable_codecs);

            for track in self.sendable_tracks.iter() {
                codecs.extend_from_slice(track.supported_codecs());
            }

            for mut codec in codecs {
                if let Some(payload_type) = payload_id {
                    codec.set_payload_type(payload_type);
                    let num_registered = codec.register_to_media_engine(&mut media_engine)?;
                    payload_id = payload_type.checked_add(num_registered);
                } else {
                    panic!("Registered too many codecs");
                }
            }
        }

        let registry = configure_nack(Registry::new(), &mut media_engine);
        let registry = configure_rtcp_reports(registry);
        let (registry, bandwidth_estimate) = configure_custom_twcc(registry, &mut media_engine)?;

        let mut setting_engine = SettingEngine::default();
        if self.mdns {
            setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::QueryAndGather);
        }

        let api_builder = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        let peer = Arc::new(WebRtc {
            peer_connection: api_builder
                .new_peer_connection(RTCConfiguration {
                    ice_servers: self.ice_servers,
                    ..Default::default()
                })
                .await?,
            signaler: self.signaler,
            closed: AtomicBool::new(false),
            bandwidth_estimate,
        });

        match self.role {
            Role::Offerer => {
                let weak_ref = Arc::downgrade(&peer);
                peer.peer_connection
                    .on_negotiation_needed(Box::new(move || {
                        let peer = weak_ref.clone();
                        Box::pin(async move {
                            if let Some(peer) = peer.upgrade() {
                                peer.start_negotiation().await;
                            }
                        })
                    }))
                    .await;
            }
            Role::Answerer => (),
        }

        let weak_ref = Arc::downgrade(&peer);
        peer.peer_connection
            .on_ice_candidate(Box::new(move |candidate| {
                let peer = weak_ref.clone();
                Box::pin(async move {
                    if let (Some(peer), Some(candidate)) = (peer.upgrade(), candidate) {
                        let _ = peer.signaler.send(Message::IceCandidate(candidate)).await;
                    }
                })
            }))
            .await;

        let peer_clone = peer.clone();
        tokio::spawn(async move {
            // TODO: swallow errors
            let peer = peer_clone.clone();
            while !peer.is_closed() {
                if let Ok(msg) = peer.signaler.recv().await {
                    match msg {
                        Message::Sdp(sdp) => {
                            let sdp_type = sdp.sdp_type;
                            peer.peer_connection.set_remote_description(sdp).await?;
                            if sdp_type == RTCSdpType::Offer {
                                let answer = peer.peer_connection.create_answer(None).await?;
                                peer.peer_connection
                                    .set_local_description(answer.clone())
                                    .await?;
                                let _ = peer.signaler.send(Message::Sdp(answer)).await;
                            }
                        }
                        Message::IceCandidate(candidate) => {
                            peer.peer_connection
                                .add_ice_candidate(candidate.to_json().await?)
                                .await?;
                        }
                        Message::Bye => break,
                    }
                }
            }
            webrtc::error::Result::Ok(())
        });

        // TODO: ICE restart

        // for (track, _) in self.tracks {
        //     // TODO: tokio::spawn a handler
        //     let _rtp_sender = peer_connection.add_track(track as _).await?;
        // }

        Ok(peer)
    }
}

pub struct WebRtc<S: Signaler + Send + Sync + 'static> {
    peer_connection: RTCPeerConnection,
    signaler: S,
    closed: AtomicBool,
    bandwidth_estimate: TwccBandwidthEstimate,
}

impl<S: Signaler + Send + Sync + 'static> WebRtc<S> {
    pub fn builder(signaler: S, role: Role) -> WebRtcBuilder<S> {
        WebRtcBuilder::new(signaler, role)
    }

    pub async fn close(&self) {
        let _ = self.signaler.send(Message::Bye).await;
        self.closed.store(true, Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub async fn start_negotiation(&self) {
        if let Ok(offer) = self.peer_connection.create_offer(None).await {
            let _ = self
                .peer_connection
                .set_local_description(offer.clone())
                .await;
            let _ = self.signaler.send(Message::Sdp(offer)).await;
        };
    }
}
