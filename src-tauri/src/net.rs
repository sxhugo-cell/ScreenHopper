//! Encrypted sessions (Noise NNpsk0 over TCP) and LAN discovery (mDNS).
use crate::core::{Core, Discovered, Session};
use crate::platform;
use crate::protocol::{self, Msg, PROTOCOL_VERSION};
use anyhow::{anyhow, bail, Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use snow::TransportState;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_SHA256";
const SERVICE: &str = "_screenhopper._tcp.local.";

fn psk(code: &str) -> [u8; 32] {
    let norm: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase();
    let mut h = Sha256::new();
    h.update(b"screenhopper/v1/");
    h.update(norm.as_bytes());
    h.finalize().into()
}

static FRAG_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<()> {
    if data.len() > u16::MAX as usize {
        bail!("frame too large");
    }
    w.write_all(&(data.len() as u16).to_be_bytes()).await?;
    w.write_all(data).await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut l = [0u8; 2];
    r.read_exact(&mut l).await?;
    let n = u16::from_be_bytes(l) as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn handshake(stream: &mut TcpStream, key: [u8; 32], initiator: bool) -> Result<TransportState> {
    let params: snow::params::NoiseParams = PATTERN.parse()?;
    let builder = snow::Builder::new(params).psk(0, &key);
    let mut buf = vec![0u8; 65535];
    if initiator {
        let mut hs = builder.build_initiator()?;
        let n = hs.write_message(&[], &mut buf)?;
        write_frame(stream, &buf[..n]).await?;
        let m = read_frame(stream).await?;
        hs.read_message(&m, &mut buf)?;
        Ok(hs.into_transport_mode()?)
    } else {
        let mut hs = builder.build_responder()?;
        let m = read_frame(stream).await?;
        hs.read_message(&m, &mut buf)?;
        let n = hs.write_message(&[], &mut buf)?;
        write_frame(stream, &buf[..n]).await?;
        Ok(hs.into_transport_mode()?)
    }
}

pub async fn listen(core: Arc<Core>) {
    let port = core.settings.read().port;
    let l = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            core.log(format!("Cannot listen on port {port}: {e}"));
            return;
        }
    };
    loop {
        match l.accept().await {
            Ok((s, addr)) => {
                let c = core.clone();
                tokio::spawn(async move {
                    if let Err(e) = run_session(c.clone(), s, false, addr.to_string(), false).await {
                        log::debug!("inbound session ended: {e:#}");
                    }
                });
            }
            Err(e) => {
                log::warn!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

pub async fn connect(core: Arc<Core>, addr: SocketAddr) -> Result<()> {
    let s = timeout(Duration::from_secs(4), TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("connection timed out"))??;
    run_session(core, s, true, addr.to_string(), false).await
}

/// Like `connect`, but for computers found by scanning the network: refuses ones whose user disconnected on purpose.
async fn connect_scanned(core: Arc<Core>, addr: SocketAddr) -> Result<()> {
    let s = timeout(Duration::from_secs(4), TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow!("connection timed out"))??;
    run_session(core, s, true, addr.to_string(), true).await
}

async fn run_session(core: Arc<Core>, mut stream: TcpStream, initiator: bool, addr: String, scanned: bool) -> Result<()> {
    stream.set_nodelay(true)?;
    let key = psk(&core.settings.read().pairing_code);
    let ts = timeout(Duration::from_secs(6), handshake(&mut stream, key, initiator))
        .await
        .map_err(|_| anyhow!("handshake timed out"))?
        .context("pairing failed - both computers must use the same pairing code")?;
    let ts = Arc::new(Mutex::new(ts));
    let (mut rd, mut wr) = stream.into_split();

    let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel::<Msg>();
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<Msg>(8);

    {
        let s = core.settings.read();
        let _ = ctrl_tx.send(Msg::Hello {
            id: s.device_id.clone(),
            name: s.device_name.clone(),
            os: platform::os_name().to_string(),
            version: PROTOCOL_VERSION,
        });
    }

    // Writer: control messages always win over bulk data so the mouse never stalls behind a file.
    let ts_w = ts.clone();
    let writer = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut buf = vec![0u8; 65535];
        loop {
            let m = tokio::select! {
                biased;
                m = ctrl_rx.recv() => match m { Some(m) => m, None => break },
                m = bulk_rx.recv() => match m { Some(m) => m, None => break },
                _ = tick.tick() => Msg::Ping,
            };
            let plain = protocol::encode(&m);
            let pieces: Vec<Vec<u8>> = if plain.len() > protocol::FRAG_MAX {
                let total = plain.chunks(protocol::FRAG_MAX).count() as u16;
                let id = FRAG_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                plain
                    .chunks(protocol::FRAG_MAX)
                    .enumerate()
                    .map(|(i, c)| protocol::encode(&Msg::Frag { id, idx: i as u16, total, data: c.to_vec() }))
                    .collect()
            } else {
                vec![plain]
            };
            let mut failed = false;
            for piece in &pieces {
                let n = match ts_w.lock().write_message(piece, &mut buf) {
                    Ok(n) => n,
                    Err(_) => {
                        failed = true;
                        break;
                    }
                };
                if write_frame(&mut wr, &buf[..n]).await.is_err() {
                    failed = true;
                    break;
                }
            }
            if failed {
                break;
            }
            if matches!(m, Msg::Bye) {
                break;
            }
        }
    });

    let mut buf = vec![0u8; 65535];
    let mut read_msg = |frame: Vec<u8>| -> Result<Msg> {
        let n = ts.lock().read_message(&frame, &mut buf)?;
        protocol::decode(&buf[..n])
    };

    // First message must be Hello.
    let first = timeout(Duration::from_secs(6), read_frame(&mut rd))
        .await
        .map_err(|_| anyhow!("no hello"))??;
    let hello = read_msg(first)?;
    let Msg::Hello { id, name, os, version } = hello else { bail!("expected hello") };
    if version != PROTOCOL_VERSION {
        bail!("{name} runs a different Screen Hopper version - install the same version on both computers");
    }
    let my_id = core.settings.read().device_id.clone();
    if id == my_id {
        bail!("connected to self");
    }
    if scanned && core.suppressed.lock().contains(&id) {
        bail!("disconnected on purpose");
    }
    let sess = Arc::new(Session { id: id.clone(), name: name.clone(), os, addr, ctrl: ctrl_tx.clone(), bulk: bulk_tx });
    {
        let mut s = core.sessions.lock();
        if s.contains_key(&id) {
            bail!("duplicate session");
        }
        s.insert(id.clone(), sess.clone());
    }
    if let Some(d) = core.discovered.lock().get_mut(&id) {
        d.error = None;
    }
    core.suppressed.lock().remove(&id);
    core.on_connected(&sess);

    let mut asm: Option<(u32, u16, Vec<u8>)> = None;
    let result: Result<()> = async {
        loop {
            let frame = timeout(Duration::from_secs(20), read_frame(&mut rd))
                .await
                .map_err(|_| anyhow!("peer timed out"))??;
            let mut msg = read_msg(frame)?;
            if let Msg::Frag { id: fid, idx, total, data } = msg {
                let ok = match &mut asm {
                    Some((i, next, _)) if *i == fid && *next == idx => true,
                    _ if idx == 0 => {
                        asm = Some((fid, 0, Vec::new()));
                        true
                    }
                    _ => false,
                };
                if !ok {
                    bail!("broken message pieces");
                }
                let a = asm.as_mut().unwrap();
                a.2.extend_from_slice(&data);
                a.1 += 1;
                if a.2.len() > 32 << 20 {
                    bail!("message too large");
                }
                if a.1 < total {
                    continue;
                }
                let whole = asm.take().unwrap().2;
                msg = protocol::decode(&whole)?;
            }
            match msg {
                Msg::Ping => {
                    let _ = ctrl_tx.send(Msg::Pong);
                }
                Msg::Pong => {}
                Msg::Bye => break,
                m => {
                    let c = core.clone();
                    let pid = id.clone();
                    crate::crashlog::guard((), move || c.handle(&pid, m));
                }
            }
        }
        Ok(())
    }
    .await;

    core.sessions.lock().remove(&id);
    writer.abort();
    core.on_disconnected(&id, &name);
    result
}

// ------------------------------------------------------------------------------------ discovery

pub fn advertise(core: &Arc<Core>) -> Option<ServiceDaemon> {
    let (id, name, port) = {
        let s = core.settings.read();
        (s.device_id.clone(), s.device_name.clone(), s.port)
    };
    let daemon = ServiceDaemon::new().ok()?;
    let host = format!("sh-{id}.local.");
    let props = [("id", id.as_str()), ("name", name.as_str()), ("os", platform::os_name())];
    let info = ServiceInfo::new(SERVICE, &id, &host, "", port, &props[..]).ok()?.enable_addr_auto();
    daemon.register(info).ok()?;
    Some(daemon)
}

pub async fn discover(core: Arc<Core>, daemon: ServiceDaemon) {
    let rx = match daemon.browse(SERVICE) {
        Ok(r) => r,
        Err(e) => {
            core.log(format!("Discovery unavailable: {e}"));
            return;
        }
    };
    let my_id = core.settings.read().device_id.clone();
    while let Ok(ev) = rx.recv_async().await {
        match ev {
            ServiceEvent::ServiceResolved(info) => {
                let props = info.get_properties();
                let id = props.get_property_val_str("id").unwrap_or_default().to_string();
                if id.is_empty() || id == my_id {
                    continue;
                }
                let name = props.get_property_val_str("name").unwrap_or("Unknown").to_string();
                let os = props.get_property_val_str("os").unwrap_or("").to_string();
                let addrs: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
                let mut d = core.discovered.lock();
                let prev_err = d.get(&id).and_then(|x| x.error.clone());
                d.insert(
                    id.clone(),
                    Discovered { id, name, os, addrs, port: info.get_port(), error: prev_err, next_try: Instant::now() },
                );
                drop(d);
                core.emit();
            }
            ServiceEvent::ServiceRemoved(_, full) => {
                let id = full.split('.').next().unwrap_or("").to_string();
                core.discovered.lock().remove(&id);
                core.emit();
            }
            _ => {}
        }
    }
}

/// Dial discovered peers whose id sorts after ours (the other side dials us otherwise).
/// Try every IPv4 address of a discovered computer; remember the error if none works.
pub fn dial(core: Arc<Core>, d: Discovered) {
    core.connecting.lock().insert(d.id.clone());
    if let Some(x) = core.discovered.lock().get_mut(&d.id) {
        x.next_try = Instant::now() + Duration::from_secs(8);
    }
    let c = core.clone();
    tokio::spawn(async move {
        let mut addrs = d.addrs.clone();
        addrs.sort_by_key(|a| a.is_ipv6());
        let mut last: Option<anyhow::Error> = None;
        let mut ok = false;
        for a in addrs {
            if a.is_ipv6() {
                continue;
            }
            match connect(c.clone(), SocketAddr::new(a, d.port)).await {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => {
                    let pairing = e.to_string().starts_with("pairing");
                    last = Some(e);
                    if pairing {
                        break;
                    }
                }
            }
        }
        if !ok {
            if let (Some(e), Some(x)) = (last, c.discovered.lock().get_mut(&d.id)) {
                x.error = Some(e.to_string());
            }
        }
        c.connecting.lock().remove(&d.id);
        c.emit();
    });
}

/// Fallback for networks where automatic discovery (mDNS) is blocked: every 15 s, look for other Screen Hopper
/// listeners on this computer's /24 network and connect to the ones we are not already connected to.
pub async fn scan_lan(core: Arc<Core>) {
    tokio::time::sleep(Duration::from_secs(6)).await;
    loop {
        if !core.offline.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(IpAddr::V4(me)) = local_ipv4() {
                let port = core.settings.read().port;
                let o = me.octets();
                let known: Vec<String> = core.sessions.lock().values().map(|s| ip_of(&s.addr)).collect();
                let mut set = tokio::task::JoinSet::new();
                for last in 1..255u8 {
                    if last == o[3] {
                        continue;
                    }
                    let ip = IpAddr::from([o[0], o[1], o[2], last]);
                    if known.contains(&ip.to_string()) {
                        continue;
                    }
                    set.spawn(async move {
                        let ok = timeout(Duration::from_millis(500), TcpStream::connect(SocketAddr::new(ip, port))).await.map(|r| r.is_ok()).unwrap_or(false);
                        (ip, ok)
                    });
                }
                let mut found = vec![];
                while let Some(Ok((ip, ok))) = set.join_next().await {
                    if ok {
                        found.push(ip);
                    }
                }
                for ip in found {
                    let c = core.clone();
                    tokio::spawn(async move {
                        if let Err(e) = connect_scanned(c, SocketAddr::new(ip, port)).await {
                            log::debug!("scan connect {ip}: {e:#}");
                        }
                    });
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

fn ip_of(addr: &str) -> String {
    addr.rsplit_once(':').map(|x| x.0).unwrap_or(addr).trim_matches(|c| c == '[' || c == ']').to_string()
}

/// This computer's LAN address (no packet is sent; the OS just picks the outgoing interface).
fn local_ipv4() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.255.254:9").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

/// Dial discovered peers whose id sorts after ours (the other side dials us otherwise).
/// Skipped while this computer is disconnected, and for computers whose user disconnected.
pub async fn auto_connect(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if core.offline.load(std::sync::atomic::Ordering::Relaxed) {
            continue;
        }
        let my_id = core.settings.read().device_id.clone();
        let suppressed = core.suppressed.lock().clone();
        let todo: Vec<Discovered> = {
            let d = core.discovered.lock();
            let s = core.sessions.lock();
            let c = core.connecting.lock();
            d.values()
                .filter(|x| {
                    my_id < x.id
                        && !suppressed.contains(&x.id)
                        && !s.contains_key(&x.id)
                        && !c.contains(&x.id)
                        && x.next_try <= Instant::now()
                })
                .cloned()
                .collect()
        };
        for d in todo {
            dial(core.clone(), d);
        }
    }
}

/// Dial every known computer now, regardless of who normally dials (used by Connect buttons).
pub fn dial_all(core: &Arc<Core>, only: Option<&str>) {
    let todo: Vec<Discovered> = {
        let d = core.discovered.lock();
        let s = core.sessions.lock();
        let c = core.connecting.lock();
        d.values()
            .filter(|x| only.map(|o| o == x.id).unwrap_or(true) && !s.contains_key(&x.id) && !c.contains(&x.id))
            .cloned()
            .collect()
    };
    for d in todo {
        dial(core.clone(), d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use std::time::Duration;

    fn mk(name: &str, port: u16, code: &str, dir: &std::path::Path) -> Arc<Core> {
        let mut s = Settings::default();
        s.device_name = name.into();
        s.port = port;
        s.pairing_code = code.into();
        s.receive_dir = dir.to_string_lossy().into_owned();
        Core::with_settings(tokio::runtime::Handle::current(), s)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pair_encrypt_and_transfer() {
        let tmp = std::env::temp_dir().join(format!("sh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SCREEN_HOPPER_CONFIG_DIR", tmp.join("cfg"));
        let (da, db) = (tmp.join("a"), tmp.join("b"));
        let a = mk("A", 34801, "AAAA-1111", &da);
        let b = mk("B", 34802, "AAAA-1111", &db);
        let bad = mk("C", 34803, "ZZZZ-9999", &tmp.join("c"));
        tokio::spawn(listen(b.clone()));
        tokio::spawn(listen(bad.clone()));
        tokio::time::sleep(Duration::from_millis(200)).await;

        // wrong pairing code must be rejected
        let e = connect(a.clone(), "127.0.0.1:34803".parse().unwrap()).await.unwrap_err();
        assert!(e.to_string().starts_with("pairing"), "got {e}");
        assert!(bad.sessions.lock().is_empty());

        // right code connects
        let a2 = a.clone();
        tokio::spawn(async move { let _ = connect(a2, "127.0.0.1:34802".parse().unwrap()).await; });
        for _ in 0..50 {
            if !a.sessions.lock().is_empty() && !b.sessions.lock().is_empty() { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(a.sessions.lock().len(), 1);
        assert_eq!(b.sessions.lock().len(), 1);

        // file + folder transfer
        let src = tmp.join("src");
        std::fs::create_dir_all(src.join("folder/inner")).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(src.join("big.bin"), &big).unwrap();
        std::fs::write(src.join("folder/inner/note.txt"), b"hello").unwrap();
        crate::transfer::send_paths(a.clone(), None, vec![src.join("big.bin"), src.join("folder")], false).await;
        for _ in 0..50 {
            if db.join("folder/inner/note.txt").exists() && std::fs::metadata(db.join("big.bin")).map(|m| m.len()).unwrap_or(0) == 300_000 { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(std::fs::read(db.join("big.bin")).unwrap(), big);
        assert_eq!(std::fs::read(db.join("folder/inner/note.txt")).unwrap(), b"hello");

        // layout propagates and is mirrored
        let aid = a.settings.read().device_id.clone();
        let bid = b.settings.read().device_id.clone();
        a.set_layout(&bid, 1440.0, -300.0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        // shared layout: B sees A at the mirrored position
        assert_eq!(b.rel_positions().get(&aid).copied(), Some((-1440.0, 300.0)));
        assert_eq!(a.rel_positions().get(&bid).copied(), Some((1440.0, -300.0)));
        assert!(!b.peer_mons.lock().is_empty() && !a.peer_mons.lock().is_empty());

        // pause keeps the connection but marks hopping paused on the other side
        a.set_paused(true);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(b.peer_paused.lock().contains(&aid));
        assert_eq!(a.sessions.lock().len(), 1, "pause must not disconnect");
        assert!(b.hop_paused(&aid));
        a.set_paused(false);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!b.peer_paused.lock().contains(&aid));

        // connected computers can't be deleted; offline records can
        assert!(!b.delete_peer(&aid));
        b.settings.write().peers.push(crate::config::PeerLayout { id: "old".into(), name: "Old PC".into(), ..Default::default() });
        b.settings.write().positions.insert("old".into(), (5000.0, 0.0));
        assert_eq!(b.clear_offline(), 1);
        assert!(b.settings.read().peers.iter().all(|p| p.id != "old"));
        assert!(b.settings.read().peers.iter().any(|p| p.id == aid));
        assert!(!b.settings.read().positions.contains_key("old"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Three computers must agree on one layout, with nobody stacked on top of anybody else.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn three_computers_share_one_layout() {
        let tmp = std::env::temp_dir().join(format!("sh-test3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SCREEN_HOPPER_CONFIG_DIR", tmp.join("cfg"));
        let a = mk("A", 34811, "K", &tmp.join("a"));
        let b = mk("B", 34812, "K", &tmp.join("b"));
        let c = mk("C", 34813, "K", &tmp.join("c"));
        for x in [&a, &b, &c] {
            tokio::spawn(listen(x.clone()));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        // full mesh, like auto-discovery produces
        for (from, port) in [(&a, 34812u16), (&a, 34813), (&b, 34813)] {
            let f = from.clone();
            tokio::spawn(async move { let _ = connect(f, format!("127.0.0.1:{port}").parse().unwrap()).await; });
        }
        for _ in 0..60 {
            let ok = [&a, &b, &c].iter().all(|x| x.sessions.lock().len() == 2 && x.rel_positions().len() == 2);
            if ok { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let ids: Vec<String> = [&a, &b, &c].iter().map(|x| x.settings.read().device_id.clone()).collect();
        // everybody holds the same map
        let pa = a.settings.read().positions.clone();
        assert_eq!(pa.len(), 3, "all three placed: {pa:?}");
        assert_eq!(pa, b.settings.read().positions.clone());
        assert_eq!(pa, c.settings.read().positions.clone());
        // and nobody overlaps
        let mut xs: Vec<f64> = ids.iter().map(|i| pa[i].0).collect();
        xs.sort_by(|p, q| p.partial_cmp(q).unwrap());
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "computers stacked: {pa:?}");
        // a drag on one computer reaches the other two
        a.set_layout(&ids[2], 0.0, 900.0);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let want = (pa[&ids[0]].0, pa[&ids[0]].1 + 900.0);
        assert_eq!(b.settings.read().positions[&ids[2]], want);
        assert_eq!(c.settings.read().positions[&ids[2]], want);

        // Pause on B takes only B out: A<->C hopping is unaffected.
        b.set_paused(true);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(a.hop_paused(&ids[1]) && c.hop_paused(&ids[1]));
        assert!(!a.hop_paused(&ids[2]) && !c.hop_paused(&ids[0]), "pause leaked to other computers");
        b.set_paused(false);
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Disconnect on A drops A's links; B and C stay linked and won't auto-dial A.
        a.set_offline(true);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(a.sessions.lock().len(), 0);
        assert_eq!(b.sessions.lock().len(), 1);
        assert!(b.suppressed.lock().contains(&ids[0]) && c.suppressed.lock().contains(&ids[0]));

        // Connect on A dials everyone again (normally found via discovery).
        for (x, port) in [(&b, 34812u16), (&c, 34813)] {
            let id = x.settings.read().device_id.clone();
            a.discovered.lock().insert(id.clone(), crate::core::Discovered {
                id, name: "x".into(), os: "macOS".into(), addrs: vec!["127.0.0.1".parse().unwrap()], port,
                error: None, next_try: std::time::Instant::now(),
            });
        }
        a.set_offline(false);
        for _ in 0..40 {
            if a.sessions.lock().len() == 2 { break; }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(a.sessions.lock().len(), 2, "Connect did not reconnect");
        assert!(!b.suppressed.lock().contains(&ids[0]), "reconnect should clear the suppression");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// End-to-end test against a real Android device or emulator (`adb forward tcp:25000 tcp:24987`).
/// Run: ANDROID_E2E_DIR=/some/dir cargo test --lib android_e2e -- --ignored --nocapture
#[cfg(test)]
mod android_e2e {
    use super::*;
    use crate::config::Settings;
    use crate::core::Mode;
    use crate::platform::InputEvent;
    use std::process::Command;
    use std::time::Duration;

    fn adb(args: &[&str]) -> String {
        let o = Command::new("adb").args(args).output().expect("adb");
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }
    fn shot(dir: &str, name: &str) {
        let o = Command::new("adb").args(["exec-out", "screencap", "-p"]).output().expect("adb");
        std::fs::write(format!("{dir}/{name}.png"), o.stdout).unwrap();
    }
    async fn wait<F: Fn() -> bool>(what: &str, f: F) {
        for _ in 0..80 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for: {what}");
    }
    fn key(c: &Core, hid: u16) {
        c.on_input(InputEvent::Key { hid, down: true });
        c.on_input(InputEvent::Key { hid, down: false });
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore]
    async fn android_e2e() {
        let dir = std::env::var("ANDROID_E2E_DIR").expect("set ANDROID_E2E_DIR");
        let tmp = std::env::temp_dir().join(format!("sh-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("SCREEN_HOPPER_CONFIG_DIR", tmp.join("cfg"));
        let mut s = Settings::default();
        s.device_name = "TestMac".into();
        s.pairing_code = std::env::var("ANDROID_E2E_CODE").unwrap_or_else(|_| "TEST-0001".into());
        s.port = 34890;
        let a = Core::with_settings(tokio::runtime::Handle::current(), s);
        let a2 = a.clone();
        tokio::spawn(async move {
            let _ = connect(a2, "127.0.0.1:25000".parse().unwrap()).await;
        });
        wait("phone connects", || !a.sessions.lock().is_empty()).await;
        let phone = a.sessions.lock().keys().next().unwrap().clone();
        let os = a.sessions.lock()[&phone].os.clone();
        println!("connected to {phone}: os={os}");
        assert_eq!(os, "Android");
        wait("phone monitors + layout", || a.peer_mons.lock().contains_key(&phone) && a.rel_positions().contains_key(&phone)).await;
        let mons = a.peer_mons.lock()[&phone].clone();
        println!("phone screen (dp): {:?}", mons);
        assert!(mons[0].w > 100.0 && mons[0].h > 300.0);
        println!("relative position of phone: {:?}", a.rel_positions()[&phone]);

        // Put the phone directly to the right of this computer's screen.
        let mine = a.local_mons();
        let w = crate::platform::union_rect(&mine).w;
        a.set_layout(&phone, w, 0.0);
        tokio::time::sleep(Duration::from_millis(400)).await;
        shot(&dir, "00-before");

        // 1. Push the cursor into the right edge: should hop onto the phone.
        let (ox, oy) = (crate::platform::union_rect(&a.mons.read()).x, crate::platform::union_rect(&a.mons.read()).y);
        let swallowed = a.on_input(InputEvent::Move { x: ox + w - 1.0, y: oy + 300.0, dx: 6.0, dy: 0.0 });
        assert!(swallowed, "edge push did not hop");
        assert!(matches!(*a.mode.lock(), Mode::Forwarding { .. }));
        tokio::time::sleep(Duration::from_millis(700)).await;
        println!("keyboard while hopped on: {}", adb(&["shell", "settings get secure default_input_method"]));
        shot(&dir, "01-hopped");

        // 2. Move the pointer over the search box and tap it.
        // The search box sits at about y = 430 dp on this layout; the pointer entered at (6, 300).
        for _ in 0..21 {
            a.on_input(InputEvent::Move { x: 0.0, y: 0.0, dx: 5.0, dy: 6.0 });
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        shot(&dir, "02-moved");
        a.on_input(InputEvent::Button { button: 0, down: true });
        tokio::time::sleep(Duration::from_millis(60)).await;
        a.on_input(InputEvent::Button { button: 0, down: false });
        tokio::time::sleep(Duration::from_millis(1200)).await;
        shot(&dir, "03-tapped");

        // 3. Type "hello".
        for hid in [0x0B, 0x08, 0x0F, 0x0F, 0x12] {
            key(&a, hid);
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
        shot(&dir, "04-typed");

        // 4. Clipboard: computer -> phone, then paste it with Cmd+V into the field.
        let payload = crate::protocol::ClipPayload { text: Some(" from-mac".into()), ..Default::default() };
        let bytes = bincode::serialize(&payload).unwrap();
        a.send_ctrl(&phone, Msg::ClipChunk { id: 77, total: bytes.len() as u64, last: true, data: bytes });
        tokio::time::sleep(Duration::from_millis(700)).await;
        a.on_input(InputEvent::Key { hid: crate::keymap::HID_LGUI, down: true });
        key(&a, 0x19);
        a.on_input(InputEvent::Key { hid: crate::keymap::HID_LGUI, down: false });
        tokio::time::sleep(Duration::from_millis(900)).await;
        shot(&dir, "05-pasted");

        // 5. Backspace twice.
        key(&a, 0x2A);
        key(&a, 0x2A);
        tokio::time::sleep(Duration::from_millis(600)).await;
        shot(&dir, "06-backspace");

        // 6. File: computer -> phone Downloads.
        let fname = format!("hello-{}.txt", std::process::id());
        let f = tmp.join(&fname);
        std::fs::write(&f, "sent over Screen Hopper\n").unwrap();
        crate::transfer::send_paths(a.clone(), Some(phone.clone()), vec![f.clone()], false).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let listing = adb(&["shell", "ls \"/sdcard/Download/Screen Hopper/\""]);
        println!("phone Downloads/Screen Hopper: [{listing}]");
        assert!(listing.contains(&fname), "file did not arrive: {listing}");
        let body = adb(&["shell", &format!("cat \"/sdcard/Download/Screen Hopper/{fname}\"")]);
        assert_eq!(body, "sent over Screen Hopper");

        // 7. Wheel: scroll the page down.
        a.on_input(InputEvent::Key { hid: crate::keymap::HID_LGUI, down: false });
        for _ in 0..6 {
            a.on_input(InputEvent::Wheel { dx: 0, dy: -240 });
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        tokio::time::sleep(Duration::from_millis(900)).await;
        shot(&dir, "07-scrolled");

        // 8. Move off the phone's left edge: control must come back.
        for _ in 0..120 {
            a.on_input(InputEvent::Move { x: 0.0, y: 0.0, dx: -8.0, dy: 0.0 });
            tokio::time::sleep(Duration::from_millis(8)).await;
            if matches!(*a.mode.lock(), Mode::Local) {
                break;
            }
        }
        wait("control returns to the computer", || matches!(*a.mode.lock(), Mode::Local)).await;
        println!("control returned");
        tokio::time::sleep(Duration::from_millis(600)).await;
        println!("keyboard after control returned: {}", adb(&["shell", "settings get secure default_input_method"]));
        tokio::time::sleep(Duration::from_millis(500)).await;
        shot(&dir, "08-returned");
        println!("android_e2e: all steps ran");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
