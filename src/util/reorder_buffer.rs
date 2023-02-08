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

#[derive(Debug)]
pub enum ReorderBufferError {
    NoMoreSavedPackets,
    HeaderParsingError,
    TrackRemoteReadTimeout,
    TrackRemoteReadError,
    PayloadReaderError,
    PayloadTooShort,
    BufferFull,
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
            .map(|_| PacketBuffer::new())
            .collect();

        ReorderBuffer {
            track,
            expected_seq_num: None,
            packets: BTreeMap::new(),
            buffers,
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

    pub async fn read_from_track<'a, T>(
        &mut self,
        reader: &mut T,
    ) -> Result<usize, ReorderBufferError>
    where
        T: PayloadReader<'a>,
    {
        if !self.packets.is_empty() {
            match self.process_saved_packets::<T>(reader) {
                Err(ReorderBufferError::NoMoreSavedPackets) => (),
                res => {
                    return res;
                }
            }
        }

        loop {
            let last_buffer = match self.buffers.last_mut() {
                Some(b) => b,
                None => {
                    let first_seq_num = {
                        // Unwrap should not panic since `self.buffers.is_empty()` implies
                        // `!self.packets.is_empty()`
                        let entry = self.packets.first_entry().unwrap();
                        *entry.key()
                    };
                    self.expected_seq_num = Some(first_seq_num);
                    return Err(ReorderBufferError::BufferFull);
                }
            };

            let track_read = timeout(READ_TIMEOUT, self.track.read(last_buffer)).await;
            match track_read {
                Err(_) => {
                    return Err(ReorderBufferError::TrackRemoteReadTimeout);
                }
                Ok(read_result) => match read_result {
                    Err(_) => {
                        return Err(ReorderBufferError::TrackRemoteReadError);
                    }
                    Ok((len, _)) => {
                        if len < 4 {
                            return Err(ReorderBufferError::PayloadTooShort);
                        }

                        let sequence_number = self.buffers.last().unwrap().get_sequence_number();
                        if self.expected_seq_num.is_none() {
                            self.expected_seq_num = Some(sequence_number);
                        }

                        match sequence_number.cmp(&self.expected_seq_num.unwrap()) {
                            std::cmp::Ordering::Equal => {
                                if !self.packets.is_empty() {
                                    let packet = RawPacket {
                                        buffer: self.buffers.pop().unwrap(),
                                        len,
                                    };
                                    if let Some(packet) =
                                        self.packets.insert(sequence_number, packet)
                                    {
                                        self.buffers.push(packet.buffer);
                                    }
                                    match self.process_saved_packets::<T>(reader) {
                                        Err(ReorderBufferError::NoMoreSavedPackets) => {
                                            continue;
                                        }
                                        res => {
                                            return res;
                                        }
                                    }
                                } else {
                                    // Advance the expected sequence number regardless of errors in the next steps
                                    self.expected_seq_num =
                                        Some(self.expected_seq_num.unwrap().next());

                                    let mut b: &[u8] = &self.buffers.last().unwrap()[..len];

                                    // Unmarshaling the header would move `b` to point to the payload
                                    if unmarshal_header(&mut b).is_none() {
                                        return Err(ReorderBufferError::HeaderParsingError);
                                    };

                                    match reader.push_payload(b) {
                                        Ok(PayloadReaderOutput::BytesWritten(n)) => {
                                            return Ok(n);
                                        }
                                        Ok(PayloadReaderOutput::NeedMoreInput) => {
                                            continue;
                                        }
                                        Err(_) => {
                                            return Err(ReorderBufferError::PayloadReaderError);
                                        }
                                    }
                                }
                            }

                            std::cmp::Ordering::Greater => {
                                let packet = RawPacket {
                                    buffer: self.buffers.pop().unwrap(),
                                    len,
                                };
                                if let Some(packet) = self.packets.insert(sequence_number, packet) {
                                    self.buffers.push(packet.buffer);
                                }
                                continue;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

struct PacketBuffer(Box<[u8; MAX_MTU]>);

impl std::ops::Deref for PacketBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        <[u8; MAX_MTU]>::as_slice(&self.0)
    }
}

impl std::ops::DerefMut for PacketBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        <[u8; MAX_MTU]>::as_mut_slice(&mut self.0)
    }
}

impl PacketBuffer {
    fn new() -> PacketBuffer {
        PacketBuffer(TryFrom::try_from(vec![0; MAX_MTU].into_boxed_slice()).unwrap())
    }

    fn get_sequence_number(&self) -> SequenceNumber {
        let mut tmp: &[u8] = &self;
        SequenceNumber(tmp.get_u32() as u16)
    }
}

pub struct RawPacket {
    buffer: PacketBuffer,
    len: usize,
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

        fn push_payload(&mut self, payload: &[u8]) -> Result<PayloadReaderOutput, Self::Error> {
            let min_len = usize::min(self.output.len(), payload.len());
            self.output[..min_len].copy_from_slice(&payload[..min_len]);
            Ok(PayloadReaderOutput::BytesWritten(min_len))
        }
    }

    #[test]
    fn sequence_number_sort() {
        const START: u16 = 65500;
        const N: u16 = 10000;
        let mut seq_nums: Vec<_> = (0..N)
            .map(|offset| SequenceNumber(START.wrapping_add(offset)))
            .collect();
        seq_nums.reverse();
        seq_nums.sort();

        for (offset, seq_num) in (0..N).zip(&seq_nums) {
            let val = START.wrapping_add(offset);
            assert_eq!(val, seq_num.0);
        }
    }

    async fn reorder_buffer_test(mut seq_nums: Vec<SequenceNumber>) {
        let packets: VecDeque<_> = seq_nums
            .iter()
            .map(|seq_num| {
                let mut payload = BytesMut::new();
                payload.put_u16(seq_num.0);
                let packet = Packet {
                    header: Header {
                        sequence_number: seq_num.0,
                        ..Default::default()
                    },
                    payload: payload.freeze(),
                };
                packet.marshal().unwrap()
            })
            .collect();

        seq_nums.sort();

        let track = DummyTrackRemote::new(packets.clone());
        let mut reorder_buffer = ReorderBuffer::new(Arc::new(track));

        let buf_len = reorder_buffer.buffers.len();

        let mut output = vec![0u8; MAX_MTU];
        let mut reader = DummyPayloadReader::new_reader(&mut output);

        for seq_num in seq_nums {
            let n = reorder_buffer.read_from_track(&mut reader).await.unwrap();
            std::mem::drop(reader);
            let mut b = &output[..n];
            assert_eq!(seq_num.0, b.get_u16());
            reader = DummyPayloadReader::new_reader(&mut output);
        }

        assert_eq!(reorder_buffer.buffers.len(), buf_len);
    }

    #[tokio::test]
    async fn reorder_buffer_inorder_test() {
        const START: u16 = 65500;
        const N: u16 = 10000;
        let seq_nums: Vec<_> = (0..N)
            .map(|offset| SequenceNumber(START.wrapping_add(offset)))
            .collect();
        reorder_buffer_test(seq_nums).await;
    }

    #[tokio::test]
    async fn reorder_buffer_simple_out_of_order_test() {
        const START: u16 = 65500;
        const N: u16 = 10000;
        let mut seq_nums: Vec<_> = (0..N)
            .map(|offset| SequenceNumber(START.wrapping_add(offset)))
            .collect();

        // Scramble seq_nums, leaving index 0 alone
        for i in (2..seq_nums.len()).step_by(2) {
            seq_nums.swap(i, i - 1);
        }

        reorder_buffer_test(seq_nums).await;
    }

    #[tokio::test]
    async fn reorder_buffer_large_window() {
        const START: u16 = 65500;
        const N: u16 = 10000;
        let mut seq_nums: Vec<_> = (0..N)
            .map(|offset| SequenceNumber(START.wrapping_add(offset)))
            .collect();

        seq_nums.swap(1, NUM_PACKETS_TO_BUFFER as usize);

        reorder_buffer_test(seq_nums).await;
    }
}
