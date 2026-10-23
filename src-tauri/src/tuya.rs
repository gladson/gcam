use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Uma câmera encontrada na conta Tuya.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TuyaDevice {
    pub device_id: String,
    pub name: String,
}

/// Resposta do endpoint api/tuya do go2rtc.
#[derive(Deserialize)]
struct Sources {
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(default)]
    name: String,
    #[serde(default)]
    url: String,
}

/// Lê um parâmetro da query string, ex.: `device_id` em `tuya://host?device_id=x&email=...`.
fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    query
        .split('&')
        .find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Faz login na conta Tuya pelo go2rtc (mesmo endpoint da tela de login dele)
/// e devolve as câmeras da conta. O trabalho bloqueante roda em thread própria, fora do runtime async.
#[tauri::command]
pub async fn tuya_login(region: String, email: String, password: String) -> Result<Vec<TuyaDevice>, String> {
    tauri::async_runtime::spawn_blocking(move || login(&region, &email, &password))
        .await
        .map_err(|e| e.to_string())?
}

fn login(region: &str, email: &str, password: &str) -> Result<Vec<TuyaDevice>, String> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get("http://127.0.0.1:1984/api/tuya")
        .query(&[
            ("region", region),
            ("email", email),
            ("password", password),
        ])
        .send()
        .map_err(|e| format!("O motor de vídeo não respondeu: {e}"))?;

    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        let text = text.trim();
        return Err(if text.is_empty() {
            "Falha no login Tuya".to_string()
        } else {
            text.to_string()
        });
    }

    let data: Sources = resp
        .json()
        .map_err(|e| format!("Resposta inesperada do go2rtc: {e}"))?;

    let devices: Vec<TuyaDevice> = data
        .sources
        .into_iter()
        .filter_map(|s| {
            let device_id = query_param(&s.url, "device_id")?.to_string();
            Some(TuyaDevice {
                device_id,
                name: s.name,
            })
        })
        .collect();

    if devices.is_empty() {
        return Err("Nenhuma câmera encontrada nesta conta Tuya".to_string());
    }
    Ok(devices)
}
