use native_tls::TlsConnector;
use std::net::TcpStream;
use crate::StreamTransport;
use crate::error::TransportError;

pub fn connect_tls(host: &str, port: u16, domain: &str, insecure: bool) -> Result<StreamTransport<native_tls::TlsStream<TcpStream>>, TransportError> {
    let stream = TcpStream::connect((host, port))?;
    let mut builder = TlsConnector::builder();
    if insecure {
        builder.danger_accept_invalid_certs(true);
    }
    let connector = builder.build()?;
    let tls_stream = connector.connect(domain, stream)?;
    Ok(StreamTransport::new(tls_stream))
}

pub fn accept_tls(stream: TcpStream, acceptor: &native_tls::TlsAcceptor) -> Result<StreamTransport<native_tls::TlsStream<TcpStream>>, TransportError> {
    let tls_stream = acceptor.accept(stream)?;
    Ok(StreamTransport::new(tls_stream))
}