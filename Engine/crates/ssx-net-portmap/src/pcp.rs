use std::net::{Ipv4Addr, Ipv6Addr};

use crate::wire::Cursor;
use crate::{Error, Protocol};

pub const VERSION: u8 = 2;
const OP_MAP: u8 = 1;
const RESPONSE_BIT: u8 = 0x80;
pub const MAX_MESSAGE: usize = 1100;
pub const MAP_SIZE: usize = 60;

fn proto_number(p: Protocol) -> u8 {
    match p {
        Protocol::Tcp => 6,
        Protocol::Udp => 17,
    }
}

pub fn map_request(
    client: Ipv4Addr,
    nonce: [u8; 12],
    protocol: Protocol,
    internal: u16,
    external: u16,
    lifetime: u32,
) -> [u8; MAP_SIZE] {
    let mut b = [0u8; MAP_SIZE];
    b[0] = VERSION;
    b[1] = OP_MAP;
    b[4..8].copy_from_slice(&lifetime.to_be_bytes());
    b[8..24].copy_from_slice(&client.to_ipv6_mapped().octets());
    b[24..36].copy_from_slice(&nonce);
    b[36] = proto_number(protocol);
    b[40..42].copy_from_slice(&internal.to_be_bytes());
    b[42..44].copy_from_slice(&external.to_be_bytes());
    b[44..60].copy_from_slice(&Ipv4Addr::UNSPECIFIED.to_ipv6_mapped().octets());
    b
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Mapped { lifetime: u32, epoch: u32, external_port: u16, external: Ipv6Addr },
    Refused { code: u8, lifetime: u32 },
    NatPmpOnly,
}

pub fn result_text(code: u8) -> &'static str {
    match code {
        1 => "UNSUPP_VERSION",
        2 => "NOT_AUTHORIZED (port mapping may be off on the gateway)",
        3 => "MALFORMED_REQUEST",
        4 => "UNSUPP_OPCODE",
        5 => "UNSUPP_OPTION",
        6 => "MALFORMED_OPTION",
        7 => "NETWORK_FAILURE",
        8 => "NO_RESOURCES",
        9 => "UNSUPP_PROTOCOL",
        10 => "USER_EX_QUOTA",
        11 => "CANNOT_PROVIDE_EXTERNAL (the port is taken outside)",
        12 => "ADDRESS_MISMATCH (a NAT between this host and the gateway?)",
        13 => "EXCESSIVE_REMOTE_PEERS",
        _ => "an unknown result code",
    }
}

pub fn map_answer(b: &[u8], nonce: [u8; 12], protocol: Protocol, internal: u16) -> Result<Answer, Error> {
    if b.len() >= 8 && b.first() == Some(&0) && b.get(2..4) == Some(&[0, 1]) {
        return Ok(Answer::NatPmpOnly);
    }
    if b.len() < 24 || b.len() > MAX_MESSAGE || !b.len().is_multiple_of(4) {
        return Err(Error::malformed(format!("PCP: an answer of {} bytes (24 to 1 100, a multiple of 4)", b.len())));
    }
    let mut c = Cursor::new(b, "PCP");
    let (version, op, _reserved, code) = (c.u8()?, c.u8()?, c.u8()?, c.u8()?);
    if version != VERSION {
        return Err(Error::malformed(format!("PCP: version {version} (2)")));
    }
    if op != RESPONSE_BIT | OP_MAP {
        return Err(Error::malformed(format!("PCP: opcode {op:#04x} (the MAP answer is 0x81)")));
    }
    let (lifetime, epoch) = (c.u32()?, c.u32()?);
    c.take(12)?;
    if code != 0 {
        return Ok(Answer::Refused { code, lifetime });
    }
    if c.left() < 36 {
        return Err(Error::malformed(format!("PCP: a MAP answer of {} bytes (60 or more)", b.len())));
    }
    let got_nonce = c.array::<12>()?;
    let proto = c.u8()?;
    c.take(3)?;
    let (inside, external_port) = (c.u16()?, c.u16()?);
    let external = Ipv6Addr::from(c.array::<16>()?);
    if got_nonce != nonce {
        return Err(Error::malformed("PCP: the answer's nonce is not this request's"));
    }
    if proto != proto_number(protocol) || inside != internal {
        return Err(Error::malformed(format!("PCP: an answer for protocol {proto} port {inside}, not this request's")));
    }
    while c.left() > 0 {
        let (_code, _reserved, len) = (c.u8()?, c.u8()?, c.u16()?);
        let padded = usize::from(len).next_multiple_of(4);
        if padded > c.left() {
            return Err(Error::malformed("PCP: an option longer than the answer"));
        }
        c.take(padded)?;
    }
    Ok(Answer::Mapped { lifetime, epoch, external_port, external })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(nonce: [u8; 12], code: u8, proto: u8, port: u16, extra: &[u8]) -> Vec<u8> {
        let mut b = vec![2, 0x81, 0, code];
        b.extend_from_slice(&7200u32.to_be_bytes());
        b.extend_from_slice(&42u32.to_be_bytes());
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&nonce);
        b.extend_from_slice(&[proto, 0, 0, 0]);
        b.extend_from_slice(&port.to_be_bytes());
        b.extend_from_slice(&port.to_be_bytes());
        b.extend_from_slice(&Ipv4Addr::new(203, 0, 113, 5).to_ipv6_mapped().octets());
        b.extend_from_slice(extra);
        b
    }

    #[test]
    fn map_request_and_answers() {
        let n = [7u8; 12];
        let r = map_request(Ipv4Addr::new(192, 168, 1, 20), n, Protocol::Udp, 27450, 27450, 7200);
        assert_eq!(&r[..8], &[2, 1, 0, 0, 0, 0, 0x1C, 0x20]);
        assert_eq!(&r[8..24], &Ipv4Addr::new(192, 168, 1, 20).to_ipv6_mapped().octets());
        assert_eq!(r[36], 17);
        let ok = answer(n, 0, 17, 27450, &[]);
        assert!(matches!(
            map_answer(&ok, n, Protocol::Udp, 27450),
            Ok(Answer::Mapped { lifetime: 7200, epoch: 42, external_port: 27450, .. })
        ));
        assert!(map_answer(&ok, [8; 12], Protocol::Udp, 27450).unwrap_err().to_string().contains("nonce"));
        assert!(map_answer(&ok, n, Protocol::Tcp, 27450).is_err());
        assert!(map_answer(&ok, n, Protocol::Udp, 1).is_err());
        let option = answer(n, 0, 17, 27450, &[0x80, 0, 0, 3, 1, 2, 3, 0]);
        assert!(matches!(map_answer(&option, n, Protocol::Udp, 27450), Ok(Answer::Mapped { .. })));
        let long_option = answer(n, 0, 17, 27450, &[0x80, 0, 0, 9, 1, 2, 3, 0]);
        assert!(map_answer(&long_option, n, Protocol::Udp, 27450).unwrap_err().to_string().contains("option"));
        assert_eq!(
            map_answer(&answer(n, 2, 17, 27450, &[])[..24], n, Protocol::Udp, 27450),
            Ok(Answer::Refused { code: 2, lifetime: 7200 })
        );
        assert_eq!(map_answer(&[0, 129, 0, 1, 0, 0, 0, 0], n, Protocol::Udp, 27450), Ok(Answer::NatPmpOnly));
        assert!(map_answer(&[0; 1104], n, Protocol::Udp, 27450).is_err());
        assert!(map_answer(&ok[..58], n, Protocol::Udp, 27450).is_err());
    }
}
