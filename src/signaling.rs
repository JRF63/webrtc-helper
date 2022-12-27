use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidate,
    peer_connection::sdp::session_description::RTCSessionDescription,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_serde() {
        let messages = [
            Message::Sdp(RTCSessionDescription::default()),
            Message::IceCandidate(RTCIceCandidate::default()),
            Message::Bye,
        ];
        for message in messages {
            let json = serde_json::to_string(&message).unwrap();
            println!("{json}");
            let _: Message = serde_json::from_str(&json).unwrap();
        }
        
    }
}
