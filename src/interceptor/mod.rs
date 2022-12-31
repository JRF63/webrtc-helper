pub mod twcc;

use crate::util::data_rate::{twcc_bandwidth_estimate_channel, TwccBandwidthEstimate};
use twcc::TwccInterceptorBuilder;
use webrtc::{
    api::{interceptor_registry::configure_twcc, media_engine::MediaEngine},
    error::Result,
    interceptor::registry::Registry,
};

pub fn configure_custom_twcc(
    mut registry: Registry,
    media_engine: &mut MediaEngine,
) -> Result<(Registry, TwccBandwidthEstimate)> {
    let (tx, rx) = twcc_bandwidth_estimate_channel();
    let builder = TwccInterceptorBuilder::new(tx);
    registry.add(Box::new(builder));
    let registry = configure_twcc(registry, media_engine)?;
    Ok((registry, rx))
}
