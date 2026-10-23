//! Protocolo local Tuya 3.4 (porta 6668), o mesmo que o app Tuya usa na rede local.
//! Serve para ler e mudar DPs da câmera, como o rastreamento de movimento.

use crate::tuya_ptz;
use aes::{
    cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt, KeyInit},
    Aes128,
};
use hmac::{Hmac, Mac};
use rsa::rand_core::{OsRng, RngCore};
use serde_json::{json, Map, Value};
use sha2::Sha256;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;

/// DP padrão da Tuya para o rastreamento de movimento em câmeras (motion_tracking).
const TRACKING_DP: &str = "161";

const PREFIX: u32 = 0x0000_55AA;
const SUFFIX: u32 = 0x0000_AA55;
const SESS_KEY_NEG_START: u32 = 0x03;
const SESS_KEY_NEG_RESP: u32 = 0x04;
const SESS_KEY_NEG_FINISH: u32 = 0x05;
const CONTROL_NEW: u32 = 0x0d;
const DP_QUERY_NEW: u32 = 0x10;

fn encrypt(key: &[u8], data: &[u8], pad: bool) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    if pad {
        let n = 16 - buf.len() % 16;
        buf.extend(std::iter::repeat(n as u8).take(n));
    }
    for block in buf.chunks_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
    }
    buf
}

pub(crate) fn decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() % 16 != 0 {
        return Err("Resposta da câmera com tamanho inválido".to_string());
    }
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let mut buf = data.to_vec();
    for block in buf.chunks_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    // Remove o preenchimento PKCS#7 quando ele for válido.
    if let Some(&n) = buf.last() {
        let n = n as usize;
        if (1..=16).contains(&n) && n <= buf.len() && buf[buf.len() - n..].iter().all(|&b| b as usize == n) {
            buf.truncate(buf.len() - n);
        }
    }
    Ok(buf)
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC aceita qualquer chave");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Conexão com a câmera. `key` começa como a localKey e vira a chave de sessão após a negociação.
struct Conn {
    stream: TcpStream,
    seq: u32,
    key: Vec<u8>,
}

