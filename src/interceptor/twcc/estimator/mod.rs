mod delay_based;
mod loss_based;

use super::data::{TwccSendInfo, TwccTime};
use async_trait::async_trait;
use std::sync::Arc;
use webrtc::{
    interceptor::{Attributes, Error, RTCPReader},
    rtcp::{
        self,
        transport_feedbacks::transport_layer_cc::{
            PacketStatusChunk, SymbolTypeTcc, TransportLayerCc,
        },
    },
};

pub struct TwccEstimatorStream {
    map: TwccSendInfo,
    next_reader: Arc<dyn RTCPReader + Send + Sync>,
}

impl TwccEstimatorStream {
    pub fn new(
        map: TwccSendInfo,
        next_reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> TwccEstimatorStream {
        TwccEstimatorStream { map, next_reader }
    }
}

#[async_trait]
impl RTCPReader for TwccEstimatorStream {
    async fn read(
        &self,
        buf: &mut [u8],
        attributes: &Attributes,
    ) -> Result<(usize, Attributes), Error> {
        let packets = rtcp::packet::unmarshal(&mut &buf[..])?;
        for packet in packets {
            let packet = packet.as_any();
            if let Some(tcc) = packet.downcast_ref::<TransportLayerCc>() {
                let mut sequence_number = tcc.base_sequence_number;
                let mut arrival_time_us = tcc.reference_time as i64 * 64000;

                let mut recv_deltas_iter = tcc.recv_deltas.iter();

                let mut with_packet_status = |status: &SymbolTypeTcc| {
                    match status {
                        SymbolTypeTcc::PacketReceivedSmallDelta
                        | SymbolTypeTcc::PacketReceivedLargeDelta => {
                            if let Some(recv_delta) = recv_deltas_iter.next() {
                                match recv_delta.type_tcc_packet {
                                    SymbolTypeTcc::PacketReceivedSmallDelta => {
                                        arrival_time_us += recv_delta.delta;
                                    }
                                    SymbolTypeTcc::PacketReceivedLargeDelta => {
                                        arrival_time_us += recv_delta.delta - 8192000;
                                    }
                                    _ => (),
                                }

                                // let send_time_us = self.map[sequence_number].load();
                                // let packet_delta = arrival_time_us - send_time_us;
                            }
                        }
                        _ => (),
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

        self.next_reader.read(buf, attributes).await
    }
}
