mod client;
mod codecs;
mod interceptor;
mod mpsc;
mod server;
mod signaling;

use webrtc::error::Result;

pub use client::StreamingClient;
pub use server::StreamingServer;
pub use codecs::{Codec};