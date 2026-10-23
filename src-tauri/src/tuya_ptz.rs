use crate::store::{self, Camera};
use rsa::{pkcs8::DecodePublicKey, Pkcs1v15Encrypt, RsaPublicKey};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager};

/// Sessões de login na nuvem Tuya, uma por conta (região + e-mail), reaproveitadas entre os comandos de PTZ.
#[derive(Default)]
pub struct Sessions(Mutex<HashMap<String, reqwest::blocking::Client>>);

/// Código de continente que a API de login espera, conforme a região escolhida.
fn continent(host: &str) -> Result<&'static str, String> {
    Ok(match host {
        "protect-eu.ismartlife.me" | "protect-we.ismartlife.me" => "EU",
        "protect-us.ismartlife.me" | "protect-ue.ismartlife.me" => "AZ",
        "protect.ismartlife.me" => "AY",
        "protect-in.ismartlife.me" => "IN",
        _ => return Err(format!("Região Tuya desconhecida: {host}")),
    })
}

/// Mesma criptografia do login do go2rtc: MD5 da senha, depois RSA PKCS#1 v1.5 com a chave da região.
fn encrypt_password(password: &str, pb_key: &str) -> Result<String, String> {
    let hashed = format!("{:x}", md5::compute(password.as_bytes()));
    // A chave vem em uma linha só; o PEM exige linhas de 64 caracteres.
    let wrapped = pb_key
        .as_bytes()
        .chunks(64)
        .filter_map(|c| std::str::from_utf8(c).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let pem = format!("-----BEGIN PUBLIC KEY-----\n{wrapped}\n-----END PUBLIC KEY-----");
    let key = RsaPublicKey::from_public_key_pem(&pem).map_err(|e| format!("Chave da Tuya inválida: {e}"))?;
    let mut rng = rsa::rand_core::OsRng;
    let encrypted = key
        .encrypt(&mut rng, Pkcs1v15Encrypt, hashed.as_bytes())
        .map_err(|e| format!("Falha ao criptografar a senha: {e}"))?;
    Ok(encrypted.iter().map(|b| format!("{b:02x}")).collect())
}

/// Chamada JSON à nuvem Tuya. Devolve o corpo, ou o erro que a Tuya informou.
fn post(client: &reqwest::blocking::Client, host: &str, path: &str, body: Value) -> Result<Value, String> {
    let resp = client
        .post(format!("https://{host}{path}"))
        .header("Origin", format!("https://{host}"))
        .json(&body)
        .send()
        .map_err(|e| format!("Falha ao falar com a Tuya: {e}"))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| format!("Resposta inesperada da Tuya ({status})"))?;
    if value["success"].as_bool() != Some(true) {
        let msg = value["errorMsg"].as_str().unwrap_or("Falha na Tuya");
        return Err(msg.to_string());
    }
    Ok(value)
}

/// Faz o login com e-mail e senha e devolve um cliente com os cookies da sessão.
fn login(host: &str, email: &str, password: &str) -> Result<reqwest::blocking::Client, String> {
    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let cc = continent(host)?;

    let token = post(
        &client,
        host,
        "/api/login/token",
        json!({ "countryCode": cc, "username": email, "isUid": false }),
    )?;
    let passwd = encrypt_password(password, token["result"]["pbKey"].as_str().unwrap_or_default())?;

    post(
        &client,
        host,
        "/api/private/email/login",
        json!({
            "countryCode": cc,
            "email": email,
            "passwd": passwd,
            "token": token["result"]["token"],
            "ifencrypt": 1,
            "options": "{\"group\":1}",
        }),
    )?;
    Ok(client)
}

/// Carrega uma câmera Tuya cadastrada.
pub fn tuya_camera(app: &AppHandle, camera_id: &str) -> Result<Camera, String> {
    let cam = store::load(app)?
        .into_iter()
        .find(|c| c.id == camera_id)
        .ok_or("Câmera não encontrada")?;
    if cam.brand != "tuya" {
        return Err("Função disponível só para câmeras Tuya".to_string());
    }
    Ok(cam)
}

/// Chamada autenticada à nuvem Tuya pela conta da câmera, reaproveitando a sessão.
/// Se a sessão estiver velha ou expirada, refaz o login uma vez e tenta de novo.
pub fn call(app: &AppHandle, cam: &Camera, path: &str, body: Value) -> Result<Value, String> {
    let key = format!("{}|{}", cam.tuya_host, cam.username);
    let sessions = app.state::<Sessions>();
    let mut map = sessions.0.lock().map_err(|e| e.to_string())?;
    if let Some(client) = map.get(&key) {
        if let Ok(value) = post(client, &cam.tuya_host, path, body.clone()) {
            return Ok(value);
        }
    }
    let client = login(&cam.tuya_host, &cam.username, &cam.password)?;
    let result = post(&client, &cam.tuya_host, path, body);
    map.insert(key, client);
    result
}

/// Envia um comando de PTZ: `Some(direção)` começa a mover, `None` para o movimento.
fn send(app: &AppHandle, camera_id: &str, direction: Option<&str>) -> Result<(), String> {
    let cam = tuya_camera(app, camera_id)?;
    let (path, body) = match direction {
        Some(d) => ("/api/jarvis/ptz/start", json!({ "devId": cam.device_id, "direction": d })),
        None => ("/api/jarvis/ptz/end", json!({ "devId": cam.device_id })),
    };
    call(app, &cam, path, body).map(|_| ())
}

pub fn move_camera(app: &AppHandle, camera_id: &str, direction: &str) -> Result<(), String> {
    if !matches!(direction, "UP" | "DOWN" | "LEFT" | "RIGHT") {
        return Err(format!("Direção inválida: {direction}"));
    }
    send(app, camera_id, Some(direction))
}

pub fn stop_camera(app: &AppHandle, camera_id: &str) -> Result<(), String> {
    send(app, camera_id, None)
}
