use super::{TwccDataMap, MAX_SEQUENCE_NUMBER_COUNT};
use async_trait::async_trait;
use std::sync::Arc;
use webrtc::{
    interceptor::{
        stream_info::StreamInfo, Attributes, Error, Interceptor, InterceptorBuilder, RTCPReader,
        RTCPWriter, RTPReader, RTPWriter,
    },
    rtcp::{
        self,
        transport_feedbacks::transport_layer_cc::{SymbolTypeTcc, TransportLayerCc},
    },
};

pub struct TwccRtcpHandlerStream {
    map: TwccDataMap,
}

#[async_trait]
impl RTCPReader for TwccRtcpHandlerStream {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> Result<(usize, Attributes), Error> {
        if let Ok(mut map) = self.map.lock() {
            let packets = rtcp::packet::unmarshal(&mut &buf[..])?;
            for packet in packets {
                let packet = packet.as_any();
                if let Some(tcc) = packet.downcast_ref::<TransportLayerCc>() {
                    let mut sequence_number = tcc.base_sequence_number;
                    let mut arrival_time = (tcc.reference_time * 64000) as i64;

                    for recv_delta in tcc.recv_deltas.iter() {
                        match recv_delta.type_tcc_packet {
                            SymbolTypeTcc::PacketReceivedSmallDelta => {
                                arrival_time += recv_delta.delta;
                                // arrival_times.push(arrival_time);
                            }
                            SymbolTypeTcc::PacketReceivedLargeDelta => {
                                arrival_time += recv_delta.delta - 8192000;
                                // arrival_times.push(arrival_time);
                            }
                            _ => (),
                        }
                    }
                }
            }

            if map.len() > MAX_SEQUENCE_NUMBER_COUNT {
                
            }
        }

        Ok((0, attributes.clone()))
    }
}

pub struct TwccRtcpHandler {
    map: TwccDataMap,
}

pub struct TwccRtcpHandlerBuilder {
    map: TwccDataMap,
}

#[async_trait]
impl Interceptor for TwccRtcpHandlerBuilder {
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
        _info: &StreamInfo,
        writer: Arc<dyn RTPWriter + Send + Sync>,
    ) -> Arc<dyn RTPWriter + Send + Sync> {
        writer
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

impl TwccRtcpHandlerBuilder {
    pub fn with_map(map: TwccDataMap) -> Self {
        Self { map }
    }
}

impl InterceptorBuilder for TwccRtcpHandlerBuilder {
    fn build(&self, _id: &str) -> Result<std::sync::Arc<dyn Interceptor + Send + Sync>, Error> {
        todo!()
    }
}
