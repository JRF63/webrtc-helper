use bytes::Buf;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::time::timeout;
use webrtc::{rtp, util::Unmarshal};

const MAX_MTU: usize = 1500;
const NUM_PACKETS_TO_BUFFER: u16 = 128;
const READ_TIMEOUT: Duration = Duration::from_millis(5000);

#[cfg(not(test))]
type TrackRemote = webrtc::track::track_remote::TrackRemote;

#[cfg(test)]
type TrackRemote = dyn tests::DummyTrackRemoteTrait;

type PacketBuffer = Box<[u8; MAX_MTU]>;

#[derive(Debug)]
pub enum ReorderBufferError {
    NoMoreSavedPackets,
    HeaderParsingError,
    PayloadReaderError,
    PayloadTooShort,
    UnableToMaintainReorderBuffer,
}

pub struct ReorderBuffer {
    track: Arc<TrackRemote>,
    expected_seq_num: Option<u16>,
    delta_offset: u16,
    packets: BTreeMap<u16, RawPacket>,
    buffers: Vec<PacketBuffer>,
    tmp: Vec<(u16, RawPacket)>,
}

impl ReorderBuffer {
    pub fn new(track: Arc<TrackRemote>) -> ReorderBuffer {
        let buffers = (0..NUM_PACKETS_TO_BUFFER)
            .map(|_| TryFrom::try_from(vec![0; MAX_MTU].into_boxed_slice()).unwrap())
            .collect();

        ReorderBuffer {
            track,
            expected_seq_num: None,
            delta_offset: 0,
            packets: BTreeMap::new(),
            buffers,
            tmp: Vec::with_capacity(NUM_PACKETS_TO_BUFFER as usize),
        }
    }

    fn reclaim_buffers(&mut self) {
        while let Some((_, packet)) = self.packets.pop_first() {
            let RawPacket { buffer, .. } = packet;
            self.buffers.push(buffer);
        }
    }

    fn process_saved_packets<T>(&mut self, output: &mut [u8]) -> Result<usize, ReorderBufferError>
    where
        T: PayloadReader,
    {
        if self.delta_offset > u16::MAX / 2 {
            // Recreate `BTreeMap`
            while let Some((delta, packet)) = self.packets.pop_first() {
                self.tmp.push((delta - self.delta_offset, packet));
            }
            while let Some((delta, packet)) = self.tmp.pop() {
                self.packets.insert(delta, packet);
            }
        }

        let mut reader = T::new_reader(output);

        while !self.packets.is_empty() {
            let first_seq_num = {
                let entry = self.packets.first_entry().unwrap(); // Safe unwrap since non-empty
                *entry.key()
            };

            if first_seq_num - self.delta_offset != 0 {
                break;
            }

            let (_, packet) = self.packets.pop_first().unwrap(); // Safe unwrap
            let RawPacket { buffer, len } = packet;

            // Reuse the buffer, adding it to the last spot
            self.buffers.push(buffer);

            let buffer = self.buffers.last().unwrap(); // Won't panic, we just pushed one

            self.delta_offset = self.delta_offset.wrapping_add(1);
            // Advance the expected sequence number regardless of errors in the next steps
            if let Some(sn) = &mut self.expected_seq_num {
                *sn = sn.wrapping_add(1);
            }

            let mut b: &[u8] = &buffer[..len];

            // Unmarshaling the header would move `b` to point to the payload
            if unmarshal_header(&mut b).is_none() {
                return Err(ReorderBufferError::HeaderParsingError);
            };

            match reader.read_payload(b) {
                Ok(PayloadReaderOutput::BytesWritten(n)) => {
                    return Ok(n); // TODO: return the n bytes written
                }
                Ok(PayloadReaderOutput::NeedMoreInput(r)) => reader = r,
                Err(_) => {
                    return Err(ReorderBufferError::PayloadReaderError);
                }
            }
        }
        Err(ReorderBufferError::NoMoreSavedPackets)
    }

