mod codec;
mod decoder;
mod encoder;
mod signaling;

use self::{decoder::MockDecoderBuilder, encoder::MockEncoderBuilder, signaling::MockSignaler};
use crate::peer::{Role, WebRtcBuilder};
use std::time::Duration;

#[tokio::test]
async fn mock_test() {
    let (encoder_signaler, decoder_signaler) = MockSignaler::channel();

    let handle_1 = tokio::spawn(async move {
        let mut encoder_builder = WebRtcBuilder::new(encoder_signaler, Role::Offerer);
        encoder_builder.with_encoder(Box::new(MockEncoderBuilder::new()));
        let encoder = encoder_builder.build().await.unwrap();
        for _ in 0..300 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        encoder.close().await;
    });

    let handle_2 = tokio::spawn(async move {
        let mut decoder_builder = WebRtcBuilder::new(decoder_signaler, Role::Answerer);
        decoder_builder.with_decoder(Box::new(MockDecoderBuilder::new()));
        let decoder = decoder_builder.build().await.unwrap();
        if !decoder.is_closed() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    let _ = handle_1.await;
    let _ = handle_2.await;
}
