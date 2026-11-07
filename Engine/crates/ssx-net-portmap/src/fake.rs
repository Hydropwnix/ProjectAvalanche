use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Seen = Arc<Mutex<Vec<(String, u16, u32)>>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PmpMode {
    NatPmp,
    Pcp,
    Garbage,
    OtherPort,
    Refuse,
    WrongNonce,
}

pub struct Pmp {
    pub addr: SocketAddrV4,
    pub seen: Seen,
    stop: Arc<AtomicBool>,
}

impl Drop for Pmp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn pmp(ip: Ipv4Addr, mode: PmpMode, lifetime: u32) -> Pmp {
    assert!(ip.is_loopback(), "a fake gateway listens on loopback only");
    let s = UdpSocket::bind((ip, 0)).unwrap();
    s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let addr = match s.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    let seen: Seen = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (seen2, stop2) = (Arc::clone(&seen), Arc::clone(&stop));
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while !stop2.load(Ordering::Relaxed) {
            let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
            let req = &buf[..n];
            let answers = answer(mode, req, lifetime, &seen2);
            for a in answers {
                let _ = s.send_to(&a, from);
            }
        }
    });
    Pmp { addr, seen, stop }
}

fn answer(mode: PmpMode, req: &[u8], lifetime: u32, seen: &Seen) -> Vec<Vec<u8>> {
    let epoch = 77u32.to_be_bytes();
    match (req.first(), mode) {
        (Some(0), PmpMode::NatPmp | PmpMode::Garbage | PmpMode::OtherPort | PmpMode::Refuse) if req.len() == 2 => {
            let mut a = vec![0, 128, 0, 0];
            a.extend_from_slice(&epoch);
            a.extend_from_slice(&[203, 0, 113, 5]);
            vec![a]
        }
        (Some(0), _) if req.len() == 12 => {
            let op = req[1];
            let internal = u16::from_be_bytes([req[4], req[5]]);
            let life = u32::from_be_bytes([req[8], req[9], req[10], req[11]]);
            seen.lock().unwrap().push((if op == 1 { "UDP" } else { "TCP" }.into(), internal, life));
            let mut a = vec![0, 128 + op, 0, 0];
            match mode {
                PmpMode::Refuse => {
                    a[3] = 2;
                    a.extend_from_slice(&epoch);
                    return vec![a];
                }
                PmpMode::Garbage => {
                    let mut wrong = a.clone();
                    wrong.extend_from_slice(&epoch);
                    wrong.extend_from_slice(&(internal ^ 1).to_be_bytes());
                    wrong.extend_from_slice(&internal.to_be_bytes());
                    wrong.extend_from_slice(&lifetime.to_be_bytes());
                    return vec![vec![0xFF; 40], wrong];
                }
                PmpMode::Pcp => return Vec::new(),
                _ => {}
            }
            a.extend_from_slice(&epoch);
            a.extend_from_slice(&internal.to_be_bytes());
            let ext = if mode == PmpMode::OtherPort && life != 0 { internal.wrapping_add(1000) } else { internal };
            a.extend_from_slice(&(if life == 0 { 0 } else { ext }).to_be_bytes());
            a.extend_from_slice(&(if life == 0 { 0 } else { lifetime }).to_be_bytes());
            vec![a]
        }
        (Some(2), PmpMode::NatPmp) => vec![vec![0, 129, 0, 1, 0, 0, 0, 77]],
        (Some(2), PmpMode::Pcp | PmpMode::WrongNonce) if req.len() == 60 => {
            let life = u32::from_be_bytes([req[4], req[5], req[6], req[7]]);
            let internal = u16::from_be_bytes([req[40], req[41]]);
            seen.lock().unwrap().push((if req[36] == 17 { "UDP" } else { "TCP" }.into(), internal, life));
            let mut a = vec![2, 0x81, 0, 0];
            a.extend_from_slice(&(if life == 0 { 0 } else { lifetime }).to_be_bytes());
            a.extend_from_slice(&epoch);
            a.extend_from_slice(&[0; 12]);
            let mut nonce = req[24..36].to_vec();
            if mode == PmpMode::WrongNonce {
                nonce[0] ^= 0xFF;
            }
            a.extend_from_slice(&nonce);
            a.extend_from_slice(&[req[36], 0, 0, 0]);
            a.extend_from_slice(&internal.to_be_bytes());
            a.extend_from_slice(&(if life == 0 { 0 } else { internal }).to_be_bytes());
            a.extend_from_slice(&Ipv4Addr::new(203, 0, 113, 5).to_ipv6_mapped().octets());
            vec![a]
        }
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IgdMode {
    Normal,
    PermanentOnly,
    Conflict,
    Huge,
    Never,
    Entities,
    Redirect,
    OtherHost,
    Slow,
}

pub struct Igd {
    pub ssdp: SocketAddrV4,
    pub seen: Seen,
    pub contacted: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Igd {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub const DESCRIPTION: &str = "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType><deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType><deviceList><device><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>CONTROL</controlURL></service></serviceList></device></deviceList></device></deviceList></device></root>";

pub fn igd(ip: Ipv4Addr, mode: IgdMode, spoof_from: Option<Ipv4Addr>) -> Igd {
    assert!(ip.is_loopback() && spoof_from.is_none_or(|s| s.is_loopback()), "a fake gateway listens on loopback only");
    let http = TcpListener::bind((ip, 0)).unwrap();
    let http_addr = match http.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    http.set_nonblocking(true).unwrap();
    let udp = UdpSocket::bind((ip, 0)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let ssdp = match udp.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    let seen: Seen = Arc::default();
    let contacted: Arc<Mutex<Vec<String>>> = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        let spoof = spoof_from.map(|s| UdpSocket::bind((s, 0)).unwrap());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !stop.load(Ordering::Relaxed) {
                let Ok((n, from)) = udp.recv_from(&mut buf) else { continue };
                if !buf[..n].starts_with(b"M-SEARCH * HTTP/1.1\r\n") {
                    continue;
                }
                if let Some(s) = &spoof {
                    let lie = "HTTP/1.1 200 OK\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: http://127.0.0.99:1/x\r\n\r\n";
                    let _ = s.send_to(lie.as_bytes(), from);
                }
                let ok = format!(
                    "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nUSN: uuid:fake\r\nEXT:\r\nLOCATION: http://{http_addr}/rootDesc.xml\r\n\r\n"
                );
                let _ = udp.send_to(ok.as_bytes(), from);
            }
        });
    }
    {
        let (stop, seen, contacted) = (Arc::clone(&stop), Arc::clone(&seen), Arc::clone(&contacted));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match http.accept() {
                    Ok((c, _)) => {
                        let (seen, contacted, stop) = (Arc::clone(&seen), Arc::clone(&contacted), Arc::clone(&stop));
                        std::thread::spawn(move || serve(c, mode, &seen, &contacted, &stop));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
    }
    Igd { ssdp, seen, contacted, stop }
}

fn arg(body: &str, name: &str) -> String {
    let open = format!("<{name}>");
    body.split_once(&open)
        .and_then(|(_, r)| r.split_once(&format!("</{name}>")))
        .map(|(v, _)| v.to_string())
        .unwrap_or_default()
}

fn serve(mut c: TcpStream, mode: IgdMode, seen: &Seen, contacted: &Mutex<Vec<String>>, stop: &AtomicBool) {
    c.set_nonblocking(false).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut req = Vec::new();
    let mut buf = [0u8; 4096];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
        let Ok(n) = c.read(&mut buf) else { return };
        if n == 0 {
            return;
        }
        req.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&req).into_owned();
    let head_end = text.find("\r\n\r\n").unwrap();
    let len: usize = text[..head_end]
        .lines()
        .find_map(|l| l.strip_prefix("Content-Length: ").map(|v| v.trim().parse().unwrap()))
        .unwrap_or(0);
    while req.len() < head_end + 4 + len {
        let Ok(n) = c.read(&mut buf) else { return };
        if n == 0 {
            break;
        }
        req.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&req).into_owned();
    let first = text.lines().next().unwrap_or_default().to_string();
    contacted.lock().unwrap().push(first.clone());
    let reply = |c: &mut TcpStream, status: &str, body: &str| {
        let _ = c.write_all(
            format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/xml\r\n\r\n{body}", body.len())
                .as_bytes(),
        );
    };
    if first.starts_with("GET /rootDesc.xml") {
        let control =
            if mode == IgdMode::OtherHost { "http://127.0.0.99:5000/ctl".to_string() } else { "/ctl".to_string() };
        let desc = DESCRIPTION.replace("CONTROL", &control);
        match mode {
            IgdMode::Huge => {
                let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\n\r\n<root>");
                let chunk = vec![b'x'; 64 * 1024];
                for _ in 0..160 {
                    if c.write_all(&chunk).is_err() {
                        return;
                    }
                }
            }
            IgdMode::Never => {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            IgdMode::Entities => {
                let laughs = "<?xml version=\"1.0\"?><!DOCTYPE root [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;\">]><root>&c;</root>";
                reply(&mut c, "200 OK", laughs);
            }
            IgdMode::Redirect => {
                let _ = c.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.99:80/rootDesc.xml\r\nContent-Length: 0\r\n\r\n",
                );
            }
            IgdMode::Slow => {
                let r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{desc}", desc.len());
                for b in r.bytes() {
                    if stop.load(Ordering::Relaxed) || c.write_all(&[b]).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
            _ => reply(&mut c, "200 OK", &desc),
        }
        return;
    }
    if first.starts_with("POST /ctl") {
        let body = &text[head_end + 4..];
        let action = text[..head_end]
            .lines()
            .find_map(|l| l.strip_prefix("SOAPAction: "))
            .and_then(|v| v.trim().trim_matches('"').rsplit_once('#').map(|(_, a)| a.to_string()))
            .unwrap_or_default();
        let ok = |name: &str, inner: &str| {
            format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:{name}Response xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">{inner}</u:{name}Response></s:Body></s:Envelope>"
            )
        };
        let fault = |code: u32, text: &str| {
            format!(
                "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>{code}</errorCode><errorDescription>{text}</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>"
            )
        };
        match action.as_str() {
            "AddPortMapping" => {
                let lease: u32 = arg(body, "NewLeaseDuration").parse().unwrap_or(u32::MAX);
                let port: u16 = arg(body, "NewExternalPort").parse().unwrap_or(0);
                let proto = arg(body, "NewProtocol");
                if mode == IgdMode::Conflict {
                    return reply(&mut c, "500 Internal Server Error", &fault(718, "ConflictInMappingEntry"));
                }
                if mode == IgdMode::PermanentOnly && lease != 0 {
                    return reply(&mut c, "500 Internal Server Error", &fault(725, "OnlyPermanentLeasesSupported"));
                }
                seen.lock().unwrap().push((format!("add {proto}"), port, lease));
                reply(&mut c, "200 OK", &ok("AddPortMapping", ""));
            }
            "DeletePortMapping" => {
                let port: u16 = arg(body, "NewExternalPort").parse().unwrap_or(0);
                seen.lock().unwrap().push((format!("delete {}", arg(body, "NewProtocol")), port, 0));
                reply(&mut c, "200 OK", &ok("DeletePortMapping", ""));
            }
            "GetExternalIPAddress" => reply(
                &mut c,
                "200 OK",
                &ok("GetExternalIPAddress", "<NewExternalIPAddress>203.0.113.5</NewExternalIPAddress>"),
            ),
            _ => reply(&mut c, "500 Internal Server Error", &fault(401, "Invalid Action")),
        }
    }
}
