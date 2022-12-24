use async_trait::async_trait;
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidate,
    peer_connection::sdp::session_description::RTCSessionDescription,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Sdp(RTCSessionDescription),
    IceCandidate(RTCIceCandidate),
    Bye,
}

#[async_trait]
pub trait Signaler: Send + Sync {
    async fn recv(&self) -> std::io::Result<Message>;
    async fn send(&self, msg: Message) -> std::io::Result<()>;
}
