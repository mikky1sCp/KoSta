// kosta-transport/src/lib.rs
pub mod error;
pub mod transport;
pub mod abridged;
pub mod tcp;
pub mod mock;

pub use transport::Transport;
pub use tcp::StreamTransport;
pub use mock::MockTransport;
pub use error::TransportError;

pub mod tls {
    use native_tls::TlsConnector;
    use std::net::TcpStream;
    use crate::StreamTransport;
    use crate::error::TransportError;

    pub fn connect_tls(host: &str, port: u16, domain: &str) -> Result<StreamTransport<native_tls::TlsStream<TcpStream>>, TransportError> {
        let stream = TcpStream::connect((host, port))?;
        let connector = TlsConnector::builder()
            .danger_accept_invalid_certs(true) // для dev
            .build()?;
        let tls_stream = connector.connect(domain, stream)?;
        Ok(StreamTransport::new(tls_stream))
    }

    pub fn accept_tls(stream: TcpStream, acceptor: &native_tls::TlsAcceptor) -> Result<StreamTransport<native_tls::TlsStream<TcpStream>>, TransportError> {
        let tls_stream = acceptor.accept(stream)?;
        Ok(StreamTransport::new(tls_stream))
    }
}