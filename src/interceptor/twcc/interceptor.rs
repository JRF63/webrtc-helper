use super::{
    estimator::TwccBandwidthEstimator,
    sender::TwccTimestampSenderStream,
    sync::{TwccBandwidthEstimate, TwccSendInfo, TwccTime},
};
use async_trait::async_trait;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use webrtc::{
    interceptor::{
        stream_info::StreamInfo, Attributes, Error, Interceptor, InterceptorBuilder, RTCPReader,
        RTCPWriter, RTPReader, RTPWriter,
    },
    rtcp::{
        self,
        transport_feedbacks::transport_layer_cc::{
            PacketStatusChunk, SymbolTypeTcc, TransportLayerCc,
        },
    },
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
        let mut received = 0;
        let mut lost = 0;

        let packets = rtcp::packet::unmarshal(&mut &buf[..])?;
        for packet in packets {
            let packet = packet.as_any();
            if let Some(tcc) = packet.downcast_ref::<TransportLayerCc>() {
                let mut sequence_number = tcc.base_sequence_number;
                let mut arrival_time = TwccTime::extract_from_rtcp(tcc);

                let mut recv_deltas_iter = tcc.recv_deltas.iter();

                let mut with_packet_status = |status: &SymbolTypeTcc| {
                    match status {
                        SymbolTypeTcc::PacketNotReceived => {
                            lost += 1;
                        }
                        SymbolTypeTcc::PacketReceivedWithoutDelta => {
                            received += 1;
                        }
                        _ => {
                            received += 1;
                            if let Some(recv_delta) = recv_deltas_iter.next() {
                                arrival_time = TwccTime::from_recv_delta(arrival_time, recv_delta);

                                let (departure_time, packet_size) = self.map.load(sequence_number);

                                if let Ok(mut bandwidth_estimator) = self.bandwidth_estimator.lock()
                                {
                                    bandwidth_estimator.process_packet_feedback(
                                        departure_time,
                                        arrival_time,
                                        packet_size,
                                        now,
                                    );
                                }
                            }
                        }
                    }
                    sequence_number = sequence_number.wrapping_add(1);
                };

                for chunk in tcc.packet_chunks.iter() {
                    match chunk {
                        PacketStatusChunk::RunLengthChunk(chunk) => {
                            for _ in 0..chunk.run_length {
                                with_packet_status(&chunk.packet_status_symbol);
                            }
                        }
                        PacketStatusChunk::StatusVectorChunk(chunk) => {
                            for status in chunk.symbol_list.iter() {
                                with_packet_status(status);
                            }
                        }
                    }
                }
            }
        }

        if let Ok(mut bandwidth_estimator) = self.bandwidth_estimator.lock() {
            bandwidth_estimator.estimate(received, lost, now);
        }

        self.next_reader.read(buf, attributes).await
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
