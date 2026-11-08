use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use avalanche_master::MasterOptions;
use avalanche_server::discovery::Extras;
use avalanche_server::portmap::PortmapTargets;
use avalanche_server::{ServerOptions, ServerThread};
use ssx_net_portmap::fake::{self, IgdMode};
use ssx_net_portmap::{Method, SsdpTarget, Targets};

use crate::common::*;

fn server(ip: IpAddr, gateway: Ipv4Addr, targets: Targets, master: &str) -> ServerThread {
    assert!(targets.loopback_only(gateway));
    let mut config = crate::common::defaults();
    config.bind = ip;
    config.port = 0;
    config.discovery = listed_on(master);
    config.port_mapping.enabled = true;
    config.port_mapping.methods = vec![Method::Upnp];
    config.port_mapping.gateway = Some(gateway);
    let options = ServerOptions {
        config,
        data_dir: temp_dir("portmap"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
        extend: Some(Box::new(move |app| {
            app.insert_resource(PortmapTargets(targets));
            app.insert_resource(Extras { interval: Some(Duration::from_secs(1)), ..Extras::default() });
        })),
    };
    avalanche_server::spawn(options).expect("the server starts")
}

#[test]
fn a_public_server_maps_its_ports_shows_them_beside_the_master_and_removes_them() {
    let mut mc = avalanche_master::config::Config::defaults();
    mc.bind = Ipv4Addr::LOCALHOST.into();
    mc.port = 0;
    mc.allow_private = true;
    let master = avalanche_master::spawn(MasterOptions {
        config: mc,
        data_dir: std::env::temp_dir(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    })
    .expect("a master");
    let gw = Ipv4Addr::new(127, 0, 46, 1);
    let g = fake::igd(gw, IgdMode::Normal, None);
    let targets = Targets { ssdp: SsdpTarget::Unicast(g.ssdp), pmp_port: 9 };
    let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 46, 2));
    let s = server(ip, gw, targets, &format!("http://{}", master.started.addr));
    let status = s.started.discovery.clone();
    let end = Instant::now() + Duration::from_secs(20);
    let lines = loop {
        let lines = status.lines();
        if lines.iter().any(|l| l.contains("mapped, and reachable from the master")) || Instant::now() > end {
            break lines;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(lines.iter().any(|l| l.contains("mapped, and reachable from the master")), "{lines:#?}");
    let port = s.started.addr.port();
    let added: Vec<(String, u16)> = g.seen.lock().unwrap().iter().map(|(a, p, _)| (a.clone(), *p)).collect();
    for want in [("add UDP", port), ("add TCP", port), ("add UDP", port + 1)] {
        assert!(added.iter().any(|(a, p)| a == want.0 && *p == want.1), "{want:?} in {added:?}");
    }
    s.stop();
    s.join().expect("stops");
    let deleted = g.seen.lock().unwrap().iter().filter(|(a, _, _)| a.starts_with("delete")).count();
    assert_eq!(deleted, 3, "the stop removes every mapping");
    master.stop();
    master.join().expect("stops");
}
