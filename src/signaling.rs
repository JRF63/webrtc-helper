use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidateInit,
    peer_connection::sdp::session_description::RTCSessionDescription,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Message {
    Sdp(RTCSessionDescription),
    IceCandidate(RTCIceCandidateInit),
    Bye,
}

#[async_trait]
pub trait Signaler: Send + Sync {
    type Error: Send + std::fmt::Display;

    async fn recv(&self) -> Result<Message, Self::Error>;
    async fn send(&self, msg: Message) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_serde() {
        let messages = [
            Message::Sdp(RTCSessionDescription::default()),
            Message::IceCandidate(RTCIceCandidateInit::default()),
            Message::Bye,
        ];
        for message in messages {
            let json = serde_json::to_string(&message).unwrap();
            println!("{json}");
            let _: Message = serde_json::from_str(&json).unwrap();
        }
        
    }
}
