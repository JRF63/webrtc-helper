use webrtc::rtp::header::Header;

pub trait RtpHeaderExt {
    fn advance_sequence_number(&mut self);
}

impl RtpHeaderExt for Header {
    fn advance_sequence_number(&mut self) {
        self.sequence_number = self.sequence_number.wrapping_add(1);
    }
}

fn next_ind(data: &[u8], start: usize) -> (usize, usize) {
    let mut zero_count = 0;

    for (i, &b) in data.iter().enumerate().skip(start) {
        if b == 0 {
            zero_count += 1;
            continue;
        } else if b == 1 && zero_count >= 2 {
            return (i - zero_count, i + 1);
        }
        zero_count = 0
    }
    (data.len(), data.len())
}

pub struct NaluChunks<'a> {
    data: &'a [u8],
    start: usize,
}

impl<'a> Iterator for NaluChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.data.len() {
            None
        } else {
            let (end, next_start) = next_ind(self.data, self.start);
            let slice = &self.data[self.start..end];
            self.start = next_start;
            Some(slice)
        }
    }
}

pub fn nalu_chunks(data: &[u8]) -> NaluChunks {
    let (_, start) = next_ind(data, 0);
    NaluChunks { data, start }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nalu_chunks_test() {
        let tests: Vec<(&[u8], Option<&[u8]>)> = vec![
            (&[], None),
            (&[0, 0, 0, 1], None),
            (&[0, 0, 0, 1, 0, 0, 1], Some(&[])),
            (&[0, 0, 0, 1, 2], Some(&[2])),
            (&[0, 0, 0, 0], None),
        ];
        for (data, res) in tests {
            assert_eq!(nalu_chunks(&data).next(), res);
        }
    }
}
