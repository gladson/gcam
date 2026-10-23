//! Descoberta de dispositivos na rede local.
//! Varre a sub-rede /24 do computador: sondas UDP (mDNS, NetBIOS, DNS reverso no roteador) vão para
//! todos os IPs e obrigam o sistema a resolver o ARP de cada um; a tabela ARP diz quem está ligado.
//! Depois, SSDP/UPnP, o broadcast da Tuya e algumas portas conhecidas ajudam a dizer o que é cada aparelho.

use crate::{detect, store, tuya_local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, UdpSocket},
    process::Command,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;

/// Portas que ajudam a reconhecer o tipo do aparelho (câmeras, TVs, impressoras, computadores...).
const PORTS: [u16; 29] = [
    21, 22, 23, 53, 80, 139, 443, 445, 515, 554, 631, 1883, 3389, 5000, 5900, 6668, 7000, 8000,
    8001, 8002, 8008, 8009, 8080, 8443, 8554, 9100, 34567, 37777, 62078,
];
/// Conexões TCP simultâneas: abaixo do limite de 256 arquivos abertos do macOS.
const WORKERS: usize = 96;
/// Base de fabricantes por MAC (OUI) mantida pelo Wireshark.
const OUI_URL: &str = "https://www.wireshark.org/download/automated/data/manuf";
const OUI_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

/// Um aparelho encontrado na rede. Os já vistos e agora desligados ficam com `online: false`.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Device {
    /// MAC quando conhecido (o IP pode mudar); senão o IP.
    pub key: String,
    pub ip: String,
    pub mac: Option<String>,
    /// MAC aleatório (celulares modernos): o fabricante não pode ser identificado por ele.
    pub private_mac: bool,
    pub vendor: Option<String>,
    pub name: Option<String>,
    /// Nome dado pelo usuário; a varredura nunca o altera.
    pub alias: Option<String>,
    pub model: Option<String>,
    pub kind: String,
    pub ports: Vec<u16>,
    pub server: Option<String>,
    pub tuya_id: Option<String>,
    pub gateway: bool,
    pub this_device: bool,
    pub online: bool,
    pub last_seen: u64,
}

/// O que o UPnP do aparelho informa sobre ele.
#[derive(Default)]
struct Upnp {
    name: Option<String>,
    manufacturer: Option<String>,
    model: Option<String>,
    device_type: String,
}

// ---------- utilidades ----------

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Roda um comando do sistema e devolve a saída (sem abrir janela de console no Windows).
fn run(cmd: &str, args: &[&str]) -> String {
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// IP do computador na rede local. O `connect` em UDP só escolhe a rota, não envia nada.
fn local_ip() -> Result<Ipv4Addr, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    sock.connect("8.8.8.8:80")
        .map_err(|_| "Sem conexão de rede".to_string())?;
    match sock.local_addr().map_err(|e| e.to_string())?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Ok(ip),
        _ => Err("Não foi possível descobrir o IP deste computador".to_string()),
    }
}

fn same_subnet(ip: Ipv4Addr, subnet: [u8; 3]) -> bool {
    let o = ip.octets();
    o[..3] == subnet && (1..=254).contains(&o[3])
}

/// Roteador padrão: o primeiro IP da sub-rede (diferente do nosso) na saída do comando de rotas.
fn gateway(local: Ipv4Addr) -> Option<Ipv4Addr> {
    let out = if cfg!(target_os = "windows") {
        run("route", &["print", "-4", "0.0.0.0"])
    } else if cfg!(target_os = "linux") {
        run("ip", &["route", "show", "default"])
    } else {
        run("route", &["-n", "get", "default"])
    };
    let o = local.octets();
    out.split_whitespace()
        .filter_map(|t| t.parse::<Ipv4Addr>().ok())
        .find(|&ip| ip != local && same_subnet(ip, [o[0], o[1], o[2]]))
}

// ---------- ARP e fabricante ----------

