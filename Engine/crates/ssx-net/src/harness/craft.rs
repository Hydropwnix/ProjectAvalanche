use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

pub fn varint(v: u64) -> Vec<u8> {
    match v {
        0..=63 => vec![v as u8],
        64..=16_383 => (v as u16 | 0x4000).to_be_bytes().to_vec(),
        16_384..=1_073_741_823 => (v as u32 | 0x8000_0000).to_be_bytes().to_vec(),
        _ => (v.min((1 << 62) - 1) | 0xC000_0000_0000_0000).to_be_bytes().to_vec(),
    }
}

pub fn header(packet_type: u8, packet_id: u16, tick: u32) -> Vec<u8> {
    let mut h = vec![packet_type];
    h.extend_from_slice(&packet_id.to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(&tick.to_be_bytes());
    h
}

pub fn data_packet(channel: u16, message: &[u8]) -> Vec<u8> {
    let mut p = header(0, 1, 0);
    p.extend(varint(u64::from(channel)));
    p.push(1);
    p.push(0);
    p.extend(varint(message.len() as u64));
    p.extend_from_slice(message);
    p
}

pub fn fragment_packet(
    packet_id: u16,
    channel: u16,
    message_id: u16,
    fragment_id: u64,
    num_fragments: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut p = header(1, packet_id, 0);
    p.extend(varint(u64::from(channel)));
    p.extend_from_slice(&message_id.to_be_bytes());
    p.extend(varint(fragment_id));
    p.extend(varint(num_fragments));
    if fragment_id == 0 {
        p.push(0);
    }
    p.extend(varint(payload.len() as u64));
    p.extend_from_slice(payload);
    p
}

pub struct Noise(pub u64);

impl Noise {
    pub fn draw(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.draw() >> 56) as u8).collect()
    }
}

pub struct Proxy {
    front: UdpSocket,
    back: UdpSocket,
    server: SocketAddr,
    client: Option<SocketAddr>,
    pub recorded: Vec<Vec<u8>>,
}

impl Proxy {
    pub fn new(server: SocketAddr) -> std::io::Result<Self> {
        let front = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        let back = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        front.set_nonblocking(true)?;
        back.set_nonblocking(true)?;
        Ok(Self { front, back, server, client: None, recorded: Vec::new() })
    }

    pub fn addr(&self) -> std::io::Result<SocketAddr> {
        self.front.local_addr()
    }

    pub fn pump(&mut self) -> std::io::Result<()> {
        let mut buf = vec![0u8; 65_536];
        loop {
            match self.front.recv_from(&mut buf) {
                Ok((n, from)) => {
                    self.client = Some(from);
                    self.recorded.push(buf[..n].to_vec());
                    self.back.send_to(&buf[..n], self.server)?;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        loop {
            match self.back.recv_from(&mut buf) {
                Ok((n, _)) => {
                    if let Some(client) = self.client {
                        self.front.send_to(&buf[..n], client)?;
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn pump_upstream(&mut self) -> std::io::Result<()> {
        let mut buf = vec![0u8; 65_536];
        loop {
            match self.front.recv_from(&mut buf) {
                Ok((n, from)) => {
                    self.client = Some(from);
                    self.recorded.push(buf[..n].to_vec());
                    self.back.send_to(&buf[..n], self.server)?;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
        loop {
            match self.back.recv_from(&mut buf) {
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn capture(&mut self) -> std::io::Result<()> {
        let mut buf = vec![0u8; 65_536];
        loop {
            match self.front.recv_from(&mut buf) {
                Ok((n, from)) => {
                    self.client = Some(from);
                    self.recorded.push(buf[..n].to_vec());
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn inject(&self, bytes: &[u8]) -> std::io::Result<()> {
        self.back.send_to(bytes, self.server).map(|_| ())
    }
}
