//! The local network: which interfaces to use (`P3-T6`), the multicast sockets (`P3-T2`), and a
//! plain-language guess at what's wrong when nothing can be found (`P3-T7`).

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use protocol::discovery::{MULTICAST_GROUP, MULTICAST_PORT};
use socket2::{Domain, Protocol, Socket, Type};

/// An IPv4 interface worth announcing on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

/// Name fragments of virtual adapters: container bridges, VPNs, VM host networks.
const VIRTUAL: &[&str] = &[
    "docker", "br-", "veth", "virbr", "vmnet", "vboxnet", "tun", "tap", "wg", "tailscale", "zt", "utun",
    "lxc", "lxd", "podman", "cni", "flannel", "kube", "vethernet", "virtualbox", "vmware", "hyper-v",
    "loopback", "npcap", "zerotier", "openvpn", "wireguard", "nordlynx", "proton",
];

fn private(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
}

/// The real LAN interfaces among `all`: private IPv4, not loopback or link-local, and not a
/// container bridge, VPN or VM network by name (spec §6.6). Wired and Wi-Fi both qualify.
pub fn pick_interfaces(all: &[Iface]) -> Vec<Iface> {
    all.iter()
        .filter(|i| private(i.ip) && !i.ip.is_loopback() && !i.ip.is_link_local())
        .filter(|i| {
            let n = i.name.to_ascii_lowercase();
            !VIRTUAL.iter().any(|v| n.contains(v)) && n != "lo"
        })
        .cloned()
        .collect()
}

/// Every IPv4 interface on this machine.
pub fn all_interfaces() -> Vec<Iface> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(Iface { name: i.name, ip: v4.ip, netmask: v4.netmask }),
            _ => None,
        })
        .collect()
}

/// The interfaces discovery uses. `GPU_SHARE_IFACE` (comma-separated names) overrides the
/// automatic choice, for networks the filter gets wrong.
pub fn lan_interfaces() -> Vec<Iface> {
    let all = all_interfaces();
    match std::env::var("GPU_SHARE_IFACE") {
        Ok(names) if !names.trim().is_empty() => {
            let names: Vec<&str> = names.split(',').map(str::trim).collect();
            all.into_iter().filter(|i| names.contains(&i.name.as_str())).collect()
        }
        _ => pick_interfaces(&all),
    }
}

/// The other addresses in an interface's /24 (spec §6.6: the subnet scan covers the local /24,
/// even on a wider network, to keep the scan to 254 probes).
pub fn subnet_24(iface: &Iface) -> Vec<Ipv4Addr> {
    let [a, b, c, _] = iface.ip.octets();
    (1..=254).map(|d| Ipv4Addr::new(a, b, c, d)).filter(|ip| *ip != iface.ip).collect()
}

/// A UDP socket on the discovery port, joined to the group on every given interface. Several
/// sockets on one machine can share the port (the host and client sides of one app, or two
/// apps), so address reuse is on.
pub fn multicast_socket(ifaces: &[Iface]) -> std::io::Result<tokio::net::UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris")))]
    s.set_reuse_port(true)?;
    s.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, MULTICAST_PORT)).into())?;
    let mut joined = 0;
    for i in ifaces {
        if s.join_multicast_v4(&MULTICAST_GROUP, &i.ip).is_ok() {
            joined += 1;
        }
    }
    if joined == 0 {
        // no LAN interface (or none would join): fall back to the default route's interface
        s.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    }
    s.set_multicast_loop_v4(true)?;
    s.set_multicast_ttl_v4(1)?; // never leaves the local network
    s.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(s.into())
}

/// Sends `payload` to the group out of each interface (or the default one if none).
pub async fn send_multicast(sock: &tokio::net::UdpSocket, ifaces: &[Iface], payload: &[u8]) {
    let dest = SocketAddr::V4(SocketAddrV4::new(MULTICAST_GROUP, MULTICAST_PORT));
    if ifaces.is_empty() {
        let _ = sock.send_to(payload, dest).await;
        return;
    }
    for i in ifaces {
        let s = socket2::SockRef::from(sock);
        if s.set_multicast_if_v4(&i.ip).is_ok() {
            let _ = sock.send_to(payload, dest).await;
        }
    }
}

/// True when Ubuntu's firewall is switched on (readable without root).
pub fn ufw_enabled() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .map(|s| s.lines().any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes")))
        .unwrap_or(false)
}

/// Whether Windows has a connected network marked Public (`P7-T2`). On Public networks Windows
/// blocks other computers from reaching this one, so sharing can't work. `None` elsewhere.
#[cfg(windows)]
pub fn network_is_public() -> Option<bool> {
    use windows::Win32::Networking::NetworkListManager::{
        INetwork, INetworkListManager, NetworkListManager, NLM_ENUM_NETWORK_CONNECTED, NLM_NETWORK_CATEGORY_PUBLIC,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let nlm: INetworkListManager = CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL).ok()?;
        let nets = nlm.GetNetworks(NLM_ENUM_NETWORK_CONNECTED).ok()?;
        let mut public = false;
        loop {
            let mut one: [Option<INetwork>; 1] = [None];
            let mut fetched = 0u32;
            if nets.Next(&mut one, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            if let Some(n) = one[0].take() {
                if n.GetCategory().ok()? == NLM_NETWORK_CATEGORY_PUBLIC {
                    public = true;
                }
            }
        }
        Some(public)
    }
}

