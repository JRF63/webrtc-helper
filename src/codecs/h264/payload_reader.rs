//! Modifed from the `Depacketizer` impl of [webrtc::rtp::codecs::h264::H264Packet].

use super::constants::*;
use bytes::BufMut;

/// `H264PayloadReader` reads payloads from RTP packets and produces NAL units.
pub struct H264PayloadReader<'a> {
    buffer: &'a mut [u8],
    is_aggregating: bool,
    init_addr: usize,
}

/// Errors that `H264PayloadReader::read` can return.
///
/// Essentially a subset of [webrtc::rtp::Error]. The addition of `NeedMoreInput` is used to signal
/// incomplete parsing of FU-A fragmented packets and must be handled by feeding more data to
/// depacketize.
pub enum H264PayloadReaderError<'a> {
    ErrShortPacket,
    StapASizeLargerThanBuffer,
    NaluTypeIsNotHandled,
    AggregationInterrupted,
    MissedAggregateStart,
    NeedMoreInput(H264PayloadReader<'a>),
}

impl<'a> H264PayloadReader<'a> {
    /// Create a new `H264PayloadReader`.
    pub fn new(buffer: &'a mut [u8]) -> H264PayloadReader<'a> {
        let buf_start = buffer.as_mut_ptr();
        H264PayloadReader {
            // The original mut slice should still be untouched by the put* operations
            buffer,
            is_aggregating: false,
            init_addr: buf_start as usize,
        }
    }

    #[inline(always)]
    fn num_bytes_written(&self) -> usize {
        self.buffer.as_ptr() as usize - self.init_addr
    }

    #[cold]
    fn single_nalu(mut self, payload: &[u8]) -> Result<usize, H264PayloadReaderError<'a>> {
        if self.is_aggregating {
            return Err(H264PayloadReaderError::AggregationInterrupted);
        }
        self.buffer.put(ANNEXB_NALUSTART_CODE);
        self.buffer.put(payload);
        Ok(self.num_bytes_written())
    }

    #[cold]
    fn stapa_nalu(mut self, payload: &[u8]) -> Result<usize, H264PayloadReaderError<'a>> {
        if self.is_aggregating {
            return Err(H264PayloadReaderError::AggregationInterrupted);
        }
        let mut curr_offset = STAPA_HEADER_SIZE;
        while curr_offset < payload.len() {
            let nalu_size =
                ((payload[curr_offset] as usize) << 8) | payload[curr_offset + 1] as usize;
            curr_offset += STAPA_NALU_LENGTH_SIZE;

            if payload.len() < curr_offset + nalu_size {
                return Err(H264PayloadReaderError::StapASizeLargerThanBuffer);
            }

            self.buffer.put(ANNEXB_NALUSTART_CODE);
            self.buffer
                .put(&payload[curr_offset..curr_offset + nalu_size]);
            curr_offset += nalu_size;
        }

        Ok(self.num_bytes_written())
    }

    #[cold]
    fn other_nalu(self, _payload: &[u8]) -> Result<usize, H264PayloadReaderError<'a>> {
        Err(H264PayloadReaderError::NaluTypeIsNotHandled)
    }

    /// Reads a payload into the buffer. This method returns the number of bytes written to the
    /// original buffer upon success, indicating one or more NALU has been successfully written to
    /// the buffer.
    pub fn read_payload(mut self, payload: &[u8]) -> Result<usize, H264PayloadReaderError<'a>> {
        if payload.len() <= 2 {
            return Err(H264PayloadReaderError::ErrShortPacket);
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

                        self.buffer.put(ANNEXB_NALUSTART_CODE);
                        self.buffer.put_u8(nalu_ref_idc | fragmented_nalu_type);
                    } else {
                        return Err(H264PayloadReaderError::MissedAggregateStart);
                    }
                }

                // Skip first 2 bytes then add to buffer
                self.buffer.put(&payload[FUA_HEADER_SIZE..]);

                if b1 & FU_END_BITMASK != 0 {
                    Ok(self.num_bytes_written())
                } else {
                    Err(H264PayloadReaderError::NeedMoreInput(self))
                }
            }
            _ => H264PayloadReader::other_nalu(self, payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use webrtc::rtp::{codecs::h264::H264Payloader, packetizer::Payloader};

    const TEST_NALU: &[u8] = include_bytes!("nalus/1.h264");

    #[test]
    fn fragment_then_unfragment() {
        let mut payloader = H264Payloader::default();
        let payloads = payloader
            .payload(1188, &Bytes::copy_from_slice(TEST_NALU))
            .unwrap();

        let mut output = vec![0u8; TEST_NALU.len()];
        let mut reader = H264PayloadReader::new(&mut output);
        let mut bytes_written = None;
        for payload in payloads {
            match reader.read_payload(&payload) {
                Ok(n) => {
                    bytes_written = Some(n);
                    break;
                }
                Err(H264PayloadReaderError::NeedMoreInput(r)) => reader = r,
                Err(_) => panic!("Error processing payloads"),
            }
        }

        let n = bytes_written.unwrap();
        assert_eq!(&output[..n], TEST_NALU);
    }
}
