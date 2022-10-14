use async_trait::async_trait;
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidate,
    peer_connection::sdp::session_description::RTCSessionDescription,
};

#[async_trait]
pub trait SignalingChannel {
    async fn recv_sdp(&self) -> std::io::Result<RTCSessionDescription>;
    async fn send_sdp(&self, sdp: RTCSessionDescription) -> std::io::Result<()>;

    async fn recv_ice_candidate(&self) -> std::io::Result<RTCIceCandidate>;
    async fn send_ice_candidate(&self, ice_candidate: RTCIceCandidate) -> std::io::Result<()>;

    async fn signal_closed(&self);
    async fn is_closed(&self) -> bool;
}

pub enum Message {
    Sdp(RTCSessionDescription),
    IceCandidate(RTCIceCandidate),
    Bye,
}

#[async_trait]
pub trait Signaler {
    async fn recv(&self) -> std::io::Result<Message>;
    async fn send(&self, msg: Message) -> std::io::Result<()>;
}
