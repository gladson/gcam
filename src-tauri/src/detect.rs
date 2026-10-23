use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub brand: String,
    pub open_ports: Vec<u16>,
    pub server: Option<String>,
    pub suggested_path: String,
    pub note: String,
}

/// Portas comuns de câmeras: HTTP, RTSP, Tuya, Hikvision, XMEye, Dahua.
const PORTS: [u16; 7] = [80, 554, 6668, 8000, 8080, 34567, 37777];
const GENERIC_PATH: &str = "/cam/realmonitor?channel=1&subtype=0";

fn connect(ip: Ipv4Addr, port: u16, timeout_ms: u64) -> Option<TcpStream> {
    let addr = SocketAddr::from((ip, port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(1000))).ok()?;
    Some(stream)
}

pub(crate) fn request(ip: Ipv4Addr, port: u16, req: &str) -> Option<String> {
    let mut stream = connect(ip, port, 800)?;
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf[..n]).into_owned())
}

pub(crate) fn header(resp: &str, name: &str) -> Option<String> {
    resp.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

/// Descobre a marca pelas portas abertas e pelo cabeçalho "Server" da câmera.
/// É uma heurística: se errar, o usuário pode trocar a marca no formulário.
#[tauri::command]
pub fn detect_camera(ip: String) -> Result<Detection, String> {
    let ip: Ipv4Addr = ip
        .trim()
        .parse()
        .map_err(|_| "IP inválido".to_string())?;

    let open: Vec<u16> = PORTS
        .iter()
        .copied()
        .filter(|&p| connect(ip, p, 500).is_some())
        .collect();
    let has = |p: u16| open.contains(&p);

    let mut server = None;
    if has(80) {
        server = request(ip, 80, &format!("GET / HTTP/1.0\r\nHost: {ip}\r\n\r\n"))
            .and_then(|r| header(&r, "Server"));
    }
    if server.is_none() && has(554) {
        server = request(
            ip,
            554,
            &format!("OPTIONS rtsp://{ip}:554/ RTSP/1.0\r\nCSeq: 1\r\n\r\n"),
        )
        .and_then(|r| header(&r, "Server"));
    }

    let s = server.clone().unwrap_or_default().to_lowercase();

    let (brand, path, note) = if s.contains("hikvision") || s.contains("app-webs") {
        ("hikvision", "/Streaming/Channels/101", "Servidor Hikvision identificado")
    } else if s.contains("dahua") || s.contains("intelbras") {
        ("dahua", GENERIC_PATH, "Servidor Dahua/Intelbras identificado")
    } else if s.contains("tp-link") || s.contains("tapo") {
        ("tapo", "/stream1", "Servidor TP-Link/Tapo identificado")
    } else if has(6668) && !has(554) {
        (
            "tuya",
            "",
            "Só a porta 6668 respondeu: câmera Tuya. O vídeo passa pela nuvem da Tuya",
        )
    } else {
        ("generic", GENERIC_PATH, "Marca não identificada: usando RTSP genérico")
    };

    Ok(Detection {
        brand: brand.to_string(),
        open_ports: open,
        server,
        suggested_path: path.to_string(),
        note: note.to_string(),
    })
}
