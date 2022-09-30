pub mod twcc;

use twcc::{TwccBandwidthEstimate, TwccInterceptorBuilder};
use webrtc::{
    api::{interceptor_registry::configure_twcc, media_engine::MediaEngine},
    error::Result,
    interceptor::registry::Registry,
};

pub fn configure_twcc_capturer(
    registry: Registry,
    media_engine: &mut MediaEngine,
) -> Result<(Registry, TwccBandwidthEstimate)> {
    let mut registry = configure_twcc(registry, media_engine)?;
    let (builder, estimate) = TwccInterceptorBuilder::new();
    registry.add(Box::new(builder));
    Ok((registry, estimate))
}
