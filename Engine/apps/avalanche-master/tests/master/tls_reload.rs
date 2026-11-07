use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, ServerName};

fn issue(dir: &Path) -> Vec<u8> {
    let mut ca = CertificateParams::new(Vec::<String>::new()).expect("params");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca, KeyPair::generate().expect("key")).expect("CA");
    let key = KeyPair::generate().expect("key");
    let leaf =
        CertificateParams::new(vec!["127.0.0.1".to_string()]).expect("params").signed_by(&key, &ca).expect("leaf");
    std::fs::write(dir.join("chain.pem"), leaf.pem()).expect("write");
    std::fs::write(dir.join("key.pem"), key.serialize_pem()).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("key.pem"), std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
    ca.der().to_vec()
}

fn ask(addr: SocketAddr, root: &[u8]) -> Result<String, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(root.to_vec())).map_err(|e| e.to_string())?;
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from("127.0.0.1").map_err(|e| e.to_string())?;
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| e.to_string())?;
    let mut socket = TcpStream::connect(addr).map_err(|e| e.to_string())?;
    socket.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    socket.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    let mut tls = rustls::Stream::new(&mut conn, &mut socket);
    tls.write_all(b"GET /v1/servers?protocol=9 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut head = [0u8; 8];
    tls.read_exact(&mut head).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&head).into_owned())
}

#[test]
fn a_renewed_certificate_is_taken_without_a_restart() {
    let dir = std::env::temp_dir().join(format!("avalanche-master-tls-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let first = issue(&dir);
    let mut config = avalanche_master::config::Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.tls = Some(avalanche_master::config::Tls { certificate: dir.join("chain.pem"), key: dir.join("key.pem") });
    let m = avalanche_master::spawn(avalanche_master::MasterOptions {
        config,
        data_dir: dir.clone(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    })
    .expect("an HTTPS master");
    let (addr, reload) = (m.started.addr, || avalanche_master::reload(&m.started.master, m.started.tls.as_deref()));
    assert_eq!(ask(addr, &first).as_deref(), Ok("HTTP/1.1"));
    let second = issue(&dir);
    assert_eq!(ask(addr, &first).as_deref(), Ok("HTTP/1.1"), "new files alone change nothing");
    assert!(ask(addr, &second).is_err(), "not taken before the reload");
    reload();
    assert_eq!(ask(addr, &second).as_deref(), Ok("HTTP/1.1"), "the renewed certificate");
    assert!(ask(addr, &first).is_err(), "the old one is gone");
    std::fs::write(dir.join("chain.pem"), b"not a certificate").expect("write");
    reload();
    assert_eq!(ask(addr, &second).as_deref(), Ok("HTTP/1.1"), "a pair that does not read keeps the one in use");
    m.stop();
    m.join().expect("stops");
    let _ = std::fs::remove_dir_all(&dir);
}
