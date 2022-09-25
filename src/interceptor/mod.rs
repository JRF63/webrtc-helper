pub mod twcc;

use twcc::TwccBandwidthEstimatorBuilder;
use webrtc::{
    api::{interceptor_registry::configure_twcc, media_engine::MediaEngine},
    error::Result,
    interceptor::registry::Registry,
};

pub fn configure_twcc_capturer(
    registry: Registry,
    media_engine: &mut MediaEngine,
) -> Result<Registry> {
    let mut registry = configure_twcc(registry, media_engine)?;
    registry.add(Box::new(TwccBandwidthEstimatorBuilder::new()));
    Ok(registry)
}
