use crate::mpsc;
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

pub struct TwccData {
    sequence_number: u16,
    send_time: SystemTime,
}

struct TwccCapturerStream {
    seq_num_sender: mpsc::Sender<TwccData>,
    hdr_ext_id: u8,
}

#[async_trait]
impl RTPWriter for TwccCapturerStream {
    async fn write(
        &self,
        pkt: &rtp::packet::Packet,
        _attributes: &Attributes,
    ) -> Result<usize, Error> {
        let mut buf = pkt
            .header
            .get_extension(self.hdr_ext_id)
            .expect("`TwccCapturerStream` must run after `TransportCcExtension` has been set");

        let tcc_ext = TransportCcExtension::unmarshal(&mut buf)?;
        let _ = self.seq_num_sender.send(TwccData {
            sequence_number: tcc_ext.transport_sequence,
            send_time: SystemTime::now(),
        });
        Ok(0)
    }
}

pub struct TwccCapturer {
    seq_num_sender: mpsc::Sender<TwccData>,
}

#[async_trait]
impl Interceptor for TwccCapturer {
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
        Arc::new(TwccCapturerStream {
            seq_num_sender: self.seq_num_sender.clone(),
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

pub struct TwccCapturerBuilder {
    seq_num_sender: mpsc::Sender<TwccData>,
}

impl TwccCapturerBuilder {
    pub fn with_seq_num_sender(seq_num_sender: mpsc::Sender<TwccData>) -> Self {
        TwccCapturerBuilder { seq_num_sender }
    }
}

impl InterceptorBuilder for TwccCapturerBuilder {
    fn build(&self, _id: &str) -> Result<Arc<dyn Interceptor + Send + Sync>, Error> {
        Ok(Arc::new(TwccCapturer {
            seq_num_sender: self.seq_num_sender.clone(),
        }))
    }
}