fn parse_mac(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let bytes = parts
        .iter()
        .map(|p| (!p.is_empty() && p.len() <= 2).then(|| u8::from_str_radix(p, 16).ok()).flatten())
        .collect::<Option<Vec<u8>>>()?;
    Some(bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

/// Tabela ARP (IP → MAC) da sub-rede. Funciona com `arp -an` (macOS), `arp -a` (Windows) e /proc/net/arp (Linux).
fn neighbors(subnet: [u8; 3]) -> HashMap<Ipv4Addr, String> {
    let table = if cfg!(target_os = "linux") {
        fs::read_to_string("/proc/net/arp").unwrap_or_default()
    } else if cfg!(target_os = "windows") {
        run("arp", &["-a"])
    } else {
        run("arp", &["-an"])
    };
    table
        .lines()
        .filter_map(|line| {
            let ip = line
                .split_whitespace()
                .find_map(|t| t.trim_matches(['(', ')']).parse::<Ipv4Addr>().ok())?;
            let mac = line.split_whitespace().find_map(parse_mac)?;
            let valid = same_subnet(ip, subnet)
                && mac != "00:00:00:00:00:00"
                && mac != "FF:FF:FF:FF:FF:FF";
            valid.then_some((ip, mac))
        })
        .collect()
}

/// "Samsung Electronics Co.,Ltd" → "Samsung"; "TP-Link Systems Inc" → "TP-Link".
fn clean_vendor(name: &str) -> String {
    const NOISE: [&str; 20] = [
        "inc", "inc.", "ltd", "ltd.", "co", "co.", "corp", "corp.", "corporation", "llc", "gmbh",
        "limited", "company", "systems", "electronics", "technologies", "technology",
        "international", "communications", "s.a.",
    ];
    let head = name.split(',').next().unwrap_or(name);
    let mut words: Vec<&str> = head.split_whitespace().collect();
    while words.len() > 1 && words.last().is_some_and(|w| NOISE.contains(&w.to_lowercase().as_str())) {
        words.pop();
    }
    words.join(" ")
}

/// Prefixo do MAC em dígitos hexadecimais → fabricante. Blocos de 24, 28 e 36 bits.
fn parse_oui(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|line| {
            let mut cols = line.split('\t').map(str::trim);
            let prefix = cols.next()?;
            let short = cols.next().filter(|s| !s.is_empty())?;
            let long = cols.next().filter(|s| !s.is_empty()).unwrap_or(short);
            let (hex, bits) = match prefix.split_once('/') {
                Some((h, b)) => (h, b.parse::<usize>().ok()?),
                None => (prefix, 24),
            };
            let nibbles: String = hex
                .chars()
                .filter(char::is_ascii_hexdigit)
                .map(|c| c.to_ascii_uppercase())
                .take(bits / 4)
                .collect();
            Some((nibbles, clean_vendor(long)))
        })
        .collect()
}

/// Carrega a base de fabricantes, baixando de novo quando não existe ou tem mais de 30 dias.
/// Sem internet, usa a cópia antiga; sem nenhuma, os fabricantes ficam em branco.
fn load_oui(app: &AppHandle) -> HashMap<String, String> {
    let Ok(dir) = store::data_dir(app) else {
        return HashMap::new();
    };
    let path = dir.join("manuf.txt");
    let fresh = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < OUI_MAX_AGE);
    if !fresh {
        let text = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .and_then(|c| c.get(OUI_URL).send())
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.text());
        if let Ok(text) = text.as_ref().map(String::as_str) {
            if text.contains('\t') {
                let part = dir.join("manuf.txt.part");
                if fs::write(&part, text).is_ok() {
                    let _ = fs::rename(&part, &path);
                }
            }
        }
    }
    parse_oui(&fs::read_to_string(&path).unwrap_or_default())
}

fn vendor_of(oui: &HashMap<String, String>, mac: &str) -> Option<String> {
    let nibbles: String = mac.chars().filter(char::is_ascii_hexdigit).collect();
    [9, 7, 6]
        .iter()
        .find_map(|&n| oui.get(nibbles.get(..n)?).cloned())
}

/// Bit "administrado localmente" do primeiro byte: MAC aleatório.
fn is_private(mac: &str) -> bool {
    u8::from_str_radix(mac.get(..2).unwrap_or("00"), 16).is_ok_and(|b| b & 0x02 != 0)
}

// ---------- DNS, mDNS e NetBIOS ----------

fn reverse_name(ip: Ipv4Addr) -> String {
    let [a, b, c, d] = ip.octets();
    format!("{d}.{c}.{b}.{a}.in-addr.arpa")
}

fn dns_query(id: u16, name: &str, qtype: u16, flags: u16, qclass: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    p.extend(id.to_be_bytes());
    p.extend(flags.to_be_bytes());
    p.extend(1u16.to_be_bytes());
    p.extend([0u8; 6]);
    for label in name.split('.') {
        p.push(label.len() as u8);
        p.extend(label.as_bytes());
    }
    p.push(0);
    p.extend(qtype.to_be_bytes());
    p.extend(qclass.to_be_bytes());
    p
}

/// Lê um nome DNS (com ponteiros de compressão). Devolve o nome e a posição logo após ele.
fn read_name(buf: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    // Limite contra laços de ponteiros em pacotes malformados.
    for _ in 0..64 {
        let len = *buf.get(pos)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(pos + 1)));
        }
        if len & 0xC0 == 0xC0 {
            end.get_or_insert(pos + 2);
            pos = ((len & 0x3F) << 8) | *buf.get(pos + 1)? as usize;
        } else {
            labels.push(String::from_utf8_lossy(buf.get(pos + 1..pos + 1 + len)?).into_owned());
            pos += 1 + len;
        }
    }
    None
}

