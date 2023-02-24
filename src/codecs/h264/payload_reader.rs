//! Modifed from the `Depacketizer` impl of [webrtc::rtp::codecs::h264::H264Packet].

use super::constants::*;

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

/// `H264PayloadReader` reads payloads from RTP packets and produces NAL units.
pub struct H264PayloadReader<'a> {
    buf_mut: UnsafeBufMut<'a>,
    is_aggregating: bool,
}

/// Errors that `H264PayloadReader::read` can return.
pub enum H264PayloadReaderError {
    PayloadTooShort,
    OutputBufferFull,
    NaluTypeIsNotHandled,
    AggregationInterrupted,
    MissedAggregateStart,
}

impl<'a> PayloadReader<'a> for H264PayloadReader<'a> {
    type Error = H264PayloadReaderError;

    #[inline]
    fn new_reader(output: &'a mut [u8]) -> H264PayloadReader<'a> {
        H264PayloadReader {
            buf_mut: UnsafeBufMut::new(output),
            is_aggregating: false,
        }
    }

    /// Reads a payload into the buffer. This method returns the number of bytes written to the
    /// original buffer upon success, indicating one or more NALU has been successfully written to
    /// the buffer.
    #[inline]
    fn push_payload(&mut self, payload: &[u8]) -> Result<PayloadReaderOutput, Self::Error> {
        if payload.len() <= FUA_HEADER_SIZE {
            return Err(H264PayloadReaderError::PayloadTooShort);
        }

        // NALU header
        //
        // +---------------+
        // |0|1|2|3|4|5|6|7|
        // +-+-+-+-+-+-+-+-+
        // |F|NRI|  Type   |
        // +---------------+
        let b0 = payload[0];

        // NALU Types
        // https://tools.ietf.org/html/rfc6184#section-5.4
        match b0 & NALU_TYPE_BITMASK {
            1..=23 => H264PayloadReader::single_nalu(self, payload),
            STAPA_NALU_TYPE => H264PayloadReader::stapa_nalu(self, payload),
            FUA_NALU_TYPE => {
                // FU header
                //
                // +---------------+
                // |0|1|2|3|4|5|6|7|
                // +-+-+-+-+-+-+-+-+
                // |S|E|R|  Type   |
                // +---------------+
                let b1 = payload[1];

                if !self.is_aggregating {
                    if b1 & FU_START_BITMASK != 0 {
                        self.is_aggregating = true;

                        let nalu_ref_idc = b0 & NALU_REF_IDC_BITMASK;
                        let fragmented_nalu_type = b1 & NALU_TYPE_BITMASK;

                        if self.buf_mut.remaining_mut() >= ANNEXB_NALUSTART_CODE.len() + 1 {
                            // SAFETY: Checked that the buffer has enough space
                            unsafe {
                                self.buf_mut.put_slice(ANNEXB_NALUSTART_CODE);
                                self.buf_mut.put_u8(nalu_ref_idc | fragmented_nalu_type);
                            }
                        } else {
                            return Err(H264PayloadReaderError::OutputBufferFull);
                        }
                    } else {
                        return Err(H264PayloadReaderError::MissedAggregateStart);
                    }
                }

                let partial_nalu = &payload[FUA_HEADER_SIZE..];
                if self.buf_mut.remaining_mut() >= partial_nalu.len() {
                    // SAFETY: Checked that the buffer has enough space
                    unsafe {
                        // Skip first 2 bytes then add to buffer
                        self.buf_mut.put_slice(partial_nalu);
                    }
                } else {
                    return Err(H264PayloadReaderError::OutputBufferFull);
                }

                if b1 & FU_END_BITMASK != 0 {
                    Ok(PayloadReaderOutput::BytesWritten(self.num_bytes_written()))
                } else {
                    Ok(PayloadReaderOutput::NeedMoreInput)
                }
            }
            _ => H264PayloadReader::other_nalu(self, payload),
        }
    }
}

impl<'a> H264PayloadReader<'a> {
    #[inline(always)]
    fn num_bytes_written(&self) -> usize {
        self.buf_mut.num_bytes_written()
    }

    #[cold]
    fn single_nalu(
        &mut self,
        payload: &[u8],
    ) -> Result<PayloadReaderOutput, H264PayloadReaderError> {
        if self.is_aggregating {
            return Err(H264PayloadReaderError::AggregationInterrupted);
        }
        if self.buf_mut.remaining_mut() >= ANNEXB_NALUSTART_CODE.len() + payload.len() {
            // SAFETY: Checked that the buffer has enough space
            unsafe {
                self.buf_mut.put_slice(ANNEXB_NALUSTART_CODE);
                self.buf_mut.put_slice(payload);
            }
            Ok(PayloadReaderOutput::BytesWritten(self.num_bytes_written()))
        } else {
            Err(H264PayloadReaderError::OutputBufferFull)
        }
    }

