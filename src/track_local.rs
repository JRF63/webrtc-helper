use std::any::Any;

use async_trait::async_trait;
use webrtc::{
    error::Result,
    rtp_transceiver::rtp_codec::{RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{TrackLocal, TrackLocalContext},
};

pub struct CustomTrackLocal {
    id: String,
    stream_id: String,
}

#[async_trait]
impl TrackLocal for CustomTrackLocal {
    async fn bind(&self, t: &TrackLocalContext) -> Result<RTCRtpCodecParameters> {
        todo!()
    }

    async fn unbind(&self, t: &TrackLocalContext) -> Result<()> {
        todo!()
    }

    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn stream_id(&self) -> &str {
        self.stream_id.as_str()
    }

    fn kind(&self) -> RTPCodecType {
        todo!()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