fn u16_at(buf: &[u8], pos: usize) -> Option<usize> {
    Some(u16::from_be_bytes([*buf.get(pos)?, *buf.get(pos + 1)?]) as usize)
}

/// Primeiro registro PTR da resposta: o nome do aparelho.
fn ptr_answer(buf: &[u8]) -> Option<String> {
    let questions = u16_at(buf, 4)?;
    let records = u16_at(buf, 6)? + u16_at(buf, 8)? + u16_at(buf, 10)?;
    let mut pos = 12;
    for _ in 0..questions {
        pos = read_name(buf, pos)?.1 + 4;
    }
    for _ in 0..records {
        let (_, p) = read_name(buf, pos)?;
        let rdata = p + 10;
        if u16_at(buf, p)? == 12 {
            return read_name(buf, rdata).map(|(n, _)| n);
        }
        pos = rdata + u16_at(buf, p + 8)?;
    }
    None
}

/// Pedido de "node status" do NetBIOS: o Windows responde com o nome do computador.
fn netbios_query() -> Vec<u8> {
    let mut p = vec![0x4E, 0x42, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0x20, b'C', b'K'];
    p.extend([b'A'; 30]);
    p.extend([0, 0, 0x21, 0, 1]);
    p
}

fn netbios_name(buf: &[u8]) -> Option<String> {
    let (_, p) = read_name(buf, 12)?;
    // Depois do nome: tipo, classe, TTL e tamanho (10 bytes), e a quantidade de nomes.
    let count = *buf.get(p + 10)? as usize;
    (0..count).find_map(|i| {
        let e = buf.get(p + 11 + i * 18..p + 11 + (i + 1) * 18)?;
        // Sufixo 0x00 e nome único (não de grupo): o nome da máquina.
        let unique = e[15] == 0 && e[16] & 0x80 == 0;
        let name = String::from_utf8_lossy(&e[..15]).trim().to_string();
        (unique && !name.is_empty()).then_some(name)
    })
}

/// Recebe respostas UDP até o prazo. Erros de "porta inalcançável" (Windows) não encerram a leitura.
fn collect(sock: &UdpSocket, until: Instant) -> Vec<(Ipv4Addr, Vec<u8>)> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() || sock.set_read_timeout(Some(left)).is_err() {
            return out;
        }
        match sock.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(a))) => out.push((*a.ip(), buf[..n].to_vec())),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                return out
            }
            Err(_) => {}
        }
    }
}

/// Envia um pacote para cada IP (duas rodadas, para pegar quem ainda resolvia o ARP) e junta as respostas.
fn udp_sweep(
    hosts: &[Ipv4Addr],
    port: u16,
    packet: impl Fn(Ipv4Addr) -> Vec<u8>,
    wait: Duration,
) -> Vec<(Ipv4Addr, Vec<u8>)> {
    match UdpSocket::bind("0.0.0.0:0") {
        Ok(sock) => sweep(&sock, hosts, port, packet, wait),
        Err(_) => Vec::new(),
    }
}

fn sweep(
    sock: &UdpSocket,
    hosts: &[Ipv4Addr],
    port: u16,
    packet: impl Fn(Ipv4Addr) -> Vec<u8>,
    wait: Duration,
) -> Vec<(Ipv4Addr, Vec<u8>)> {
    let until = Instant::now() + wait;
    let mut out = Vec::new();
    for round in 0..2 {
        for &ip in hosts {
            let _ = sock.send_to(&packet(ip), (ip, port));
        }
        let stop = if round == 0 { Instant::now() + wait / 2 } else { until };
        out.extend(collect(sock, stop.min(until)));
    }
    out
}

/// Pedido de eco ICMP (ping), com o checksum preenchido.
fn icmp_echo(seq: u16) -> Vec<u8> {
    let mut p = vec![8, 0, 0, 0, 0x43, 0x4D];
    p.extend(seq.to_be_bytes());
    p.extend(b"camera-manager");
    let mut sum: u32 = p
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32)
        .sum();
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    p[2..4].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    p
}

/// Resposta de eco (tipo 0). No macOS a resposta chega com o cabeçalho IP na frente.
fn is_echo_reply(buf: &[u8]) -> bool {
    let icmp = match buf.first() {
        Some(&b) if b >> 4 == 4 => buf.get((b & 0x0F) as usize * 4..).unwrap_or_default(),
        _ => buf,
    };
    icmp.first() == Some(&0)
}

