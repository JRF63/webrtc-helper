mod client;
mod server;
mod codecs;
mod signaling;
mod interceptor;
mod mpsc;

use webrtc::error::Result;

pub use server::StreamingServer;
pub use client::StreamingClient;