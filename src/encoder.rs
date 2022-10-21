use crate::util::{time::RtpTimestamp, data_rate::DataRate};

pub trait Encoder {
    fn next_frame(&mut self) -> (&[u8], RtpTimestamp);
    fn set_data_rate(&mut self, data_rate: DataRate);
}