/// Ping em todos os IPs: acha aparelhos que não respondem a mais nada (celulares, notebooks com firewall).
/// Usa socket ICMP sem privilégio (macOS e Linux); sem ele (Windows), roda o comando `ping`.
fn ping_sweep(hosts: &[Ipv4Addr], wait: Duration) -> HashSet<Ipv4Addr> {
    use socket2::{Domain, Protocol, Socket, Type};
    if let Ok(sock) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::ICMPV4)) {
        let sock = UdpSocket::from(sock);
        return sweep(&sock, hosts, 0, |ip| icmp_echo(ip.octets()[3] as u16), wait)
            .into_iter()
            .filter(|(_, buf)| is_echo_reply(buf))
            .map(|(ip, _)| ip)
            .collect();
    }
    let args: &[&str] = if cfg!(windows) { &["-n", "1", "-w", "800"] } else { &["-c", "1", "-W", "1"] };
    parallel(hosts.to_vec(), 32, move |ip| {
        let mut all = args.to_vec();
        let target = ip.to_string();
        all.push(&target);
        // Toda resposta de verdade traz o TTL; "destino inalcançável" não.
        run("ping", &all).to_lowercase().contains("ttl=").then_some(ip)
    })
    .into_iter()
    .collect()
}

/// Nomes que o roteador conhece pelo DHCP, via DNS reverso. O id da consulta é o último octeto do IP.
fn reverse_dns(gateway: Ipv4Addr, hosts: &[Ipv4Addr], wait: Duration) -> HashMap<Ipv4Addr, String> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return HashMap::new();
    };
    for &ip in hosts {
        let id = ip.octets()[3] as u16;
        let _ = sock.send_to(&dns_query(id, &reverse_name(ip), 12, 0x0100, 1), (gateway, 53));
    }
    let [a, b, c, _] = gateway.octets();
    collect(&sock, Instant::now() + wait)
        .into_iter()
        .filter(|(src, _)| *src == gateway)
        .filter_map(|(_, buf)| {
            let id = u16_at(&buf, 0)?;
            Some((Ipv4Addr::new(a, b, c, id as u8), ptr_answer(&buf)?))
        })
        .collect()
}

// ---------- SSDP/UPnP ----------

/// Pergunta por multicast quem fala UPnP (TVs, roteadores, impressoras) e guarda o endereço da descrição de cada um.
fn ssdp(wait: Duration) -> HashMap<Ipv4Addr, Vec<String>> {
    let mut map: HashMap<Ipv4Addr, Vec<String>> = HashMap::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return map;
    };
    let msg = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: ssdp:all\r\n\r\n";
    for _ in 0..2 {
        let _ = sock.send_to(msg.as_bytes(), "239.255.255.250:1900");
    }
    for (ip, buf) in collect(&sock, Instant::now() + wait) {
        if let Some(loc) = detect::header(&String::from_utf8_lossy(&buf), "location") {
            let list = map.entry(ip).or_default();
            if !list.contains(&loc) {
                list.push(loc);
            }
        }
    }
    map
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn xml_tag(xml: &str, tag: &str) -> Option<String> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    let value = xml_unescape(xml[start..end].trim());
    (!value.is_empty()).then_some(value)
}

/// Lê a descrição UPnP. Só aceita endereços do próprio aparelho, para não seguir links para fora da rede.
fn upnp_info(ip: Ipv4Addr, locations: &[String]) -> Option<Upnp> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    locations.iter().take(3).find_map(|loc| {
        let url = reqwest::Url::parse(loc).ok()?;
        if url.host_str() != Some(ip.to_string().as_str()) {
            return None;
        }
        let xml = client.get(url).send().ok()?.text().ok()?;
        let model = match (xml_tag(&xml, "modelName"), xml_tag(&xml, "modelNumber")) {
            (Some(n), Some(num)) if !n.contains(&num) => Some(format!("{n} {num}")),
            (n, num) => n.or(num),
        };
        let info = Upnp {
            name: xml_tag(&xml, "friendlyName"),
            manufacturer: xml_tag(&xml, "manufacturer"),
            model,
            device_type: xml_tag(&xml, "deviceType").unwrap_or_default(),
        };
        (info.name.is_some() || info.model.is_some()).then_some(info)
    })
}

// ---------- Tuya ----------

