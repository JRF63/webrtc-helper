pub mod twcc;

use crate::mpsc;
use twcc::sender::{TwccCapturerBuilder, TwccData};
use webrtc::{
    api::{interceptor_registry::configure_twcc_sender_only, media_engine::MediaEngine},
    error::Result,
    interceptor::registry::Registry,
};

pub fn configure_twcc_capturer(
    registry: Registry,
    media_engine: &mut MediaEngine,
) -> Result<(Registry, mpsc::Receiver<TwccData>)> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let mut registry = configure_twcc_sender_only(registry, media_engine)?;
    registry.add(Box::new(TwccCapturerBuilder::with_seq_num_sender(tx)));
    Ok((registry, rx))
}
