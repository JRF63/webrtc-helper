use crate::{
    decoder::DecoderBuilder,
    encoder::{EncoderBuilder, EncoderTrackLocal},
    interceptor::configure_custom_twcc,
    signaling::{Message, Signaler},
    codecs::Codec
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Mutex;
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
    rtp_transceiver::{
        rtp_receiver::RTCRtpReceiver, rtp_transceiver_direction::RTCRtpTransceiverDirection,
        RTCRtpTransceiverInit,
    },
    track::track_remote::TrackRemote,
};

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
    mdns: bool,
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
            mdns: false,
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
            const DYNAMIC_PAYLOAD_TYPE_START: u8 = 96u8;

            let mut codecs = Vec::new();
            for encoder in self.encoders.iter() {
                codecs.extend_from_slice(encoder.supported_codecs());
            }
            for decoder in self.decoders.iter() {
                codecs.extend_from_slice(decoder.supported_codecs());
            }

            let mut payload_id = Some(DYNAMIC_PAYLOAD_TYPE_START);

            for codec in codecs {
                if let Some(payload_type) = payload_id {
                    let num_registered =
                        codec.register_to_media_engine(&mut media_engine, payload_type)?;
                    payload_id = payload_type.checked_add(num_registered);
                } else {
                    panic!("Registered too many codecs");
                }
            }

            if let Some(payload_type) = payload_id {
                // Needed for H264 playback some reason
                Codec::register_ulpfec(&mut media_engine, payload_type)?;
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

        let peer = Arc::new(WebRtcPeer {
            peer_connection: api_builder
                .new_peer_connection(RTCConfiguration {
                    ice_servers: self.ice_servers,
                    ..Default::default()
                })
                .await?,
            signaler: self.signaler,
            closed: AtomicBool::new(false),
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
                    }));
            }
            Role::Answerer => (),
        }

        let weak_ref = Arc::downgrade(&peer);
        peer.peer_connection
            .on_ice_candidate(Box::new(move |candidate| {
                let peer = weak_ref.clone();
                Box::pin(async move {
                    if let (Some(peer), Some(candidate)) = (peer.upgrade(), candidate) {
                        if let Ok(json) = candidate.to_json() {
                            let _ = peer.signaler.send(Message::IceCandidate(json)).await;
                        }
                    }
                })
            }));

        let peer_clone = peer.clone();
        tokio::spawn(async move {
            // TODO: swallow errors
            let peer = peer_clone;
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
                                .add_ice_candidate(candidate)
                                .await?;
                        }
                        Message::Bye => {
                            peer.close().await;
                            break;
                        }
                    }
                }
            }
            webrtc::error::Result::Ok(())
        });

        let decoders = Arc::new(Mutex::new(self.decoders));
        peer.peer_connection
            .on_track(Box::new(
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
                            if decoder.is_codec_supported(&codec) {
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
            if let Some(track) = EncoderTrackLocal::new(encoder_builder, bandwidth_estimate.clone())
            {
                let track = Arc::new(track);
                let transceiver = peer
                    .peer_connection
                    .add_transceiver_from_track(
                        track,
                        &[RTCRtpTransceiverInit {
                            direction: RTCRtpTransceiverDirection::Sendonly,
                            send_encodings: Vec::new(),
                        }],
                    )
                    .await?;

                if let Some(sender) = transceiver.sender().await {
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 1500];
                        while let Ok(_) = sender.read(&mut buf).await {}
                    });
                }
            } else {
                // TODO: log error
            }
        }

        // TODO: ICE restart

        Ok(peer)
    }
}

pub struct WebRtcPeer<S: Signaler + 'static> {
    peer_connection: RTCPeerConnection,
    signaler: S,
    closed: AtomicBool,
}

impl<S: Signaler + 'static> WebRtcPeer<S> {
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
