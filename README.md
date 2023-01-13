# webrtc-helper

Wrapper for [webrtc-rs](https://github.com/webrtc-rs/webrtc) to facilitate custom encoders/decoders. It abstracts away the boilerplate-y code required to initiate a WebRTC connection through a few key traits.

Primary requirement is a signaling channel through which the SDP and ICE candidates are exchanged:

```rust
#[async_trait]
pub trait Signaler: Send + Sync {
    type Error: Send + std::fmt::Display;

    async fn recv(&self) -> Result<Message, Self::Error>;

    async fn send(&self, msg: Message) -> Result<(), Self::Error>;
}
```

## Features

Includes an implementation of [Transport-Wide Congestion Control](src/interceptor/twcc/mod.rs).