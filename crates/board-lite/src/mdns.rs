//! A minimal mDNS responder (RFC 6762, 6763) for one service on the default
//! route's interface: `_pd-device._tcp`, with the instance's SRV, TXT and A.
//! It announces when its address appears or changes (DHCP may come late) and
//! answers the queries that name the service, the instance or the host.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

pub const SERVICE: &str = "_pd-device._tcp.local";
const META: &str = "_services._dns-sd._udp.local";
const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const PORT: u16 = 5353;
const TTL_NAME: u32 = 4500;
const TTL_HOST: u32 = 120;
const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;
const TYPE_ANY: u16 = 255;
/// Class IN with the cache-flush bit, for records only this board owns.
const CLASS_FLUSH: u16 = 0x8001;
const CLASS_IN: u16 = 1;

/// What's advertised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advert {
    /// `<instance>._pd-device._tcp.local`'s first label.
    pub instance: String,
    /// `<host>.local`'s first label.
    pub host: String,
    pub port: u16,
    pub txt: Vec<(String, String)>,
}

impl Advert {
    fn instance_name(&self) -> String {
        format!("{}.{SERVICE}", self.instance)
    }

    fn host_name(&self) -> String {
        format!("{}.local", self.host)
    }

    fn names(&self, name: &str) -> bool {
        [SERVICE, META, &self.instance_name(), &self.host_name()]
            .iter()
            .any(|ours| ours.eq_ignore_ascii_case(name))
    }
}

fn put_name(out: &mut Vec<u8>, name: &str) {
    for label in name.split('.') {
        let bytes = &label.as_bytes()[..label.len().min(63)];
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.push(0);
}

fn put_record(out: &mut Vec<u8>, name: &str, kind: u16, class: u16, ttl: u32, data: &[u8]) {
    put_name(out, name);
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&class.to_be_bytes());
    out.extend_from_slice(&ttl.to_be_bytes());
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
}

fn name_bytes(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    put_name(&mut out, name);
    out
}

/// A response with every record of the advert. No name compression: it fits
/// and it stays simple.
pub fn response(advert: &Advert, ip: Ipv4Addr) -> Vec<u8> {
    let mut out = vec![0, 0, 0x84, 0, 0, 0, 0, 5, 0, 0, 0, 0];
    let instance = advert.instance_name();
    let host = advert.host_name();
    put_record(
        &mut out,
        META,
        TYPE_PTR,
        CLASS_IN,
        TTL_NAME,
        &name_bytes(SERVICE),
    );
    put_record(
        &mut out,
        SERVICE,
        TYPE_PTR,
        CLASS_IN,
        TTL_NAME,
        &name_bytes(&instance),
    );
    let mut srv = vec![0, 0, 0, 0];
    srv.extend_from_slice(&advert.port.to_be_bytes());
    srv.extend_from_slice(&name_bytes(&host));
    put_record(&mut out, &instance, TYPE_SRV, CLASS_FLUSH, TTL_HOST, &srv);
    let mut txt = Vec::new();
    for (key, value) in &advert.txt {
        let entry = format!("{key}={value}");
        let bytes = &entry.as_bytes()[..entry.len().min(255)];
        txt.push(bytes.len() as u8);
        txt.extend_from_slice(bytes);
    }
    if txt.is_empty() {
        txt.push(0);
    }
    put_record(&mut out, &instance, TYPE_TXT, CLASS_FLUSH, TTL_NAME, &txt);
    put_record(&mut out, &host, TYPE_A, CLASS_FLUSH, TTL_HOST, &ip.octets());
    out
}

/// Reads a name at `at`, following compression; the name and where the bytes
/// after it start.
fn read_name(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut after = None;
    // A pointer loop can't get past this many jumps.
    for _ in 0..64 {
        let len = *packet.get(at)? as usize;
        match len {
            0 => {
                let end = after.unwrap_or(at + 1);
                return Some((labels.join("."), end));
            }
            1..=63 => {
                let label = packet.get(at + 1..at + 1 + len)?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + len;
            }
            192..=255 => {
                let low = *packet.get(at + 1)? as usize;
                after.get_or_insert(at + 2);
                at = ((len & 0x3f) << 8) | low;
            }
            _ => return None,
        }
    }
    None
}

/// Whether `packet` is a query asking for anything of the advert.
pub fn asks_for(packet: &[u8], advert: &Advert) -> bool {
    if packet.len() < 12 || packet[2] & 0x80 != 0 {
        return false;
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]);
    let mut at = 12;
    for _ in 0..questions {
        let Some((name, next)) = read_name(packet, at) else {
            return false;
        };
        let Some(kind) = packet.get(next..next + 2) else {
            return false;
        };
        let kind = u16::from_be_bytes([kind[0], kind[1]]);
        if advert.names(&name) && matches!(kind, TYPE_PTR | TYPE_SRV | TYPE_TXT | TYPE_A | TYPE_ANY)
        {
            return true;
        }
        at = next + 4;
    }
    false
}