/// Decodifica o anúncio UDP da Tuya: aberto na porta 6666, cifrado com a chave fixa da Tuya na 6667.
fn tuya_payload(key: &[u8], data: &[u8]) -> Option<Value> {
    if data.get(..4)? != [0, 0, 0x55, 0xAA] {
        return None;
    }
    let len = u32::from_be_bytes(data.get(12..16)?.try_into().ok()?) as usize;
    let payload = data.get(16..(16 + len).checked_sub(8)?)?;
    // Com ou sem o código de retorno (4 bytes) antes dos dados.
    [4, 0].iter().find_map(|&skip| {
        let body = payload.get(skip..)?;
        serde_json::from_slice::<Value>(body).ok().or_else(|| {
            let plain = (body.len() % 16 == 0).then(|| tuya_local::decrypt(key, body).ok())??;
            serde_json::from_slice(&plain).ok()
        })
    })
}

/// Escuta os anúncios que aparelhos Tuya mandam a cada poucos segundos. IP → (gwId, versão).
fn tuya_broadcasts(wait: Duration) -> HashMap<Ipv4Addr, (String, String)> {
    let key = md5::compute(b"yGAdlopoPVldABfn").0;
    let until = Instant::now() + wait;
    let listeners: Vec<_> = [6666u16, 6667]
        .into_iter()
        .filter_map(|port| UdpSocket::bind(("0.0.0.0", port)).ok())
        .map(|sock| thread::spawn(move || collect(&sock, until)))
        .collect();
    let mut map = HashMap::new();
    for packets in listeners.into_iter().filter_map(|h| h.join().ok()) {
        for (src, buf) in packets {
            let Some(v) = tuya_payload(&key, &buf) else { continue };
            let Some(id) = v["gwId"].as_str() else { continue };
            let ip = v["ip"].as_str().and_then(|s| s.parse().ok()).unwrap_or(src);
            let version = v["version"].as_str().unwrap_or_default().to_string();
            map.insert(ip, (id.to_string(), version));
        }
    }
    map
}

// ---------- portas e HTTP ----------

/// Testa conexões TCP em paralelo. Devolve (IP, porta, aberta). Porta recusada também prova que o aparelho está ligado.
fn tcp_probe(jobs: Vec<(Ipv4Addr, u16)>, timeout: Duration) -> Vec<(Ipv4Addr, u16, bool)> {
    parallel(jobs, WORKERS, move |(ip, port)| {
        match TcpStream::connect_timeout(&SocketAddr::from((ip, port)), timeout) {
            Ok(_) => Some((ip, port, true)),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => Some((ip, port, false)),
            Err(_) => None,
        }
    })
}

