use std::io::{Read, Write};
use crate::abridged;
use crate::error::TransportError;
use crate::transport::Transport;

pub struct StreamTransport<S: Read + Write> {
    stream: S,
}

impl<S: Read + Write> StreamTransport<S> {
    pub fn new(stream: S) -> Self {
        StreamTransport { stream }
    }
}

impl<S: Read + Write + 'static> Transport for StreamTransport<S> {
    fn send(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let frame = abridged::encode_frame(data);
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        abridged::decode_frame(&mut self.stream)
    }
}