
pub enum DepacketizerError {
    NeedMoreInput,
    PayloadTooShort,
    OutputBufferFull,
    UnsupportedPayloadType,
    AggregationInterrupted,
    MissedAggregateStart,
}

pub trait Depacketizer<'a>
where
    Self: Sized,
{
    fn new_reader(output: &'a mut [u8]) -> Self;

    fn push_payload(&mut self, payload: &[u8]) -> Result<(), DepacketizerError>;

    /// Consume the `Depacketizer` and return the number of bytes written.
    fn finish(self) -> usize;
}
