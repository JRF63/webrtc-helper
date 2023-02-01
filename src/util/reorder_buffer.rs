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
    UninitializedSequenceNumber,
}

pub struct ReorderBuffer {
    track: Arc<TrackRemote>,
    expected_seq_num: Option<SequenceNumber>,
    packets: BTreeMap<SequenceNumber, RawPacket>,
    buffers: Vec<PacketBuffer>,
}

impl ReorderBuffer {
    pub fn new(track: Arc<TrackRemote>) -> ReorderBuffer {
        let buffers = (0..NUM_PACKETS_TO_BUFFER)
            .map(|_| TryFrom::try_from(vec![0; MAX_MTU].into_boxed_slice()).unwrap())
            .collect();

        ReorderBuffer {
            track,
            expected_seq_num: None,
            packets: BTreeMap::new(),
            buffers,
        }
    }

    fn reclaim_buffers(&mut self) {
        while let Some((_, packet)) = self.packets.pop_first() {
            self.buffers.push(packet.buffer);
        }
    }

    fn process_saved_packets<'a, T>(&mut self, reader: &mut T) -> Result<usize, ReorderBufferError>
    where
        T: PayloadReader<'a>,
    {
        while !self.packets.is_empty() {
            let first_seq_num = {
                let entry = self.packets.first_entry().unwrap(); // Safe unwrap since non-empty
                *entry.key()
            };

            if let Some(expected_seq_num) = &mut self.expected_seq_num {
                if first_seq_num != *expected_seq_num {
                    break;
                } else {
                    // Advance the expected sequence number regardless of errors in the next steps
                    *expected_seq_num = expected_seq_num.next();
                }
            } else {
                return Err(ReorderBufferError::UninitializedSequenceNumber);
            }

            let (_, packet) = self.packets.pop_first().unwrap(); // Safe unwrap
            let RawPacket { buffer, len } = packet;

            // Reuse the buffer, adding it to the last spot
            self.buffers.push(buffer);

            let last = self.buffers.last().unwrap(); // Won't panic, we just pushed one

            let mut b: &[u8] = &last[..len];

            // Unmarshaling the header would move `b` to point to the payload
            if unmarshal_header(&mut b).is_none() {
                return Err(ReorderBufferError::HeaderParsingError);
            };

            match reader.push_payload(b) {
                Ok(PayloadReaderOutput::BytesWritten(n)) => {
                    return Ok(n);
                }
                Ok(PayloadReaderOutput::NeedMoreInput) => continue,
                Err(_) => {
                    return Err(ReorderBufferError::PayloadReaderError);
                }
            }
        }
        Err(ReorderBufferError::NoMoreSavedPackets)
    }

