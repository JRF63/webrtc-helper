use super::{
    estimator::TwccBandwidthEstimator,
    sender::TwccTimestampSenderStream,
    sync::{TwccBandwidthEstimate, TwccSendInfo},
};
use async_trait::async_trait;
use std::{
    sync::{Arc, Mutex},
    time::{Instant, SystemTime},
};
use webrtc::{
    interceptor::{
        stream_info::StreamInfo, Attributes, Error, Interceptor, InterceptorBuilder, RTCPReader,
        RTCPWriter, RTPReader, RTPWriter,
    },
    rtcp::{
        self, receiver_report::ReceiverReport,
        transport_feedbacks::transport_layer_cc::TransportLayerCc,
    },
    rtp::extension::abs_send_time_extension::unix2ntp,
};

pub struct TwccStream {
    map: TwccSendInfo,
    bandwidth_estimator: Mutex<TwccBandwidthEstimator>,
    next_reader: Arc<dyn RTCPReader + Send + Sync>,
}

impl TwccStream {
    pub fn new(
        map: TwccSendInfo,
        estimate: TwccBandwidthEstimate,
        next_reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> TwccStream {
        TwccStream {
            map,
            bandwidth_estimator: Mutex::new(TwccBandwidthEstimator::new(estimate)),
            next_reader,
        }
    }
}

#[async_trait]
impl RTCPReader for TwccStream {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> Result<(usize, Attributes), Error> {
        let now = Instant::now();

        let (n, attr) = self.next_reader.read(buf, attributes).await?;

        let mut b = &buf[..n];
        let packets = rtcp::packet::unmarshal(&mut b)?;
        for packet in packets {
            let packet = packet.as_any();
            if let Some(tcc) = packet.downcast_ref::<TransportLayerCc>() {
                if let Ok(mut bandwidth_estimator) = self.bandwidth_estimator.lock() {
                    bandwidth_estimator.process_feedback(tcc, &self.map);
                }
            } else if let Some(rr) = packet.downcast_ref::<ReceiverReport>() {
                let now = (unix2ntp(SystemTime::now()) >> 16) as u32;

                // Get the last RTT
                let rtt_ms = rr
                    .reports
                    .iter()
                    .map(|recp| calculate_rtt_ms(now, recp.delay, recp.last_sender_report))
                    .reduce(|_, item| item);

                if let Some(rtt_ms) = rtt_ms {
                    if let Ok(mut bandwidth_estimator) = self.bandwidth_estimator.lock() {
                        bandwidth_estimator.update_rtt(rtt_ms);
                    }
                }
            }
        }

        if let Ok(mut bandwidth_estimator) = self.bandwidth_estimator.lock() {
            bandwidth_estimator.estimate(now);
        }

        Ok((n, attr))
    }
}

pub struct TwccInterceptor {
    map: TwccSendInfo,
    estimate: TwccBandwidthEstimate,
    start_time: Instant,
}

#[async_trait]
impl Interceptor for TwccInterceptor {
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync> {
        Arc::new(TwccStream::new(
            self.map.clone(),
            self.estimate.clone(),
            reader,
        ))
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
    estimate: TwccBandwidthEstimate,
}

impl TwccInterceptorBuilder {
    pub fn new() -> (TwccInterceptorBuilder, TwccBandwidthEstimate) {
        let estimate = TwccBandwidthEstimate::new();
        (
            TwccInterceptorBuilder {
                map: TwccSendInfo::new(),
                estimate: estimate.clone(),
            },
            estimate,
        )
    }
}

impl InterceptorBuilder for TwccInterceptorBuilder {
    fn build(&self, _id: &str) -> Result<Arc<dyn Interceptor + Send + Sync>, Error> {
        Ok(Arc::new(TwccInterceptor {
            map: self.map.clone(),
            estimate: self.estimate.clone(),
            start_time: Instant::now(),
        }))
    }
}

// TODO: This was copied from interceptor::stats::StatsInterceptor
fn calculate_rtt_ms(now: u32, delay: u32, last_sender_report: u32) -> f64 {
    let rtt = now - delay - last_sender_report;
    let rtt_seconds = rtt >> 16;
    let rtt_fraction = (rtt & (u16::MAX as u32)) as f64 / (u16::MAX as u32) as f64;
    rtt_seconds as f64 * 1000.0 + (rtt_fraction as f64) * 1000.0
}
