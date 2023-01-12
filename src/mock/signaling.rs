use crate::signaling::{Message, Signaler};
use async_trait::async_trait;
use tokio::sync::{mpsc::{UnboundedSender, UnboundedReceiver, unbounded_channel}, Mutex};

pub struct MockSignaler {
    tx: UnboundedSender<Message>,
    rx: Mutex<UnboundedReceiver<Message>>,
}

#[async_trait]
impl Signaler for MockSignaler {
    type Error = std::io::Error;

    async fn recv(&self) -> Result<Message, Self::Error> {
        let mut lock = self.rx.lock().await;
        let msg = lock.recv().await;
        msg.ok_or(std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
    }

    async fn send(&self, msg: Message) -> Result<(), Self::Error> {
        self.tx.send(msg).map_err(|_| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
    }
}

impl MockSignaler {
    pub fn channel() -> (Self, Self) {
        let (tx1, rx1) = unbounded_channel();
        let (tx2, rx2) = unbounded_channel();
        let a = MockSignaler {
            tx: tx1,
            rx: Mutex::new(rx2),
        };
        let b = MockSignaler {
            tx: tx2,
            rx: Mutex::new(rx1),
        };
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
    use super::*;

    #[tokio::test]
    async fn signaler_test() {
        let (a, b) = MockSignaler::channel();

        tokio::spawn(async move {
            let msg = Message::IceCandidate(RTCIceCandidateInit {
                candidate: "test".to_owned(),
                ..RTCIceCandidateInit::default()
            });
            a.send(msg).await.unwrap();
        });

        tokio::spawn(async move {
            let msg = b.recv().await.unwrap();
            if let Message::IceCandidate(c) = msg {
                assert_eq!(c.candidate, "test");
            }
        });
    }
}