    pub async fn dummy_read<'a, T>(&mut self, reader: &mut T) -> Result<usize, ReorderBufferError>
    where
        T: PayloadReader<'a>,
    {
        debug_assert!(!self.buffers.is_empty());
        let mut buffer = self.buffers.pop().unwrap(); // Should not panic

        loop {
            match timeout(READ_TIMEOUT, self.track.read(&mut *buffer)).await {
                Err(_) => {
                    // Retry in case of timeout
                    log::error!("Timed-out while reading from `TrackRemote`");
                    reader.reset();
                    continue;
                }
                Ok(read_result) => match read_result {
                    Err(_) => {
                        // Also retry if there is a read error
                        log::error!("Read error while reading from `TrackRemote`");
                        reader.reset();
                        continue; // TODO: Signal to caller to possibly call PLI
                    }
                    Ok((len, _)) => {
                        if len < 4 {
                            self.buffers.push(buffer);
                            return Err(ReorderBufferError::PayloadTooShort);
                        }

                        let packet = RawPacket { buffer, len };
                        let sequence_number = packet.get_sequence_number();

                        if self.expected_seq_num.is_none() {
                            self.expected_seq_num = Some(sequence_number);
                        }

                        match sequence_number.cmp(&self.expected_seq_num.unwrap()) {
                            std::cmp::Ordering::Equal => {
                                if !self.packets.is_empty() {
                                    if let Some(packet) =
                                        self.packets.insert(sequence_number, packet)
                                    {
                                        self.buffers.push(packet.buffer);
                                    }
                                    buffer = self.buffers.pop().unwrap();
                                    match self.process_saved_packets::<T>(reader) {
                                        Err(ReorderBufferError::NoMoreSavedPackets) => {
                                            continue;
                                        }
                                        res => {
                                            self.buffers.push(buffer);
                                            return res;
                                        }
                                    }
                                } else {
                                    // Advance the expected sequence number regardless of errors in the next steps
                                    self.expected_seq_num =
                                        Some(self.expected_seq_num.unwrap().next());

                                    // Reuse the buffer, adding it to the last spot
                                    self.buffers.push(packet.buffer);

                                    let last = self.buffers.last().unwrap(); // Won't panic, we just pushed one

                                    let mut b: &[u8] = &last[..len];

                                    // Unmarshaling the header would move `b` to point to the payload
                                    if unmarshal_header(&mut b).is_none() {
                                        return Err(ReorderBufferError::HeaderParsingError);
                                    };

                                    match reader.push_payload(b) {
                                        Ok(PayloadReaderOutput::BytesWritten(n)) => {
                                            return Ok(n);
                                        }
                                        Ok(PayloadReaderOutput::NeedMoreInput) => {
                                            buffer = self.buffers.pop().unwrap();
                                            continue;
                                        }
                                        Err(_) => {
                                            return Err(ReorderBufferError::PayloadReaderError);
                                        }
                                    }
                                }
                            }

                            std::cmp::Ordering::Greater => {
                                if !self.buffers.is_empty() {
                                    if let Some(packet) =
                                        self.packets.insert(sequence_number, packet)
                                    {
                                        self.buffers.push(packet.buffer);
                                    }
                                    buffer = self.buffers.pop().unwrap();
                                    continue;
                                } else {
                                    // No more empty buffers
                                    self.reclaim_buffers();
                                    self.expected_seq_num = Some(sequence_number);
                                    self.packets.insert(sequence_number, packet);

                                    return Err(ReorderBufferError::UnableToMaintainReorderBuffer);
                                }
                            }
                            std::cmp::Ordering::Less => {
                                return Err(ReorderBufferError::UnableToMaintainReorderBuffer)
                            }
                        }
                    }
                },
            }
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct SequenceNumber(u16);

impl SequenceNumber {
    #[inline]
    fn next(&self) -> SequenceNumber {
        SequenceNumber(self.0.wrapping_add(1))
    }
}

impl PartialOrd for SequenceNumber {
    /// Total ordering from RFC1982.
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        const THRESHOLD: u16 = 1 << 15;

        if self.0 == other.0 {
            Some(std::cmp::Ordering::Equal)
        } else {
            if self.0 < other.0 {
                if other.0.wrapping_sub(self.0) < THRESHOLD {
                    Some(std::cmp::Ordering::Less)
                } else {
                    Some(std::cmp::Ordering::Greater)
                }
            } else {
                if other.0.wrapping_sub(self.0) > THRESHOLD {
                    Some(std::cmp::Ordering::Greater)
                } else {
                    Some(std::cmp::Ordering::Less)
                }
            }
        }
    }
}

impl Ord for SequenceNumber {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // SAFETY: `partial_cmp` never returns `None`
        unsafe { self.partial_cmp(other).unwrap_unchecked() }
    }
}

pub struct RawPacket {
    buffer: PacketBuffer,
    len: usize,
}

impl RawPacket {
    fn get_sequence_number(&self) -> SequenceNumber {
        let mut tmp: &[u8] = &self.buffer[..];
        SequenceNumber(tmp.get_u32() as u16)
    }
}

pub enum PayloadReaderOutput {
    BytesWritten(usize),
    NeedMoreInput,
}

pub trait PayloadReader<'a>
where
    Self: Sized,
{
    type Error;

    fn new_reader(output: &'a mut [u8]) -> Self;

    fn reset(&mut self);

    fn push_payload(&mut self, payload: &[u8]) -> Result<PayloadReaderOutput, Self::Error>;
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
    use bytes::{Buf, BufMut, Bytes, BytesMut};
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

    struct DummyPayloadReader<'a> {
        output: &'a mut [u8],
    }

    impl<'a> PayloadReader<'a> for DummyPayloadReader<'a> {
        type Error = ();

        fn new_reader(output: &'a mut [u8]) -> Self {
            Self { output }
        }

        fn reset(&mut self) {}

        fn push_payload(&mut self, payload: &[u8]) -> Result<PayloadReaderOutput, Self::Error> {
            let min_len = usize::min(self.output.len(), payload.len());
            self.output[..min_len].copy_from_slice(&payload[..min_len]);
            Ok(PayloadReaderOutput::BytesWritten(min_len))
        }
    }

    #[tokio::test]
    async fn reorder_buffer_test() {
        const START: u16 = 0;
        const N: u16 = 256;
        let mut packets = VecDeque::with_capacity(N as usize);
        for offset in 0..N {
            let i = START.wrapping_add(offset);
            let mut payload = BytesMut::new();
            payload.put_u16(i);
            let packet = Packet {
                header: Header {
                    sequence_number: i,
                    ..Default::default()
                },
                payload: payload.freeze(),
            };
            packets.push_back(packet.marshal().unwrap())
        }

        let track = DummyTrackRemote::new(packets);
        let mut reorder_buffer = ReorderBuffer::new(Arc::new(track));

        let mut output = vec![0u8; MAX_MTU];
        let mut reader = DummyPayloadReader::new_reader(&mut output);
        for offset in 0..N {
            let i = START.wrapping_add(offset);
            let n = reorder_buffer.dummy_read(&mut reader).await.unwrap();
            std::mem::drop(reader);
            let mut b = &output[..n];
            assert_eq!(i, b.get_u16());
            reader = DummyPayloadReader::new_reader(&mut output);
        }
    }
}