/// Roda `f` em cada item com no máximo `workers` threads e junta os resultados.
fn parallel<T, R>(items: Vec<T>, workers: usize, f: impl Fn(T) -> Option<R> + Send + Sync + 'static) -> Vec<R>
where
    T: Send + 'static,
    R: Send + 'static,
{
    let workers = workers.min(items.len());
    let items = Arc::new(Mutex::new(items.into_iter()));
    let f = Arc::new(f);
    let (tx, rx) = mpsc::channel();
    for _ in 0..workers {
        let (items, f, tx) = (Arc::clone(&items), Arc::clone(&f), tx.clone());
        thread::spawn(move || loop {
            let Some(item) = items.lock().ok().and_then(|mut i| i.next()) else {
                break;
            };
            if let Some(r) = f(item) {
                let _ = tx.send(r);
            }
        });
    }
    drop(tx);
    rx.into_iter().collect()
}

/// Quem está ligado, por TCP. Complementa a tabela ARP, que o macOS pode esconder de outros programas.
fn tcp_alive(hosts: &[Ipv4Addr]) -> HashSet<Ipv4Addr> {
    let jobs = hosts
        .iter()
        .flat_map(|&ip| [80, 443, 7000, 62078].map(|p| (ip, p)))
        .collect();
    tcp_probe(jobs, Duration::from_millis(300))
        .into_iter()
        .map(|(ip, _, _)| ip)
        .collect()
}

fn open_ports(hosts: &[Ipv4Addr]) -> HashMap<Ipv4Addr, Vec<u16>> {
    let jobs = hosts
        .iter()
        .flat_map(|&ip| PORTS.iter().map(move |&p| (ip, p)))
        .collect();
    let mut map: HashMap<Ipv4Addr, Vec<u16>> = HashMap::new();
    for (ip, port, _) in tcp_probe(jobs, Duration::from_millis(500)).into_iter().filter(|r| r.2) {
        map.entry(ip).or_default().push(port);
    }
    map.values_mut().for_each(|v| v.sort_unstable());
    map
}

fn html_title(resp: &str) -> Option<String> {
    let lower = resp.to_lowercase();
    let start = lower.find("<title")?;
    let start = start + lower[start..].find('>')? + 1;
    let end = start + lower[start..].find("</title>")?;
    let title = xml_unescape(resp[start..end].trim());
    (2..=60).contains(&title.chars().count()).then_some(title)
}

/// Cabeçalho "Server" e título da página (HTTP) ou do servidor RTSP.
fn web_info(ip: Ipv4Addr, ports: &[u16]) -> (Option<String>, Option<String>) {
    let mut server = None;
    let mut title = None;
    for port in [80, 8080, 8000].into_iter().filter(|p| ports.contains(p)) {
        if let Some(resp) = detect::request(ip, port, &format!("GET / HTTP/1.0\r\nHost: {ip}\r\n\r\n")) {
            server = server.or_else(|| detect::header(&resp, "Server"));
            title = title.or_else(|| html_title(&resp));
        }
    }
    if server.is_none() && ports.contains(&554) {
        server = detect::request(ip, 554, &format!("OPTIONS rtsp://{ip}:554/ RTSP/1.0\r\nCSeq: 1\r\n\r\n"))
            .and_then(|r| detect::header(&r, "Server"));
    }
    (server, title)
}

// ---------- classificação ----------

fn classify(d: &Device, upnp_type: &str) -> &'static str {
    let has = |p: u16| d.ports.contains(&p);
    let text = [&d.name, &d.vendor, &d.model, &d.server]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .chain([upnp_type])
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let words: HashSet<&str> = text.split(|c: char| !c.is_alphanumeric()).collect();
    // Palavras curtas comparam a palavra inteira; as longas, qualquer trecho.
    let any = |keys: &[&str]| {
        keys.iter()
            .any(|k| if k.len() <= 4 { words.contains(k) } else { text.contains(k) })
    };

    if d.gateway {
        "router"
    } else if d.this_device {
        "computer"
    } else if [554, 8554, 34567, 37777].into_iter().any(has)
        || any(&["camera", "ipc", "nvr", "dvr", "hikvision", "dahua", "intelbras", "reolink", "ezviz", "imou", "yoosee"])
    {
        "camera"
    } else if [515, 631, 9100].into_iter().any(has) || any(&["printer", "epson", "brother", "canon"]) {
        "printer"
    } else if [8001, 8002, 8008, 8009].into_iter().any(has)
        || any(&["tv", "mediarenderer", "chromecast", "roku", "crystal uhd", "bravia", "webos"])
    {
        "tv"
    } else if has(62078) || any(&["iphone", "ipad", "android", "galaxy", "pixel", "redmi", "xiaomi", "edge", "motorola"]) {
        "phone"
    } else if [22, 139, 445, 3389, 5900].into_iter().any(has)
        || any(&["macbook", "imac", "desktop", "laptop", "notebook", "windows", "pc"])
    {
        "computer"
    } else if d.tuya_id.is_some() || has(6668) || has(1883) || any(&["tuya", "espressif", "smart"]) {
        "iot"
    } else {
        "unknown"
    }
}

/// Primeiro rótulo do nome ("MacBook-Pro.local" → "MacBook-Pro"), ignorando nomes técnicos.
fn short_name(name: String) -> Option<String> {
    let first = name.split('.').next().unwrap_or_default().trim().to_string();
    (!first.is_empty() && !first.starts_with('_')).then_some(first)
}

// ---------- varredura ----------

