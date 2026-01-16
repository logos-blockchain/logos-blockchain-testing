use nomos_libp2p::{Multiaddr, Protocol};

pub fn extract_udp_port(addr: &Multiaddr) -> Option<u16> {
    addr.iter().find_map(|protocol| {
        if let Protocol::Udp(port) = protocol {
            Some(port)
        } else {
            None
        }
    })
}

pub fn find_matching_host(addr: &Multiaddr, original_ports: &[u16]) -> Option<usize> {
    extract_udp_port(addr).and_then(|port| {
        original_ports
            .iter()
            .position(|candidate| *candidate == port)
    })
}