    #[cold]
    fn stapa_nalu(
        &mut self,
        payload: &[u8],
    ) -> Result<PayloadReaderOutput, H264PayloadReaderError> {
        if self.is_aggregating {
            return Err(H264PayloadReaderError::AggregationInterrupted);
        }
        let mut curr_offset = STAPA_HEADER_SIZE;

        while curr_offset < payload.len() {
            // Get 2 bytes of the NALU size
            let nalu_size_bytes = payload
                .get(curr_offset..curr_offset + 2)
                .ok_or(H264PayloadReaderError::PayloadTooShort)?;

            // NALU size is a 16-bit unsigned integer in network byte order.
            // The compiler should be able to deduce that `try_into().unwrap()` would not panic.
            let nalu_size = u16::from_be_bytes(nalu_size_bytes.try_into().unwrap()) as usize;

            curr_offset += STAPA_NALU_LENGTH_SIZE;

            let nalu = payload
                .get(curr_offset..curr_offset + nalu_size)
                .ok_or(H264PayloadReaderError::PayloadTooShort)?;

            if self.buf_mut.remaining_mut() >= ANNEXB_NALUSTART_CODE.len() + nalu.len() {
                // SAFETY: Checked that the buffer has enough space
                unsafe {
                    self.buf_mut.put_slice(ANNEXB_NALUSTART_CODE);
                    self.buf_mut.put_slice(nalu);
                }
            } else {
                return Err(H264PayloadReaderError::OutputBufferFull);
            }

            curr_offset += nalu_size;
        }

        Ok(PayloadReaderOutput::BytesWritten(self.num_bytes_written()))
    }

    #[cold]
    fn other_nalu(&self, _payload: &[u8]) -> Result<PayloadReaderOutput, H264PayloadReaderError> {
        Err(H264PayloadReaderError::NaluTypeIsNotHandled)
    }
}

struct UnsafeBufMut<'a> {
    buffer: &'a mut [u8],
    index: usize,
}

impl<'a> UnsafeBufMut<'a> {
    #[inline(always)]
    fn new(buffer: &'a mut [u8]) -> UnsafeBufMut<'a> {
        UnsafeBufMut { buffer, index: 0 }
    }

    // Same as `bytes::BufMut` but without length checks.
    #[inline(always)]
    unsafe fn put_slice(&mut self, src: &[u8]) {
        let num_bytes = src.len();
        std::ptr::copy_nonoverlapping(
            src.as_ptr(),
            self.buffer.get_unchecked_mut(self.index..).as_mut_ptr(),
            num_bytes,
        );
        self.index = self.index.wrapping_add(num_bytes);
    }

    // Same as `bytes::BufMut` but directly inserts to the slice without checks.
    #[inline(always)]
    unsafe fn put_u8(&mut self, n: u8) {
        *self.buffer.get_unchecked_mut(self.index) = n;
        self.index += 1;
    }

    #[inline(always)]
    fn remaining_mut(&self) -> usize {
        self.buffer.len() - self.index
    }

    #[inline(always)]
    fn num_bytes_written(&self) -> usize {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use webrtc::rtp::{codecs::h264::H264Payloader, packetizer::Payloader};

    const TEST_NALU: &[u8] = include_bytes!("nalus/1.h264");

    #[test]
    fn unsafe_buf_mut() {
        let mut vec = vec![0u8; 8];
        let mut b = UnsafeBufMut::new(&mut vec);
        let data = [42, 42];
        unsafe {
            assert_eq!(b.remaining_mut(), 8);
            b.put_u8(42);
            assert_eq!(b.remaining_mut(), 7);
            assert_eq!(b.index, 1);
            b.put_slice(&data);
            assert_eq!(b.remaining_mut(), 5);
            assert_eq!(b.index, 3);
            b.put_u8(42);
            assert_eq!(b.remaining_mut(), 4);
            assert_eq!(b.index, 4);
        }
    }

    #[test]
    fn fragment_then_unfragment() {
        let mut payloader = H264Payloader::default();
        let payloads = payloader
            .payload(1188, &Bytes::copy_from_slice(TEST_NALU))
            .unwrap();

        let mut output = vec![0u8; TEST_NALU.len()];
        let mut reader = H264PayloadReader::new_reader(&mut output);
        let mut bytes_written = None;
        for payload in payloads {
            match reader.push_payload(&payload) {
                Ok(PayloadReaderOutput::BytesWritten(n)) => {
                    bytes_written = Some(n);
                    break;
                }
                Ok(PayloadReaderOutput::NeedMoreInput) => continue,
                Err(_) => panic!("Error processing payloads"),
            }
        }

        let n = bytes_written.unwrap();
        assert_eq!(&output[..n], TEST_NALU);
    }
}