/// `oui` carrega a base de fabricantes; roda em paralelo com as sondas.
fn scan(oui: impl FnOnce() -> HashMap<String, String> + Send + 'static) -> Vec<Device> {
    let Ok(local) = local_ip() else {
        return Vec::new();
    };
    let [a, b, c, _] = local.octets();
    let subnet = [a, b, c];
    let hosts: Vec<Ipv4Addr> = (1..=254).map(|n| Ipv4Addr::new(a, b, c, n)).collect();
    let gw = gateway(local);
    let wait = Duration::from_millis(2500);

    // Fase 1: todas as sondas ao mesmo tempo. Elas também preenchem a tabela ARP.
    let oui = thread::spawn(oui);
    let tcp = thread::spawn({
        let hosts = hosts.clone();
        move || tcp_alive(&hosts)
    });
    let ping = thread::spawn({
        let hosts = hosts.clone();
        move || ping_sweep(&hosts, wait)
    });
    let tuya = thread::spawn(move || tuya_broadcasts(Duration::from_secs(6)));
    let ssdp = thread::spawn(move || ssdp(wait));
    let mdns = thread::spawn({
        let hosts = hosts.clone();
        // Consulta PTR com o bit "resposta unicast": responde direto para a nossa porta.
        move || udp_sweep(&hosts, 5353, |ip| dns_query(0, &reverse_name(ip), 12, 0, 0x8001), wait)
    });
    let nbns = thread::spawn({
        let hosts = hosts.clone();
        move || udp_sweep(&hosts, 137, |_| netbios_query(), wait)
    });
    let dns = thread::spawn({
        let hosts = hosts.clone();
        move || gw.map(|g| reverse_dns(g, &hosts, wait)).unwrap_or_default()
    });

    let mdns: HashMap<Ipv4Addr, String> = mdns
        .join()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(ip, buf)| Some((ip, ptr_answer(&buf)?)))
        .collect();
    let nbns: HashMap<Ipv4Addr, String> = nbns
        .join()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(ip, buf)| Some((ip, netbios_name(&buf)?)))
        .collect();
    let dns = dns.join().unwrap_or_default();
    let ssdp = ssdp.join().unwrap_or_default();
    let tcp = tcp.join().unwrap_or_default();
    let ping = ping.join().unwrap_or_default();
    let arp = neighbors(subnet);

    let alive: BTreeSet<Ipv4Addr> = arp
        .keys()
        .chain(mdns.keys())
        .chain(nbns.keys())
        .chain(ssdp.keys())
        .chain(tcp.iter())
        .chain(ping.iter())
        .copied()
        .chain([local])
        .chain(gw)
        .filter(|&ip| same_subnet(ip, subnet))
        .collect();
    let alive: Vec<Ipv4Addr> = alive.into_iter().collect();

    // Fase 2: portas de quem respondeu; fase 3: HTTP/RTSP e UPnP de cada um, em paralelo.
    let ports = open_ports(&alive);
    let details: Vec<_> = alive
        .iter()
        .map(|&ip| {
            let open = ports.get(&ip).cloned().unwrap_or_default();
            let locations = ssdp.get(&ip).cloned().unwrap_or_default();
            thread::spawn(move || {
                let (server, title) = web_info(ip, &open);
                (ip, open, server, title, upnp_info(ip, &locations))
            })
        })
        .collect();

    let tuya = tuya.join().unwrap_or_default();
    let oui = oui.join().unwrap_or_default();
    let seen = now();

    details
        .into_iter()
        .filter_map(|h| h.join().ok())
        .map(|(ip, open, server, title, upnp)| {
            let upnp = upnp.unwrap_or_default();
            let mac = arp.get(&ip).cloned();
            let private_mac = mac.as_deref().is_some_and(is_private);
            let vendor = mac
                .as_deref()
                .filter(|_| !private_mac)
                .and_then(|m| vendor_of(&oui, m))
                .or(upnp.manufacturer.map(|m| clean_vendor(&m)));
            let tuya_info = tuya.get(&ip);
            let name = upnp
                .name
                .or_else(|| mdns.get(&ip).cloned().and_then(short_name))
                .or_else(|| nbns.get(&ip).cloned())
                .or_else(|| dns.get(&ip).cloned().and_then(short_name))
                .or(title);
            let model = upnp
                .model
                .or_else(|| tuya_info.map(|(_, v)| format!("Tuya {v}").trim().to_string()));
            let mut d = Device {
                key: mac.clone().unwrap_or_else(|| ip.to_string()),
                ip: ip.to_string(),
                mac,
                private_mac,
                vendor,
                name,
                alias: None,
                model,
                kind: String::new(),
                ports: open,
                server,
                tuya_id: tuya_info.map(|(id, _)| id.clone()),
                gateway: Some(ip) == gw,
                this_device: ip == local,
                online: true,
                last_seen: seen,
            };
            d.kind = classify(&d, &upnp.device_type).to_string();
            d
        })
        .collect()
}

// ---------- histórico ----------

fn file(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(store::data_dir(app)?.join("network.json"))
}

fn load(app: &AppHandle) -> Result<Vec<Device>, String> {
    let file = file(app)?;
    if !file.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("network.json inválido: {e}"))
}

fn save(app: &AppHandle, devices: &[Device]) -> Result<(), String> {
    let text = serde_json::to_string_pretty(devices).map_err(|e| e.to_string())?;
    fs::write(file(app)?, text).map_err(|e| e.to_string())
}

fn sort(devices: &mut [Device]) {
    devices.sort_by_key(|d| d.ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(u32::MAX));
}

/// Junta a varredura com o histórico: quem sumiu fica desligado e o que já se sabia não se perde.
fn merge(app: &AppHandle, found: Vec<Device>) -> Result<Vec<Device>, String> {
    let mut all = load(app)?;
    all.iter_mut().for_each(|d| d.online = false);
    for mut d in found {
        if let Some(old) = all.iter_mut().find(|o| o.key == d.key) {
            d.alias = old.alias.take();
            d.name = d.name.or(old.name.take());
            d.vendor = d.vendor.or(old.vendor.take());
            d.model = d.model.or(old.model.take());
            d.server = d.server.or(old.server.take());
            d.tuya_id = d.tuya_id.or(old.tuya_id.take());
            if d.kind == "unknown" {
                d.kind = std::mem::take(&mut old.kind);
            }
            *old = d;
        } else {
            all.push(d);
        }
    }
    // Um desligado com o IP de um ligado é registro velho daquele IP (o aparelho trocou de endereço).
    let online: HashSet<String> = all.iter().filter(|d| d.online).map(|d| d.ip.clone()).collect();
    all.retain(|d| d.online || !online.contains(&d.ip));
    sort(&mut all);
    save(app, &all)?;
    Ok(all)
}

