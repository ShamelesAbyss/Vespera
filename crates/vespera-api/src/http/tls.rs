use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use super::ServerError;

pub(super) fn acceptor(certificate: &Path, private_key: &Path) -> Result<TlsAcceptor, ServerError> {
    let cert_pem = read_pem(certificate, 128 * 1024)?;
    let chain = CertificateDer::pem_slice_iter(&cert_pem)
        .take(17)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ServerError::TlsIdentity)?;
    if !(1..=16).contains(&chain.len()) {
        return Err(ServerError::TlsIdentity);
    }
    let key_pem = read_pem(private_key, 16 * 1024)?;
    let key = PrivateKeyDer::from_pem_slice(&key_pem).map_err(|_| ServerError::TlsIdentity)?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .map_err(|_| ServerError::TlsIdentity)?
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .map_err(|_| ServerError::TlsIdentity)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn read_pem(path: &Path, limit: u64) -> Result<Vec<u8>, ServerError> {
    let file = File::open(path).map_err(ServerError::TlsIo)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(ServerError::TlsIo)?;
    if bytes.len() as u64 > limit {
        return Err(ServerError::TlsIo(io::ErrorKind::InvalidData.into()));
    }
    Ok(bytes)
}
