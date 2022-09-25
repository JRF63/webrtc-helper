use super::data::TwccDataMap;
use async_trait::async_trait;
use std::{sync::Arc, time::Instant};
use webrtc::{
    interceptor::{Attributes, Error, RTPWriter},
    rtp::{self, extension::transport_cc_extension::TransportCcExtension},
    util::Unmarshal,
};

pub struct TwccTimestampSenderStream {
    map: TwccDataMap,
    hdr_ext_id: u8,
    next_writer: Arc<dyn RTPWriter + Send + Sync>,
    start_time: Instant,
}

impl TwccTimestampSenderStream {
    pub fn new(
        map: TwccDataMap,
        hdr_ext_id: u8,
        next_writer: Arc<dyn RTPWriter + Send + Sync>,
        start_time: Instant,
    ) -> Self {
        Self {
            map,
            hdr_ext_id,
            next_writer,
            start_time,
        }
    }
}

#[async_trait]
impl RTPWriter for TwccTimestampSenderStream {
    async fn write(
        &self,
        pkt: &rtp::packet::Packet,
        attributes: &Attributes,
    ) -> Result<usize, Error> {
        // `TwccExtensionCapturerStream` must run after `TransportCcExtension` has been set
        if let Some(mut buf) = pkt.header.get_extension(self.hdr_ext_id) {
            let tcc_ext = TransportCcExtension::unmarshal(&mut buf)?;

            let timestamp = {
                const REFERENCE_TIME_WRAPAROUND: u128 = (1 << 24) * 64000;
                let duration = Instant::now().duration_since(self.start_time).as_micros();
                // Imitate the RTCP reference time having 24 bits for easy comparison
                (duration % REFERENCE_TIME_WRAPAROUND) as i64
            };

            self.map[tcc_ext.transport_sequence].store(timestamp);
        }
        self.next_writer.write(pkt, attributes).await
    }
}