    pub async fn dummy_read<T>(&mut self, output: &mut [u8]) -> Result<usize, ReorderBufferError>
    where
        T: PayloadReader,
    {
        debug_assert!(!self.buffers.is_empty());
        let mut buffer = self.buffers.pop().unwrap(); // Should not panic
        let mut reader = T::new_reader(output);

        loop {
            match timeout(READ_TIMEOUT, self.track.read(&mut *buffer)).await {
                Err(_) => {
                    // Retry in case of timeout
                    log::error!("Timed-out while reading from `TrackRemote`");
                    std::mem::drop(reader);
                    reader = T::new_reader(output);
                    continue;
                }
                Ok(read_result) => match read_result {
                    Err(_) => {
                        // Also retry if there is a read error
                        log::error!("Read error while reading from `TrackRemote`");
                        std::mem::drop(reader);
                        reader = T::new_reader(output);
                        continue; // TODO: Signal to caller to possibly call PLI
                    }
                    Ok((len, _)) => {
                        if len < 4 {
                            self.buffers.push(buffer);
                            return Err(ReorderBufferError::PayloadTooShort);
                        }

                        // Don't need to parse the whole header yet
                        let sequence_number = {
                            let mut tmp: &[u8] = &buffer[..];
                            tmp.get_u32() as u16
                        };

                        if self.expected_seq_num.is_none() {
                            self.expected_seq_num = Some(sequence_number);
                        }
                        let delta = sequence_number.wrapping_sub(self.expected_seq_num.unwrap());

                        if delta == 0 {
                            if !self.packets.is_empty() {
                                if let Some(packet) = self.packets.insert(
                                    delta.wrapping_add(self.delta_offset),
                                    RawPacket { buffer, len },
                                ) {
                                    self.buffers.push(packet.buffer);
                                }
                                match self.process_saved_packets::<T>(output) {
                                    Err(ReorderBufferError::NoMoreSavedPackets) => {
                                        buffer = self.buffers.pop().unwrap(); // TODO: Possibly empty?
                                        continue;
                                    }
                                    res => return res,
                                }
                            } else {
                                self.delta_offset = 0;

                                // Advance the expected sequence number regardless of errors in the next steps
                                if let Some(sn) = &mut self.expected_seq_num {
                                    *sn = sn.wrapping_add(1);
                                }

                                let mut b: &[u8] = &buffer[..len];

                                // Unmarshaling the header would move `b` to point to the payload
                                if unmarshal_header(&mut b).is_none() {
                                    self.buffers.push(buffer);
                                    return Err(ReorderBufferError::HeaderParsingError);
                                };

                                match reader.read_payload(b) {
                                    Ok(PayloadReaderOutput::BytesWritten(n)) => {
                                        self.buffers.push(buffer);
                                        return Ok(n);
                                    }
                                    Ok(PayloadReaderOutput::NeedMoreInput(r)) => {
                                        std::mem::drop(reader);
                                        reader = r;
                                        continue;
                                    }
                                    Err(_) => {
                                        self.buffers.push(buffer);
                                        return Err(ReorderBufferError::PayloadReaderError);
                                    }
                                }
                            }
                        } else {
                            // Wrap-around tolerant comparison
                            if delta > u16::MAX / 2 {
                                // Current seq num comes *before* the expected seq num
                                continue; // TODO: Ignore?
                            } else {
                                // Current seq num comes *after* the expected seq num

                                if !self.buffers.is_empty() && delta < NUM_PACKETS_TO_BUFFER {
                                    if let Some(packet) = self.packets.insert(
                                        delta.wrapping_add(self.delta_offset),
                                        RawPacket { buffer, len },
                                    ) {
                                        self.buffers.push(packet.buffer);
                                    }
                                    buffer = self.buffers.pop().unwrap(); // Not empty, should not panic

                                    continue;
                                } else {
                                    // No more empty buffers or `delta` is too large
                                    self.reclaim_buffers();
                                    self.expected_seq_num = None;
                                    self.delta_offset = 0;
                                    self.packets.insert(0, RawPacket { buffer, len });

                                    return Err(ReorderBufferError::UnableToMaintainReorderBuffer);
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

pub struct RawPacket {
    buffer: PacketBuffer,
    len: usize,
}

pub enum PayloadReaderOutput<T> {
    BytesWritten(usize),
    NeedMoreInput(T),
}

pub trait PayloadReader
where
    Self: Sized,
{
    type Reader<'a>: PayloadReader;
    type Error;

    fn new_reader<'a>(output: &'a mut [u8]) -> Self::Reader<'a>;

    fn read_payload(self, payload: &[u8]) -> Result<PayloadReaderOutput<Self>, Self::Error>;
}

fn unmarshal_header(buffer: &mut &[u8]) -> Option<rtp::header::Header> {
    let header = rtp::header::Header::unmarshal(buffer).ok()?;
    if header.padding {
        let payload_len = buffer.remaining();
        if payload_len > 0 {
            let padding_len = buffer[payload_len - 1] as usize;
            if padding_len <= payload_len {
                *buffer = &buffer[..payload_len - padding_len];
                Some(header)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        Some(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, Bytes, BytesMut};
    use std::{
        collections::{HashMap, VecDeque},
        sync::Mutex,
    };
    use webrtc::{
        rtp::{header::Header, packet::Packet},
        util::Marshal,
    };

    #[async_trait::async_trait]
    pub trait DummyTrackRemoteTrait {
        async fn read(
            &self,
            b: &mut [u8],
        ) -> Result<(usize, std::collections::HashMap<usize, usize>), webrtc::Error>;
    }

    struct DummyTrackRemote {
        packets: Mutex<VecDeque<Bytes>>,
    }

    impl DummyTrackRemote {
        fn new(packets: VecDeque<Bytes>) -> DummyTrackRemote {
            DummyTrackRemote {
                packets: Mutex::new(packets),
            }
        }
    }

    #[async_trait::async_trait]
    impl DummyTrackRemoteTrait for DummyTrackRemote {
        async fn read(
            &self,
            b: &mut [u8],
        ) -> Result<(usize, HashMap<usize, usize>), webrtc::Error> {
            let mut lock = self.packets.lock().unwrap();
            if let Some(packet) = lock.pop_front() {
                let min_len = usize::min(packet.len(), b.len());
                b[..min_len].copy_from_slice(&packet[..min_len]);
                Ok((min_len, HashMap::new()))
            } else {
                Err(webrtc::Error::ErrUnknownType)
            }
        }
    }

    // struct DummyPayloadReader<'a> {
    //     output: &'a mut [u8],
    // }

    // impl<'a> PayloadReader<'a> for DummyPayloadReader<'a> {
    //     type Error = ();

    //     fn new_reader(output: &'a mut [u8]) -> Self {
    //         Self { output }
    //     }

    //     fn read_payload(self, payload: &[u8]) -> Result<PayloadReaderOutput<Self>, Self::Error> {
    //         let min_len = usize::min(self.output.len(), payload.len());
    //         self.output[..min_len].copy_from_slice(&payload[..min_len]);
    //         Ok(PayloadReaderOutput::BytesWritten(min_len))
    //     }
    // }

    // #[tokio::test]
    // async fn reorder_buffer_test() {
    //     const START: u16 = 0;
    //     const N: u16 = 256;
    //     let mut packets = VecDeque::with_capacity(N as usize);
    //     for offset in 0..N {
    //         let i = START.wrapping_add(offset);
    //         let mut payload = BytesMut::new();
    //         payload.put_u16(i);
    //         let packet = Packet {
    //             header: Header {
    //                 sequence_number: i,
    //                 ..Default::default()
    //             },
    //             payload: payload.freeze(),
    //         };
    //         packets.push_back(packet.marshal().unwrap())
    //     }

    //     let track = DummyTrackRemote::new(packets);
    //     let mut reorder_buffer = ReorderBuffer::new(Arc::new(track));

    //     let mut output = vec![0u8; MAX_MTU];
    //     for offset in 0..N {
    //         let i = START.wrapping_add(offset);
    //         let n = reorder_buffer
    //             .dummy_read::<DummyPayloadReader>(&mut output)
    //             .await
    //             .unwrap();
    //     }
    // }
}
