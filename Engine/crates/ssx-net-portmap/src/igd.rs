use std::net::{Ipv4Addr, SocketAddrV4};

use crate::ssdp::{Location, parse_url};
use crate::xml::{self, Element, escape};
use crate::{Error, Protocol};

const SERVICES: [&str; 3] = [
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service {
    pub service_type: String,
    pub control: Location,
}

pub fn service(description: &[u8], location: &Location, gateway: Ipv4Addr) -> Result<Service, Error> {
    let root = xml::parse(description)?;
    if root.name != "root" {
        return Err(Error::malformed("UPnP: the description's root is not <root>"));
    }
    let base = root.text_of("URLBase").filter(|b| !b.is_empty());
    let services: Vec<&Element> = root.walk().into_iter().filter(|e| e.name == "service").collect();
    for wanted in SERVICES {
        let Some(s) = services.iter().find(|s| s.text_of("serviceType") == Some(wanted)) else { continue };
        let url = s
            .text_of("controlURL")
            .filter(|u| !u.is_empty())
            .ok_or_else(|| Error::malformed("UPnP: a service without its controlURL"))?;
        let control = resolve(url, base, location)?;
        if *control.addr.ip() != gateway {
            return Err(Error::malformed(format!(
                "UPnP: the control URL is on {}, not on the gateway {gateway}",
                control.addr.ip()
            )));
        }
        return Ok(Service { service_type: wanted.to_string(), control });
    }
    Err(Error::malformed("UPnP: the gateway offers no WANIPConnection or WANPPPConnection service"))
}

fn resolve(url: &str, base: Option<&str>, location: &Location) -> Result<Location, Error> {
    if url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://")) {
        return parse_url(url);
    }
    let base = match base {
        Some(b) => parse_url(b)?,
        None => location.clone(),
    };
    let path = if url.starts_with('/') {
        url.to_string()
    } else {
        let dir = base.path.rsplit_once('/').map_or("", |(d, _)| d);
        format!("{dir}/{url}")
    };
    parse_url(&format!("http://{}{path}", base.addr))
}

fn proto(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "TCP",
        Protocol::Udp => "UDP",
    }
}

pub fn envelope(service_type: &str, action: &str, args: &[(&str, String)]) -> String {
    let mut body = String::new();
    for (k, v) in args {
        body.push_str(&format!("<{k}>{}</{k}>", escape(v)));
    }
    format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{body}</u:{action}></s:Body></s:Envelope>\r\n",
        escape(service_type)
    )
}

pub fn add_port_mapping(
    service_type: &str,
    protocol: Protocol,
    port: u16,
    client: Ipv4Addr,
    description: &str,
    lease: u32,
) -> String {
    envelope(
        service_type,
        "AddPortMapping",
        &[
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", proto(protocol).to_string()),
            ("NewInternalPort", port.to_string()),
            ("NewInternalClient", client.to_string()),
            ("NewEnabled", "1".to_string()),
            ("NewPortMappingDescription", description.to_string()),
            ("NewLeaseDuration", lease.to_string()),
        ],
    )
}

pub fn delete_port_mapping(service_type: &str, protocol: Protocol, port: u16) -> String {
    envelope(
        service_type,
        "DeletePortMapping",
        &[
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", proto(protocol).to_string()),
        ],
    )
}

pub fn get_external_address(service_type: &str) -> String {
    envelope(service_type, "GetExternalIPAddress", &[])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Done(Vec<(String, String)>),
    Fault { code: u32, description: String },
}

pub fn fault_text(code: u32) -> &'static str {
    match code {
        402 => "invalid arguments",
        501 => "the action failed",
        606 => "not authorized (port mapping may be off on the gateway)",
        714 => "no such mapping",
        715 => "a wildcard is not permitted for the remote host",
        716 => "a wildcard is not permitted for the external port",
        718 => "the port is mapped to another host already",
        724 => "the external and internal ports must be equal",
        725 => "only permanent leases are supported",
        726 => "the remote host must be a wildcard",
        727 => "the external port must be a wildcard",
        728 => "no port maps are available",
        729 => "a conflict with another mechanism",
        _ => "an error",
    }
}

