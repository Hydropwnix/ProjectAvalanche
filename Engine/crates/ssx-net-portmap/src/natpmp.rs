use std::net::Ipv4Addr;

use crate::wire::Cursor;
use crate::{Error, Protocol};

pub const VERSION: u8 = 0;
const OP_ADDRESS: u8 = 0;
const RESPONSE: u8 = 128;

pub fn address_request() -> [u8; 2] {
    [VERSION, OP_ADDRESS]
}

fn op(protocol: Protocol) -> u8 {
    match protocol {
        Protocol::Udp => 1,
        Protocol::Tcp => 2,
    }
}

pub fn map_request(protocol: Protocol, internal: u16, external: u16, lifetime: u32) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[0] = VERSION;
    b[1] = op(protocol);
    b[4..6].copy_from_slice(&internal.to_be_bytes());
    b[6..8].copy_from_slice(&external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime.to_be_bytes());
    b
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Address { epoch: u32, external: Ipv4Addr },
    Mapped { epoch: u32, internal: u16, external: u16, lifetime: u32 },
    Refused { code: u16, epoch: u32 },
}

pub fn result_text(code: u16) -> &'static str {
    match code {
        1 => "unsupported version",
        2 => "not authorized or refused (port mapping may be off on the gateway)",
        3 => "network failure (the gateway has no external address yet)",
        4 => "out of resources (no more mappings)",
        5 => "unsupported opcode",
        _ => "an unknown result code",
    }
}

pub fn address_answer(b: &[u8]) -> Result<Answer, Error> {
    let mut c = head(b, RESPONSE + OP_ADDRESS)?;
    let (code, epoch) = (c.u16()?, c.u32()?);
    if code != 0 {
        return Ok(Answer::Refused { code, epoch });
    }
    if b.len() != 12 {
        return Err(Error::malformed(format!("NAT-PMP: an address answer of {} bytes (12)", b.len())));
    }
    let external = Ipv4Addr::from(c.array::<4>()?);
    Ok(Answer::Address { epoch, external })
}

pub fn map_answer(b: &[u8], protocol: Protocol, internal: u16) -> Result<Answer, Error> {
    let mut c = head(b, RESPONSE + op(protocol))?;
    let (code, epoch) = (c.u16()?, c.u32()?);
    if code != 0 {
        return Ok(Answer::Refused { code, epoch });
    }
    if b.len() != 16 {
        return Err(Error::malformed(format!("NAT-PMP: a mapping answer of {} bytes (16)", b.len())));
    }
    let (inside, external, lifetime) = (c.u16()?, c.u16()?, c.u32()?);
    if inside != internal {
        return Err(Error::malformed(format!("NAT-PMP: an answer for internal port {inside}, not {internal}")));
    }
    Ok(Answer::Mapped { epoch, internal, external, lifetime })
}

fn head(b: &[u8], expected_op: u8) -> Result<Cursor<'_>, Error> {
    if b.len() < 8 || b.len() > 16 {
        return Err(Error::malformed(format!("NAT-PMP: an answer of {} bytes", b.len())));
    }
    let mut c = Cursor::new(b, "NAT-PMP");
    let (version, opcode) = (c.u8()?, c.u8()?);
    if version != VERSION {
        return Err(Error::malformed(format!("NAT-PMP: version {version} (0)")));
    }
    if opcode != expected_op {
        return Err(Error::malformed(format!(
            "NAT-PMP: opcode {opcode} (the answer to this request is {expected_op})"
        )));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_and_answers() {
        assert_eq!(
            map_request(Protocol::Udp, 27450, 27450, 7200),
            [0, 1, 0, 0, 0x6B, 0x3A, 0x6B, 0x3A, 0, 0, 0x1C, 0x20]
        );
        assert_eq!(map_request(Protocol::Tcp, 1, 0, 0)[1], 2);
        let mut ok = vec![0, 129, 0, 0, 0, 0, 0, 9];
        ok.extend_from_slice(&27450u16.to_be_bytes());
        ok.extend_from_slice(&27451u16.to_be_bytes());
        ok.extend_from_slice(&3600u32.to_be_bytes());
        assert_eq!(
            map_answer(&ok, Protocol::Udp, 27450),
            Ok(Answer::Mapped { epoch: 9, internal: 27450, external: 27451, lifetime: 3600 })
        );
        assert!(map_answer(&ok, Protocol::Tcp, 27450).is_err(), "the TCP answer's opcode is 130");
        assert!(map_answer(&ok, Protocol::Udp, 1).is_err(), "another internal port");
        assert!(map_answer(&ok[..15], Protocol::Udp, 27450).is_err());
        assert!(map_answer(&[ok.clone(), vec![0]].concat(), Protocol::Udp, 27450).is_err());
        let refused = [0, 129, 0, 2, 0, 0, 0, 1];
        assert_eq!(map_answer(&refused, Protocol::Udp, 27450), Ok(Answer::Refused { code: 2, epoch: 1 }));
        assert!(address_answer(&[1, 128, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4]).is_err(), "version 1");
        assert_eq!(
            address_answer(&[0, 128, 0, 0, 0, 0, 0, 5, 203, 0, 113, 5]),
            Ok(Answer::Address { epoch: 5, external: Ipv4Addr::new(203, 0, 113, 5) })
        );
        assert!(address_answer(&[0; 7]).is_err());
    }
}
