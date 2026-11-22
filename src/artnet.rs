//! Art-Net-Sender in eigenem Thread.
//!
//! - sendet jedes aktive Universum mit `refresh_hz` (Keep-Alive)
//! - Änderungen gehen sofort raus, aber nie schneller als ~44 Hz pro Universum
//! - prüft ca. jede Sekunde, ob Konfig-Dateien gespeichert wurden → neu laden
//! - Netzwerkfehler (Kabel gezogen, WLAN weg) → Socket neu aufbauen, weiter

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::{self, DMX_SIZE};
use crate::shared::{Shared, MAX_BUFS};
use crate::BridgeParams;

const MIN_INTERVAL: Duration = Duration::from_micros(22_700);
const CHECK_INTERVAL: Duration = Duration::from_millis(750);
const TICK: Duration = Duration::from_millis(2);
const PACKET_LEN: usize = 18 + DMX_SIZE;

pub struct Sender {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Sender {
    pub fn start(shared: Arc<Shared>, params: Arc<BridgeParams>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::Builder::new()
            .name("dmx-bridge-artnet".into())
            .spawn(move || run(shared, params, stop2))
            .ok();
        Self { stop, handle }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Konfig-Ordner aus dem Plugin-State, leer = Standardordner
pub fn effective_dir(params: &BridgeParams) -> PathBuf {
    let s = params
        .config_dir
        .read()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if s.is_empty() {
        config::default_config_dir()
    } else {
        PathBuf::from(s)
    }
}

fn make_socket() -> std::io::Result<UdpSocket> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    Ok(s)
}

fn write_header(p: &mut [u8; PACKET_LEN]) {
    p[..8].copy_from_slice(b"Art-Net\0");
    p[8] = 0x00; // OpDmx 0x5000, little endian
    p[9] = 0x50;
    p[10] = 0; // ProtVer 14
    p[11] = 14;
    p[13] = 0; // Physical
    p[16] = (DMX_SIZE >> 8) as u8;
    p[17] = (DMX_SIZE & 0xFF) as u8;
}

fn run(shared: Arc<Shared>, params: Arc<BridgeParams>, stop: Arc<AtomicBool>) {
    let mut packet = [0u8; PACKET_LEN];
    write_header(&mut packet);

    let mut current_dir: Option<PathBuf> = None;
    let mut fp = 0u64;
    let mut last_check: Option<Instant> = None;

    let mut target = SocketAddrV4::new(Ipv4Addr::BROADCAST, 6454);
    let mut refresh = Duration::from_millis(25);

    let mut socket: Option<UdpSocket> = None;
    let mut socket_retry = Instant::now();
    let mut error_shown = false;

    let mut seq: u8 = 0;
    let mut frame = [0u8; DMX_SIZE];
    let mut last_frame = [[0u8; DMX_SIZE]; MAX_BUFS];
    let mut last_time: [Option<Instant>; MAX_BUFS] = [None; MAX_BUFS];

    while !stop.load(Relaxed) {
        let now = Instant::now();

        // ---- Konfiguration (neu) laden ----
        let forced = shared.reload_request.swap(false, Relaxed);
        if forced || last_check.map_or(true, |t| now.duration_since(t) >= CHECK_INTERVAL) {
            last_check = Some(now);
            let dir = effective_dir(&params);
            let new_fp = config::fingerprint(&dir);
            if forced || current_dir.as_ref() != Some(&dir) || new_fp != fp {
                let show = config::load_show(&dir);
                fp = config::fingerprint(&dir);
                target = SocketAddrV4::new(show.artnet_ip, show.artnet_port);
                refresh = Duration::from_secs_f32(1.0 / show.refresh_hz);
                shared.apply_show(&show);
                if let Ok(mut g) = shared.show.lock() {
                    *g = Some(Arc::new(show));
                }
                current_dir = Some(dir);
                last_time = [None; MAX_BUFS];
            }
        }

        // ---- Socket ----
        if socket.is_none() && now >= socket_retry {
            match make_socket() {
                Ok(s) => socket = Some(s),
                Err(e) => {
                    shared.set_net_error(Some(format!("Socket: {e}")));
                    error_shown = true;
                    socket_retry = now + Duration::from_secs(2);
                }
            }
        }

        // ---- Senden ----
        let mut failed = false;
        if params.output_enabled.value() {
            if let Some(sock) = socket.as_ref() {
                let master = if params.blackout.value() {
                    0.0
                } else {
                    params.master.value().clamp(0.0, 1.0)
                };

                for b in 0..MAX_BUFS {
                    if !shared.buf_active[b].load(Relaxed) {
                        continue;
                    }
                    for (a, out) in frame.iter_mut().enumerate() {
                        *out = shared.output_value(b, a, master);
                    }
                    let due = match last_time[b] {
                        None => true,
                        Some(t) => {
                            let el = now.duration_since(t);
                            el >= refresh || (el >= MIN_INTERVAL && frame != last_frame[b])
                        }
                    };
                    if !due {
                        continue;
                    }

                    let uni = shared.buf_universe[b].load(Relaxed);
                    seq = if seq == 255 { 1 } else { seq + 1 };
                    packet[12] = seq;
                    packet[14] = (uni & 0xFF) as u8; // SubUni
                    packet[15] = ((uni >> 8) & 0x7F) as u8; // Net
                    packet[18..].copy_from_slice(&frame);

                    match sock.send_to(&packet, target) {
                        Ok(_) => {
                            shared.packets_sent.fetch_add(1, Relaxed);
                            last_frame[b] = frame;
                            last_time[b] = Some(now);
                            if error_shown {
                                shared.set_net_error(None);
                                error_shown = false;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(e) => {
                            shared.send_errors.fetch_add(1, Relaxed);
                            shared.set_net_error(Some(crate::tf!("Senden an {target}: {e}", "Sending to {target}: {e}")));
                            error_shown = true;
                            failed = true;
                            break;
                        }
                    }
                }
            }
        }
        if failed {
            socket = None;
            socket_retry = now + Duration::from_secs(1);
        }

        thread::sleep(TICK);
    }
}

// ---------------------------------------------------------------------------
// Node-Suche (ArtPoll) und lokale IP
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub ip: Ipv4Addr,
    pub short_name: String,
    pub long_name: String,
}

/// Über welche eigene IP würde der Rechner `target` erreichen?
/// (Es wird nichts gesendet – das Betriebssystem wählt nur die Route.)
pub fn local_ip_for(target: Ipv4Addr) -> Option<Ipv4Addr> {
    let probe = if target.is_broadcast() || target.is_unspecified() {
        Ipv4Addr::new(8, 8, 8, 8)
    } else {
        target
    };
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((probe, 6454)).ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn artpoll_packet() -> [u8; 14] {
    let mut p = [0u8; 14];
    p[..8].copy_from_slice(b"Art-Net\0");
    p[8] = 0x00; // OpPoll 0x2000, little endian
    p[9] = 0x20;
    p[11] = 14; // ProtVer
    p
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

pub fn parse_poll_reply(buf: &[u8]) -> Option<NodeInfo> {
    if buf.len() < 108 || &buf[..8] != b"Art-Net\0" || buf[8] != 0x00 || buf[9] != 0x21 {
        return None;
    }
    Some(NodeInfo {
        ip: Ipv4Addr::new(buf[10], buf[11], buf[12], buf[13]),
        short_name: cstr(&buf[26..44]),
        long_name: cstr(&buf[44..108]),
    })
}

/// Sucht ~2,5 s lang nach Art-Net-Nodes; Ergebnis landet in `shared.nodes`.
pub fn start_discovery(shared: Arc<Shared>) {
    if shared.discovery_running.swap(true, Relaxed) {
        return;
    }
    if let Ok(mut n) = shared.nodes.lock() {
        n.clear();
    }
    let _ = thread::Builder::new()
        .name("dmx-bridge-artpoll".into())
        .spawn(move || {
            let status = match discover(&shared) {
                Ok(0) => crate::i18n::t(
                    "Kein Node geantwortet. Ist der Node an und im selben Netz?",
                    "No node answered. Is the node switched on and in the same network?",
                )
                .to_string(),
                Ok(n) => crate::tf!("{n} Node(s) gefunden.", "{n} node(s) found."),
                Err(e) => crate::tf!("Suche fehlgeschlagen: {e}", "Search failed: {e}"),
            };
            if let Ok(mut s) = shared.discovery_status.lock() {
                *s = status;
            }
            shared.discovery_running.store(false, Relaxed);
        });
}

fn discover(shared: &Shared) -> std::io::Result<usize> {
    // Antworten kommen laut Spezifikation an Port 6454; ist der belegt
    // (z. B. QLC+ offen), mit freiem Port weitermachen.
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 6454))
        .or_else(|_| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(200)))?;

    let mut targets = vec![
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(2, 255, 255, 255),
        Ipv4Addr::new(10, 255, 255, 255),
    ];
    if let Some(ip) = local_ip_for(Ipv4Addr::BROADCAST) {
        let o = ip.octets();
        targets.push(Ipv4Addr::new(o[0], o[1], o[2], 255));
    }
    let poll = artpoll_packet();
    for t in &targets {
        let _ = sock.send_to(&poll, (*t, 6454));
    }

    let own: Vec<Ipv4Addr> = local_ip_for(Ipv4Addr::BROADCAST).into_iter().collect();
    let deadline = Instant::now() + Duration::from_millis(2500);
    let mut buf = [0u8; 1024];
    let mut found = 0;
    while Instant::now() < deadline {
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if let Some(node) = parse_poll_reply(&buf[..n]) {
                if own.contains(&node.ip) {
                    continue;
                }
                if let Ok(mut list) = shared.nodes.lock() {
                    if !list.iter().any(|x| x.ip == node.ip) {
                        list.push(node);
                        found += 1;
                    }
                }
            }
        }
    }
    Ok(found)
}
