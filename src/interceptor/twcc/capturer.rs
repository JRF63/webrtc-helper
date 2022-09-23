use async_trait::async_trait;
use std::{sync::Arc, time::SystemTime};
use webrtc::{
    interceptor::{
        stream_info::StreamInfo, Attributes, Error, Interceptor,
        InterceptorBuilder, RTCPReader, RTCPWriter, RTPReader, RTPWriter,
    },
    rtp::{self, extension::transport_cc_extension::TransportCcExtension},
    util::Unmarshal,
};
use super::TwccDataMap;

struct TwccExtensionCapturerStream {
    map: TwccDataMap,
    hdr_ext_id: u8,
}

#[async_trait]
impl RTPWriter for TwccExtensionCapturerStream {
    async fn write(
        &self,
        pkt: &rtp::packet::Packet,
        _attributes: &Attributes,
    ) -> Result<usize, Error> {
        let mut buf = pkt
            .header
            .get_extension(self.hdr_ext_id)
            .expect("`TwccExtensionCapturerStream` must run after `TransportCcExtension` has been set");

        let tcc_ext = TransportCcExtension::unmarshal(&mut buf)?;
        if let Ok(mut map) = self.map.lock() {
            map.insert(tcc_ext.transport_sequence as _, SystemTime::now());
        }
        Ok(0)
    }
}

pub struct TwccExtensionCapturer {
    map: TwccDataMap,
}

#[async_trait]
impl Interceptor for TwccExtensionCapturer {
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync> {
        reader
    }

    async fn bind_rtcp_writer(
        &self,
        writer: Arc<dyn RTCPWriter + Send + Sync>,
    ) -> Arc<dyn RTCPWriter + Send + Sync> {
        writer
    }

    async fn bind_local_stream(
        &self,
        info: &StreamInfo,
        writer: Arc<dyn RTPWriter + Send + Sync>,
    ) -> Arc<dyn RTPWriter + Send + Sync> {
        const TRANSPORT_CC_URI: &str =
            "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

        let mut hdr_ext_id = 0u8;
        for e in &info.rtp_header_extensions {
            if e.uri == TRANSPORT_CC_URI {
                hdr_ext_id = e.id as u8;
                break;
            }
        }
        if hdr_ext_id == 0 {
            return writer;
        }
        Arc::new(TwccExtensionCapturerStream {
            map: self.map.clone(),
            hdr_ext_id,
        })
    }

    async fn unbind_local_stream(&self, _info: &StreamInfo) {}

    async fn bind_remote_stream(
        &self,
        _info: &StreamInfo,
        reader: Arc<dyn RTPReader + Send + Sync>,
    ) -> Arc<dyn RTPReader + Send + Sync> {
        reader
    }

    async fn unbind_remote_stream(&self, _info: &StreamInfo) {}

    async fn close(&self) -> Result<(), Error> {
        Ok(())
    }
}

pub struct TwccExtensionCapturerBuilder {
    map: TwccDataMap,
}

impl TwccExtensionCapturerBuilder {
    pub fn with_map(map: TwccDataMap) -> Self {
        Self { map }
    }
}

impl InterceptorBuilder for TwccExtensionCapturerBuilder {
    fn build(&self, _id: &str) -> Result<Arc<dyn Interceptor + Send + Sync>, Error> {
        Ok(Arc::new(TwccExtensionCapturer {
            map: self.map.clone(),
        }))
    }
}
