use super::{estimator::TwccEstimatorStream, sender::TwccTimestampSenderStream, data::TwccSendInfo};
use async_trait::async_trait;
use std::{sync::Arc, time::Instant};
use webrtc::interceptor::{
    stream_info::StreamInfo, Error, Interceptor, InterceptorBuilder, RTCPReader, RTCPWriter,
    RTPReader, RTPWriter,
};

pub struct TwccInterceptor {
    map: TwccSendInfo,
    start_time: Instant,
}

#[async_trait]
impl Interceptor for TwccInterceptor {
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync> {
        Arc::new(TwccEstimatorStream::new(self.map.clone(), reader))
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
        Arc::new(TwccTimestampSenderStream::new(
            self.map.clone(),
            hdr_ext_id,
            writer,
            self.start_time,
        ))
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

pub struct TwccInterceptorBuilder {
    map: TwccSendInfo,
}

impl TwccInterceptorBuilder {
    pub fn new() -> TwccInterceptorBuilder {
        TwccInterceptorBuilder { map: TwccSendInfo::new() }
    }
}

impl InterceptorBuilder for TwccInterceptorBuilder {
    fn build(&self, _id: &str) -> Result<Arc<dyn Interceptor + Send + Sync>, Error> {
        Ok(Arc::new(TwccInterceptor {
            map: self.map.clone(),
            start_time: Instant::now(),
        }))
    }
}
