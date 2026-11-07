use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use crate::config::Tls;

const MAX_PEM: u64 = 1024 * 1024;
const MAX_CHAIN: usize = 16;

fn read(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let shown = path.display();
    let meta = std::fs::metadata(path).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown}: not a file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_PEM + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{shown}: {e}"))?;
    if bytes.len() as u64 > MAX_PEM {
        return Err(format!("{shown}: larger than {MAX_PEM} bytes"));
    }
    Ok(bytes)
}

fn private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.permissions().mode();
        if mode & 0o004 != 0 {
            return Err(format!(
                "{} can be read by every user (mode {:o}): run `chmod o-r {}`",
                path.display(),
                mode & 0o777,
                path.display()
            ));
        }
    }
    let _ = path;
    Ok(())
}

pub struct Slot(std::sync::RwLock<TlsAcceptor>);

impl Slot {
    pub fn new(acceptor: TlsAcceptor) -> Self {
        Self(std::sync::RwLock::new(acceptor))
    }

    pub fn current(&self) -> TlsAcceptor {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn reload(&self, tls: &Tls) -> Result<(), String> {
        let fresh = acceptor(tls)?;
        *self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner) = fresh;
        Ok(())
    }
}

pub(crate) fn leaf(tls: &Tls) -> Result<CertificateDer<'static>, String> {
    let chain_pem = read(&tls.certificate)?;
    match CertificateDer::pem_slice_iter(&chain_pem).next() {
        Some(Ok(leaf)) => Ok(leaf.into_owned()),
        Some(Err(e)) => Err(format!("{}: {e}", tls.certificate.display())),
        None => Err(format!("{}: no certificate", tls.certificate.display())),
    }
}

pub fn acceptor(tls: &Tls) -> Result<TlsAcceptor, String> {
    let chain_pem = read(&tls.certificate)?;
    private(&tls.key)?;
    let key_pem = read(&tls.key)?;
    let chain = CertificateDer::pem_slice_iter(&chain_pem)
        .take(MAX_CHAIN + 1)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", tls.certificate.display()))?;
    if chain.is_empty() || chain.len() > MAX_CHAIN {
        return Err(format!("{}: 1 to {MAX_CHAIN} certificates", tls.certificate.display()));
    }
    let key = PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| format!("{}: {e}", tls.key.display()))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| format!("the certificate and the key: {e}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}