/// The address the default route would send multicast from, if any yet.
fn own_address() -> Option<Ipv4Addr> {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    probe.connect((GROUP, PORT)).ok()?;
    match probe.local_addr().ok()? {
        SocketAddr::V4(address) if !address.ip().is_unspecified() => Some(*address.ip()),
        _ => None,
    }
}

/// Answers forever. Returns only if the port can't be had (another responder
/// on the board), with why.
pub fn run(advert: &Advert) -> std::io::Error {
    let socket = match UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
        Ok(socket) => socket,
        Err(error) => return error,
    };
    let _ = socket.set_multicast_ttl_v4(255);
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut address = None;
    let mut buffer = [0u8; 1500];
    loop {
        // DHCP may come after this starts, or change: join and say so then.
        let now = own_address();
        if now != address {
            address = now;
            if let Some(ip) = address {
                let _ = socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED);
                let packet = response(advert, ip);
                let _ = socket.send_to(&packet, (GROUP, PORT));
                std::thread::sleep(Duration::from_secs(1));
                let _ = socket.send_to(&packet, (GROUP, PORT));
            }
        }
        let Ok((read, from)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        if let Some(ip) = address
            && asks_for(&buffer[..read], advert)
        {
            let packet = response(advert, ip);
            // One-shot queriers (not from port 5353) expect a unicast answer.
            let to = if from.port() == PORT {
                SocketAddr::from((GROUP, PORT))
            } else {
                from
            };
            let _ = socket.send_to(&packet, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advert() -> Advert {
        Advert {
            instance: "luckfox-pico-pro-8f3a1c2d".into(),
            host: "luckfox".into(),
            port: 5899,
            txt: vec![("model".into(), "luckfox-pico-pro".into())],
        }
    }

    fn query(name: &str, kind: u16) -> Vec<u8> {
        let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        put_name(&mut packet, name);
        packet.extend_from_slice(&kind.to_be_bytes());
        packet.extend_from_slice(&1u16.to_be_bytes());
        packet
    }

    #[test]
    fn it_answers_for_its_own_names_only() {
        let ad = advert();
        assert!(asks_for(&query(SERVICE, TYPE_PTR), &ad));
        assert!(asks_for(&query("_PD-DEVICE._TCP.LOCAL", TYPE_PTR), &ad));
        assert!(asks_for(&query(META, TYPE_PTR), &ad));
        assert!(asks_for(&query("luckfox.local", TYPE_A), &ad));
        assert!(asks_for(
            &query("luckfox-pico-pro-8f3a1c2d._pd-device._tcp.local", TYPE_SRV),
            &ad
        ));
        assert!(!asks_for(&query("_http._tcp.local", TYPE_PTR), &ad));
        assert!(!asks_for(&query("luckfox.local", 28), &ad), "no AAAA");
        // A response isn't a query; garbage isn't anything.
        assert!(!asks_for(&response(&ad, Ipv4Addr::LOCALHOST), &ad));
        assert!(!asks_for(&[0u8; 5], &ad));
    }

    #[test]
    fn a_compressed_question_and_a_pointer_loop() {
        let ad = advert();
        // Second question points back at the first's name.
        let mut packet = vec![0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0];
        put_name(&mut packet, "_http._tcp.local");
        packet.extend_from_slice(&[0, 12, 0, 1]);
        put_name(&mut packet, "_pd-device");
        packet.pop();
        packet.extend_from_slice(&[0xc0, 12 + 6]); // "_tcp.local"
        packet.extend_from_slice(&[0, 12, 0, 1]);
        assert!(asks_for(&packet, &ad));
        let mut looped = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xc0, 12];
        looped.extend_from_slice(&[0, 12, 0, 1]);
        assert!(!asks_for(&looped, &ad));
    }

    #[test]
    fn the_response_carries_all_five_records() {
        let packet = response(&advert(), Ipv4Addr::new(10, 63, 90, 11));
        assert_eq!(&packet[..4], &[0, 0, 0x84, 0]);
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 5);
        let mut at = 12;
        let mut kinds = Vec::new();
        for _ in 0..5 {
            let (_, next) = read_name(&packet, at).unwrap();
            kinds.push(u16::from_be_bytes([packet[next], packet[next + 1]]));
            let length = u16::from_be_bytes([packet[next + 8], packet[next + 9]]) as usize;
            at = next + 10 + length;
        }
        assert_eq!(at, packet.len());
        assert_eq!(kinds, [TYPE_PTR, TYPE_PTR, TYPE_SRV, TYPE_TXT, TYPE_A]);
        assert_eq!(&packet[packet.len() - 4..], &[10, 63, 90, 11]);
    }
}