impl Conn {
    fn open(ip: &str, local_key: &[u8]) -> Result<Self, String> {
        let addr: SocketAddr = format!("{ip}:6668")
            .parse()
            .map_err(|_| format!("IP inválido: {ip}"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
            .map_err(|e| format!("Não consegui conectar na câmera ({ip}:6668): {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        Ok(Conn { stream, seq: 1, key: local_key.to_vec() })
    }

    /// Envia um quadro 55AA: cabeçalho, dados criptografados, HMAC-SHA256 e sufixo.
    fn send(&mut self, cmd: u32, payload: &[u8]) -> Result<(), String> {
        let enc = encrypt(&self.key, payload, true);
        let mut frame = Vec::with_capacity(enc.len() + 52);
        for v in [PREFIX, self.seq, cmd, (enc.len() + 32 + 4) as u32] {
            frame.extend(v.to_be_bytes());
        }
        frame.extend(&enc);
        let mac = hmac(&self.key, &frame);
        frame.extend(mac);
        frame.extend(SUFFIX.to_be_bytes());
        self.seq += 1;
        self.stream
            .write_all(&frame)
            .map_err(|e| format!("Falha ao enviar para a câmera: {e}"))
    }

    /// Lê um quadro e devolve o comando e os dados já descriptografados.
    fn recv(&mut self) -> Result<(u32, Vec<u8>), String> {
        let mut header = [0u8; 16];
        self.stream
            .read_exact(&mut header)
            .map_err(|e| format!("A câmera não respondeu: {e}"))?;
        let word = |i: usize| u32::from_be_bytes(header[i..i + 4].try_into().unwrap());
        if word(0) != PREFIX {
            return Err("Resposta da câmera em formato desconhecido".to_string());
        }
        let cmd = word(8);
        let len = word(12) as usize;
        let mut body = vec![0u8; len];
        self.stream
            .read_exact(&mut body)
            .map_err(|e| format!("Resposta incompleta da câmera: {e}"))?;
        // Depois dos dados vêm 32 bytes de HMAC e 4 de sufixo. As respostas trazem
        // um código de retorno de 4 bytes antes dos dados criptografados.
        let data_len = len.saturating_sub(36);
        let start = if data_len % 16 == 4 { 4 } else { 0 };
        let enc = &body[start..data_len];
        if enc.is_empty() {
            return Ok((cmd, Vec::new()));
        }
        Ok((cmd, decrypt(&self.key, enc)?))
    }

    /// Negocia a chave de sessão do protocolo 3.4.
    fn negotiate(&mut self) -> Result<(), String> {
        let local_key = self.key.clone();
        let mut local_nonce = [0u8; 16];
        OsRng.fill_bytes(&mut local_nonce);
        self.send(SESS_KEY_NEG_START, &local_nonce)?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let remote_nonce = loop {
            if Instant::now() > deadline {
                return Err("A câmera não respondeu à negociação".to_string());
            }
            let (cmd, data) = self.recv()?;
            if cmd != SESS_KEY_NEG_RESP {
                continue;
            }
            if data.len() < 48 || data[16..48] != hmac(&local_key, &local_nonce)[..] {
                return Err("A câmera recusou a chave local (localKey diferente)".to_string());
            }
            break data[..16].to_vec();
        };

        self.send(SESS_KEY_NEG_FINISH, &hmac(&local_key, &remote_nonce))?;
        let mixed: Vec<u8> = local_nonce.iter().zip(&remote_nonce).map(|(a, b)| a ^ b).collect();
        self.key = encrypt(&local_key, &mixed, false);
        Ok(())
    }

    /// Lê mensagens até achar uma com DPs, ou até acabar o tempo.
    fn wait_dps(&mut self) -> Result<Map<String, Value>, String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let (_, data) = self.recv()?;
            if let Some(dps) = parse_dps(&data) {
                return Ok(dps);
            }
        }
        Err("A câmera não informou o estado".to_string())
    }
}

/// Extrai os DPs de uma resposta. Ela pode vir com o cabeçalho "3.4" e com ou sem o envelope "data".
fn parse_dps(data: &[u8]) -> Option<Map<String, Value>> {
    let data = if data.starts_with(b"3.4") && data.len() > 15 { &data[15..] } else { data };
    let start = data.iter().position(|&b| b == b'{')?;
    let value: Value = serde_json::from_slice(&data[start..]).ok()?;
    let dps = value.get("dps").or_else(|| value.get("data").and_then(|d| d.get("dps")))?;
    dps.as_object().cloned()
}

/// Busca a localKey da câmera na nuvem Tuya e abre uma sessão local com ela.
fn connect(app: &AppHandle, camera_id: &str) -> Result<Conn, String> {
    let cam = tuya_ptz::tuya_camera(app, camera_id)?;
    if cam.ip.trim().is_empty() {
        return Err("Informe o IP da câmera (em Editar) para usar o rastreamento".to_string());
    }
    let trace: String = (0..16).map(|_| format!("{:x}", OsRng.next_u32() % 16)).collect();
    let config = tuya_ptz::call(
        app,
        &cam,
        "/api/jarvis/config",
        json!({ "devId": cam.device_id, "clientTraceId": trace }),
    )?;
    let local_key = config["result"]["localKey"].as_str().unwrap_or_default();
    if local_key.len() != 16 {
        return Err("A Tuya não devolveu a chave local desta câmera".to_string());
    }
    let mut conn = Conn::open(cam.ip.trim(), local_key.as_bytes())?;
    conn.negotiate()?;
    Ok(conn)
}

fn tracking_value(dps: &Map<String, Value>) -> Result<bool, String> {
    match dps.get(TRACKING_DP) {
        Some(Value::Bool(on)) => Ok(*on),
        Some(other) => Err(format!("O DP {TRACKING_DP} tem um valor inesperado: {other}")),
        None => {
            let mut ids: Vec<&String> = dps.keys().collect();
            ids.sort_by_key(|k| k.parse::<u32>().unwrap_or(u32::MAX));
            let ids: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
            Err(format!(
                "A câmera não tem o DP {TRACKING_DP} de rastreamento. DPs disponíveis: {}",
                ids.join(", ")
            ))
        }
    }
}

/// Lê se o rastreamento de movimento está ligado.
pub fn get_tracking(app: &AppHandle, camera_id: &str) -> Result<bool, String> {
    let mut conn = connect(app, camera_id)?;
    conn.send(DP_QUERY_NEW, b"{}")?;
    tracking_value(&conn.wait_dps()?)
}

/// Liga ou desliga o rastreamento de movimento e devolve o estado que a câmera confirmou.
pub fn set_tracking(app: &AppHandle, camera_id: &str, on: bool) -> Result<bool, String> {
    let mut conn = connect(app, camera_id)?;
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body = json!({ "protocol": 5, "t": t, "data": { "dps": { TRACKING_DP: on } } });
    // Comandos de controle levam o cabeçalho de versão: "3.4" seguido de 12 bytes zerados.
    let mut payload = b"3.4".to_vec();
    payload.extend([0u8; 12]);
    payload.extend(body.to_string().as_bytes());
    conn.send(CONTROL_NEW, &payload)?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let dps = conn.wait_dps()?;
        if dps.contains_key(TRACKING_DP) {
            return tracking_value(&dps);
        }
    }
    Err("A câmera não confirmou a mudança".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn decrypts_real_camera_broadcast() {
        // Anúncio UDP capturado da câmera 192.168.0.6, cifrado com a chave pública de descoberta da Tuya.
        let frame = unhex("000055aa0000000000000023000000cc0000000023d01e5fef56833314cef60fb086ed33df68b9d2cad124f740318902544ba5991a4d9ac8bef04773cc3a8022d7d6fa5963ee274834a1153b778dc4d5a61a8468e5009ca4c2b6940ff0c50c5e4948159f02da2d7bf10b10284b5270bc2b6fd231ffb70b01464ebee37d5cd9e5c640ebf114543e11e948be7fa6843f0bba2e89757546651ee574fa8c1cea436a3b154904eb58637ec14f725d8af3846dfba9ff0285f3553afd0a2a4d5428447ff0b8506d452f59b88a946f133dcef437156ff9a5a92434cb0000aa55");
        let len = u32::from_be_bytes(frame[12..16].try_into().unwrap()) as usize;
        let enc = &frame[16 + 4..16 + len - 8];
        let key = unhex(&format!("{:x}", md5::compute(b"yGAdlopoPVldABfn")));
        let text = String::from_utf8(decrypt(&key, enc).unwrap()).unwrap();
        assert!(text.starts_with('{') && text.ends_with('}'), "{text}");
        assert!(text.contains("\"version\":\"3.4\""), "{text}");
    }

    #[test]
    fn hmac_matches_rfc4231() {
        let mac = hmac(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn roundtrip_and_payloads() {
        let key = b"0123456789abcdef";
        assert_eq!(decrypt(key, &encrypt(key, b"{}", true)).unwrap(), b"{}");
        assert_eq!(encrypt(key, &[7u8; 16], false).len(), 16);

        let body = json!({ "data": { "dps": { TRACKING_DP: true } } });
        assert_eq!(body.to_string(), r#"{"data":{"dps":{"161":true}}}"#);

        let mut with_header = b"3.4".to_vec();
        with_header.extend([0u8; 12]);
        with_header.extend(br#"{"protocol":4,"t":1,"data":{"dps":{"161":false,"119":"0"}}}"#);
        assert_eq!(parse_dps(&with_header).unwrap()["161"], Value::Bool(false));
        assert_eq!(parse_dps(br#"{"dps":{"161":true},"t":1}"#).unwrap()["161"], Value::Bool(true));
    }
}
