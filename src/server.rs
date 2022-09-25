use crate::{
    interceptor::configure_twcc_capturer,
    signaling::{Message, Signaler},
    Result,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use webrtc::{
    api::{
        interceptor_registry::configure_nack, media_engine::MediaEngine,
        setting_engine::SettingEngine, APIBuilder,
    },
    ice::mdns::MulticastDnsMode,
    ice_transport::ice_server::RTCIceServer,
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, sdp::sdp_type::RTCSdpType, RTCPeerConnection,
    },
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::track_local_static_rtp::TrackLocalStaticRTP,
};

pub struct StreamingServerBuilder<S: Signaler + Send + Sync + 'static> {
    tracks: Vec<(Arc<TrackLocalStaticRTP>, RTPCodecType)>,
    signaler: S,
    ice_servers: Vec<RTCIceServer>,
    mdns: bool,
}

impl<S: Signaler + Send + Sync + 'static> StreamingServerBuilder<S> {
    pub fn new(signaler: S) -> Self {
        StreamingServerBuilder {
            tracks: Vec::new(),
            signaler,
            ice_servers: Vec::new(),
            mdns: false,
        }
    }

    pub async fn build(self) -> Result<Arc<StreamingServer<S>>> {
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
        let registry = configure_twcc_capturer(registry, &mut media_engine)?;

        let mut setting_engine = SettingEngine::default();
        if self.mdns {
            setting_engine.set_ice_multicast_dns_mode(MulticastDnsMode::QueryAndGather);
        }

        let api_builder = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .with_setting_engine(setting_engine)
            .build();

        let streaming_server = Arc::new(StreamingServer {
            peer_connection: api_builder
                .new_peer_connection(RTCConfiguration {
                    ice_servers: self.ice_servers,
                    ..Default::default()
                })
                .await?,
            signaler: self.signaler,
            closed: AtomicBool::new(false),
        });

        let streaming_serve_clone = streaming_server.clone();
        streaming_server
            .peer_connection
            .on_negotiation_needed(Box::new(move || {
                let streaming_server = streaming_serve_clone.clone();
                Box::pin(async move {
                    if let Ok(offer) = streaming_server.peer_connection.create_offer(None).await {
                        let _ = streaming_server
                            .peer_connection
                            .set_local_description(offer.clone())
                            .await;
                        let _ = streaming_server
                            .signaler
                            .send(Message::Sdp(offer))
                            .await;
                    }
                })
            }))
            .await;

        let streaming_server_clone = streaming_server.clone();
        streaming_server
            .peer_connection
            .on_ice_candidate(Box::new(move |candidate| {
                let streaming_server = streaming_server_clone.clone();
                Box::pin(async move {
                    if let Some(candidate) = candidate {
                        streaming_server
                            .signaler
                            .send(Message::IceCandidate(candidate))
                            .await
                            .expect("Peer A: Unable to send ICE candidate");
                    }
                })
            }))
            .await;

        let streaming_server_clone = streaming_server.clone();
        tokio::spawn(async move {
            // TODO: swallow errors
            let streaming_server = streaming_server_clone.clone();
            while !streaming_server.is_closed() {
                if let Ok(msg) = streaming_server.signaler.recv().await {
                    match msg {
                        Message::Sdp(sdp) => {
                            let sdp_type = sdp.sdp_type;
                            streaming_server
                                .peer_connection
                                .set_remote_description(sdp)
                                .await?;
                            if sdp_type == RTCSdpType::Offer {
                                let answer =
                                    streaming_server.peer_connection.create_answer(None).await?;
                                streaming_server
                                    .peer_connection
                                    .set_local_description(answer.clone())
                                    .await?;
                                let _ = streaming_server.signaler.send(Message::Sdp(answer)).await;
                            }
                        }
                        Message::IceCandidate(candidate) => {
                            streaming_server
                                .peer_connection
                                .add_ice_candidate(candidate.to_json().await?)
                                .await?;
                        }
                        Message::Bye => break,
                    }
                }
            }
            Result::Ok(())
        });

        // TODO: ICE restart

        // for (track, _) in self.tracks {
        //     // TODO: tokio::spawn a handler
        //     let _rtp_sender = peer_connection.add_track(track as _).await?;
        // }

        Ok(streaming_server)
    }
}

pub struct StreamingServer<S: Signaler + Send + Sync + 'static> {
    peer_connection: RTCPeerConnection,
    signaler: S,
    closed: AtomicBool,
    // bandwidth_estimate: ??
}

impl<S: Signaler + Send + Sync + 'static> StreamingServer<S> {
    pub fn builder() -> StreamingServerBuilder<S> {
        todo!()
    }

    pub async fn close(&self) {
        let _ = self.signaler.send(Message::Bye).await;
        self.closed.store(true, Ordering::Release);
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}