/// Dispositivos da última varredura, sem varrer de novo.
#[tauri::command]
pub fn network_devices(app: AppHandle) -> Result<Vec<Device>, String> {
    let mut all = load(&app)?;
    sort(&mut all);
    Ok(all)
}

/// Varre a rede (leva alguns segundos) e devolve a lista atualizada.
#[tauri::command]
pub async fn scan_network(app: AppHandle) -> Result<Vec<Device>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        local_ip()?;
        let found = scan({
            let app = app.clone();
            move || load_oui(&app)
        });
        merge(&app, found)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Dá um nome ao dispositivo. Nome vazio volta a mostrar o nome detectado.
#[tauri::command]
pub fn rename_device(app: AppHandle, key: String, alias: String) -> Result<Vec<Device>, String> {
    let mut all = load(&app)?;
    let device = all
        .iter_mut()
        .find(|d| d.key == key)
        .ok_or("Dispositivo não encontrado")?;
    let alias = alias.trim();
    device.alias = (!alias.is_empty()).then(|| alias.to_string());
    save(&app, &all)?;
    sort(&mut all);
    Ok(all)
}

/// Remove um dispositivo desligado do histórico.
#[tauri::command]
pub fn forget_device(app: AppHandle, key: String) -> Result<Vec<Device>, String> {
    let mut all = load(&app)?;
    all.retain(|d| d.key != key);
    save(&app, &all)?;
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Varredura real da rede: `cargo test live_scan -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_scan() {
        let path = std::env::var("MANUF").unwrap_or_default();
        let devices = scan(move || parse_oui(&fs::read_to_string(path).unwrap_or_default()));
        for d in &devices {
            println!(
                "{:<14} {:<9} {:<18} {:<22} {:<26} {:<20} {:?} {:?} {:?}",
                d.ip, d.kind, d.mac.clone().unwrap_or_default(), d.vendor.clone().unwrap_or_default(),
                d.name.clone().unwrap_or_default(), d.model.clone().unwrap_or_default(), d.ports, d.server, d.tuya_id
            );
        }
        assert!(!devices.is_empty());
    }

    #[test]
    fn parses_macs_from_each_os() {
        assert_eq!(parse_mac("f4:dd:6:a3:c1:4b").as_deref(), Some("F4:DD:06:A3:C1:4B"));
        assert_eq!(parse_mac("e4-fa-c4-4e-53-50").as_deref(), Some("E4:FA:C4:4E:53:50"));
        assert_eq!(parse_mac("192.168.0.1"), None);
    }

    #[test]
    fn cleans_vendor_names() {
        assert_eq!(clean_vendor("Samsung Electronics Co.,Ltd"), "Samsung");
        assert_eq!(clean_vendor("TP-Link Systems Inc"), "TP-Link");
        assert_eq!(clean_vendor("Apple, Inc."), "Apple");
    }

    #[test]
    fn looks_up_oui_blocks() {
        let db = parse_oui("E4:FA:C4\tTPLink\tTP-Link Systems Inc\n00:1B:C5:00:00/36\tConverging\tConverging Systems Inc.\n");
        assert_eq!(vendor_of(&db, "E4:FA:C4:4E:53:50").as_deref(), Some("TP-Link"));
        assert_eq!(vendor_of(&db, "00:1B:C5:00:00:12").as_deref(), Some("Converging"));
        assert!(is_private("86:71:82:E3:8E:96"));
        assert!(!is_private("E4:FA:C4:4E:53:50"));
    }

    #[test]
    fn reads_ptr_answer() {
        // Resposta com a pergunta repetida e um PTR comprimido apontando para "local".
        let mut r = dns_query(7, "3.0.168.192.in-addr.arpa", 12, 0x8400, 1);
        r[6..8].copy_from_slice(&1u16.to_be_bytes());
        let name_ptr = [0xC0, 0x0C];
        r.extend(name_ptr);
        r.extend([0, 12, 0, 1, 0, 0, 0, 120]);
        let rdata = [4, b'm', b'a', b'c', b'1', 5, b'l', b'o', b'c', b'a', b'l', 0];
        r.extend((rdata.len() as u16).to_be_bytes());
        r.extend(rdata);
        assert_eq!(ptr_answer(&r).as_deref(), Some("mac1.local"));
    }
}
