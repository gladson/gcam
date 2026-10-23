use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

/// Uma câmera cadastrada. Os campos Tuya ficam vazios para câmeras RTSP.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub brand: String,
    pub ip: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub tuya_host: String,
}

/// Pasta de dados do app (criada se não existir).
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn load(app: &AppHandle) -> Result<Vec<Camera>, String> {
    let file = data_dir(app)?.join("cameras.json");
    if !file.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("cameras.json inválido: {e}"))
}

pub fn save(app: &AppHandle, cams: &[Camera]) -> Result<(), String> {
    let file = data_dir(app)?.join("cameras.json");
    let text = serde_json::to_string_pretty(cams).map_err(|e| e.to_string())?;
    fs::write(file, text).map_err(|e| e.to_string())
}

/// Gera um id simples e seguro para usar como nome de stream no go2rtc.
pub fn new_id() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("cam{ms}")
}
