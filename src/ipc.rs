//! One Snag at a time. Launching Snag again (a shortcut, `Snag.exe <link>`,
//! "Open with") hands its links to the window that's already open, over a
//! loopback socket guarded by a random token, then exits.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::hub::Hub;
use crate::platform;
use crate::settings::{data_dir, write_atomic};
use crate::util;

#[derive(Serialize, Deserialize)]
struct Instance {
    port: u16,
    token: String,
    pid: u32,
}

fn instance_file() -> std::path::PathBuf {
    data_dir().join("instance.json")
}

/// True when another Snag took the links (so this one should exit).
pub fn forward_to_running(links: &[String]) -> bool {
    let Ok(text) = fs::read_to_string(instance_file()) else { return false };
    let Ok(inst) = serde_json::from_str::<Instance>(&text) else { return false };
    if inst.pid == std::process::id() {
        return false;
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, inst.port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(700)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    // Lets the running window come to the front.
    platform::allow_foreground(inst.pid);
    let mut msg = format!("SNAG1 {}\n", inst.token);
    for l in links {
        msg.push_str(&format!("ADD {l}\n"));
    }
    msg.push_str("SHOW\nEND\n");
    if stream.write_all(msg.as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    let _ = BufReader::new(stream).read_line(&mut reply);
    reply.trim() == "OK"
}

pub fn listen(hub: &Hub) {
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return };
    let Ok(addr) = listener.local_addr() else { return };
    let token = format!("{:016x}{:016x}", util::random_u64(), util::random_u64());
    let inst = Instance { port: addr.port(), token: token.clone(), pid: std::process::id() };
    let _ = fs::create_dir_all(data_dir());
    if write_atomic(&instance_file(), serde_json::to_string(&inst).unwrap_or_default().as_bytes()).is_err() {
        return;
    }
    let hub = hub.clone();
    let _ = thread::Builder::new().name("instance".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            handle(&hub, stream, &token);
        }
    });
}

fn handle(hub: &Hub, stream: TcpStream, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream.take(1_000_000));
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || first.trim() != format!("SNAG1 {token}") {
        return; // not another Snag (a web page poking at localhost, say)
    }
    let mut links = Vec::new();
    let mut show = false;
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if let Some(url) = line.strip_prefix("ADD ") {
            links.extend(util::extract_links(url));
        } else if line == "SHOW" {
            show = true;
        } else if line == "END" {
            break;
        }
    }
    hub.lock().inbox.extend(links);
    if show {
        platform::bring_to_front();
    }
    hub.repaint();
    let _ = writer.write_all(b"OK\n");
}

pub fn release() {
    if let Ok(text) = fs::read_to_string(instance_file())
        && let Ok(inst) = serde_json::from_str::<Instance>(&text)
        && inst.pid == std::process::id()
    {
        let _ = fs::remove_file(instance_file());
    }
}
