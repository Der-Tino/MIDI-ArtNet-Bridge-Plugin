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
                            shared.set_net_error(Some(format!("Senden an {target}: {e}")));
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