#[cfg(not(windows))]
pub fn network_is_public() -> Option<bool> {
    None
}

/// The explanation shown when the network is Public.
pub fn public_network_warning(is_public: Option<bool>) -> Option<String> {
    (is_public == Some(true)).then(|| {
        "Windows thinks this network is public, so it keeps other computers out and your friends can't reach this \
         one. Open Settings, then Network & internet, choose your Wi-Fi or Ethernet network, and set its network \
         profile type to Private."
            .to_string()
    })
}

/// What discovery has observed, for [`diagnose`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observations {
    /// LAN interfaces found.
    pub interfaces: usize,
    /// Any announcement arrived, our own looped-back one included.
    pub heard_multicast: bool,
    /// A subnet scan finished.
    pub scanned: bool,
    /// Addresses in the scan that answered at all (accepted or refused the connection).
    pub scan_responders: usize,
    pub ufw_enabled: bool,
}

/// A plain-language explanation for an empty device list, or `None` if nothing looks wrong
/// (spec §6.6, `P3-T7`).
pub fn diagnose(o: &Observations) -> Option<String> {
    if o.interfaces == 0 {
        return Some("This computer doesn't seem to be on a home network. Connect to your Wi-Fi or plug in a network cable.".into());
    }
    if o.scanned && o.scan_responders == 0 {
        return Some(
            "Your Wi-Fi seems to keep devices from seeing each other. This happens on guest networks and some mesh routers \
             (\"client isolation\"). Connect both computers to the main Wi-Fi, or use Add by IP address."
                .into(),
        );
    }
    if o.ufw_enabled {
        return Some(format!(
            "This computer's firewall (ufw) is on and may be blocking other computers. In a terminal, run: \
             sudo ufw allow {MULTICAST_PORT}/udp"
        ));
    }
    if !o.heard_multicast {
        return Some(
            "Computers on this network can't announce themselves; the router may be blocking it. If you know your friend's \
             address, use Add by IP address."
                .into(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i(name: &str, ip: [u8; 4]) -> Iface {
        Iface { name: name.into(), ip: Ipv4Addr::from(ip), netmask: Ipv4Addr::new(255, 255, 255, 0) }
    }

    #[test]
    fn keeps_wifi_and_ethernet_and_drops_the_rest() {
        let all = vec![
            i("lo", [127, 0, 0, 1]),
            i("wlp2s0", [192, 168, 1, 23]),
            i("enp3s0", [10, 0, 0, 5]),
            i("docker0", [172, 17, 0, 1]),
            i("br-1a2b3c", [172, 18, 0, 1]),
            i("veth12ab", [172, 17, 0, 2]),
            i("virbr0", [192, 168, 122, 1]),
            i("tailscale0", [100, 101, 102, 103]),
            i("wg0", [10, 8, 0, 2]),
            i("tun0", [10, 9, 0, 2]),
            i("vEthernet (WSL)", [172, 20, 0, 1]),
            i("VirtualBox Host-Only Network", [192, 168, 56, 1]),
            i("Wi-Fi", [192, 168, 0, 14]),
            i("Ethernet", [169, 254, 3, 4]),
        ];
        let names: Vec<String> = pick_interfaces(&all).into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["wlp2s0", "enp3s0", "Wi-Fi"]);
    }

    #[test]
    fn the_scan_covers_the_slash_24_without_ourselves() {
        let ips = subnet_24(&i("wlan0", [192, 168, 1, 23]));
        assert_eq!(ips.len(), 253);
        assert!(!ips.contains(&Ipv4Addr::new(192, 168, 1, 23)));
        assert_eq!(ips[0], Ipv4Addr::new(192, 168, 1, 1));
    }

    #[test]
    fn a_public_network_gets_an_explanation_and_nothing_else_does() {
        assert!(public_network_warning(Some(true)).unwrap().contains("Private"));
        assert_eq!(public_network_warning(Some(false)), None);
        assert_eq!(public_network_warning(None), None);
    }

    #[test]
    fn diagnosis_in_order_of_likelihood() {
        let ok = Observations { interfaces: 1, heard_multicast: true, ..Default::default() };
        assert_eq!(diagnose(&ok), None);
        assert!(diagnose(&Observations::default()).unwrap().contains("home network"));
        let isolated = Observations { interfaces: 1, heard_multicast: true, scanned: true, scan_responders: 0, ..Default::default() };
        assert!(diagnose(&isolated).unwrap().contains("client isolation"));
        let ufw = Observations { interfaces: 1, heard_multicast: true, ufw_enabled: true, ..Default::default() };
        assert!(diagnose(&ufw).unwrap().contains("ufw allow 53737/udp"));
        let silent = Observations { interfaces: 1, ..Default::default() };
        assert!(diagnose(&silent).unwrap().contains("Add by IP"));
    }
}