pub fn answer(status: u16, body: &[u8], action: &str) -> Result<Answer, Error> {
    let root = xml::parse(body)?;
    if root.name != "Envelope" {
        return Err(Error::malformed(format!("SOAP: an answer ({status}) that is no envelope")));
    }
    let body = root.child("Body").ok_or_else(|| Error::malformed("SOAP: an envelope without its Body"))?;
    if let Some(fault) = body.child("Fault") {
        let error = fault.child("detail").and_then(|d| d.child("UPnPError"));
        let code = error
            .and_then(|e| e.text_of("errorCode"))
            .and_then(|c| (c.len() <= 4).then(|| c.parse::<u32>().ok()).flatten())
            .unwrap_or(0);
        let description = ssx_core::text::shown_max(
            error
                .and_then(|e| e.text_of("errorDescription"))
                .or_else(|| fault.text_of("faultstring"))
                .unwrap_or_default(),
            120,
        )
        .to_string();
        return Ok(Answer::Fault { code, description });
    }
    if status != 200 {
        return Err(Error::malformed(format!("SOAP: status {status} without a fault")));
    }
    let wanted = format!("{action}Response");
    let resp = body.child(&wanted).ok_or_else(|| Error::malformed(format!("SOAP: no <{wanted}>")))?;
    Ok(Answer::Done(resp.children.iter().map(|c| (c.name.clone(), c.text.trim().to_string())).collect()))
}

pub fn external_address(args: &[(String, String)]) -> Result<Ipv4Addr, Error> {
    args.iter()
        .find(|(k, _)| k == "NewExternalIPAddress")
        .and_then(|(_, v)| v.parse().ok())
        .ok_or_else(|| Error::malformed("SOAP: no NewExternalIPAddress that is an IPv4 address"))
}

pub fn soap_action(service_type: &str, action: &str) -> String {
    format!("\"{service_type}#{action}\"")
}

pub fn location(addr: SocketAddrV4, path: &str) -> Location {
    Location { addr, path: path.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const DESCRIPTION: &str = "<?xml version=\"1.0\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major></specVersion><device><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType><deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType><deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType><serviceList><service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service></serviceList></device></deviceList></device></deviceList></device></root>";

    #[test]
    fn the_service_and_its_control_url() {
        let gw = Ipv4Addr::new(192, 168, 1, 1);
        let loc = Location { addr: SocketAddrV4::new(gw, 5000), path: "/rootDesc.xml".into() };
        let s = service(DESCRIPTION.as_bytes(), &loc, gw).expect("reads");
        assert_eq!(s.control.path, "/ctl/IPConn");
        assert_eq!(s.control.addr.port(), 5000);
        let other = DESCRIPTION.replace("/ctl/IPConn", "http://10.9.9.9:80/ctl");
        assert!(service(other.as_bytes(), &loc, gw).unwrap_err().to_string().contains("not on the gateway"));
        let relative = DESCRIPTION.replace("/ctl/IPConn", "ctl");
        let loc2 = Location { addr: SocketAddrV4::new(gw, 5000), path: "/dev/rootDesc.xml".into() };
        assert_eq!(service(relative.as_bytes(), &loc2, gw).unwrap().control.path, "/dev/ctl");
        let none = DESCRIPTION.replace("WANIPConnection:1", "Layer3Forwarding:1");
        assert!(service(none.as_bytes(), &loc, gw).is_err());
    }

    #[test]
    fn soap_answers_and_faults() {
        let ok = "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:GetExternalIPAddressResponse xmlns:u=\"x\"><NewExternalIPAddress>203.0.113.5</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>";
        let Answer::Done(args) = answer(200, ok.as_bytes(), "GetExternalIPAddress").unwrap() else { panic!() };
        assert_eq!(external_address(&args).unwrap(), Ipv4Addr::new(203, 0, 113, 5));
        let fault = "<s:Envelope xmlns:s=\"x\"><s:Body><s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\"><errorCode>718</errorCode><errorDescription>ConflictInMappingEntry</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>";
        assert_eq!(
            answer(500, fault.as_bytes(), "AddPortMapping").unwrap(),
            Answer::Fault { code: 718, description: "ConflictInMappingEntry".into() }
        );
        assert!(answer(500, b"<s:Envelope><s:Body/></s:Envelope>", "AddPortMapping").is_err());
        let env = add_port_mapping(
            SERVICES[1],
            Protocol::Udp,
            27450,
            Ipv4Addr::new(192, 168, 1, 20),
            "Project <Avalanche>",
            7200,
        );
        assert!(env.contains("<NewProtocol>UDP</NewProtocol>") && env.contains("Project &lt;Avalanche&gt;"), "{env}");
        assert!(xml::parse(env.as_bytes()).is_ok());
    }
}
