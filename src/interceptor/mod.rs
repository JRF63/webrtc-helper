pub mod twcc;

use twcc::{capturer::TwccExtensionCapturerBuilder, handler::TwccRtcpHandlerBuilder};
use webrtc::{
    api::{interceptor_registry::configure_twcc, media_engine::MediaEngine},
    error::Result,
    interceptor::registry::Registry,
};
use std::{sync::{Arc, Mutex}, collections::BTreeMap};

pub fn configure_twcc_capturer(
    registry: Registry,
    media_engine: &mut MediaEngine,
) -> Result<Registry> {
    let map = Arc::new(Mutex::new(BTreeMap::new()));
    let mut registry = configure_twcc(registry, media_engine)?;
    registry.add(Box::new(TwccExtensionCapturerBuilder::with_map(
        map.clone(),
    )));
    registry.add(Box::new(TwccRtcpHandlerBuilder::with_map(map)));
    Ok(registry)
}
