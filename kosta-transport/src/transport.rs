use std::any::Any;
use crate::error::TransportError;

pub trait Transport: Any {
    fn send(&mut self, data: &[u8]) -> Result<(), TransportError>;
    fn recv(&mut self) -> Result<Vec<u8>, TransportError>;
}

// Реализация для Box<T>, где T: Transport
impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, data: &[u8]) -> Result<(), TransportError> {
        (**self).send(data)
    }
    fn recv(&mut self) -> Result<Vec<u8>, TransportError> {
        (**self).recv()
    }
}