mod detect;
mod go2rtc;
mod network;
mod store;
mod tuya;
mod tuya_local;
mod tuya_ptz;

use store::Camera;
use tauri::{AppHandle, RunEvent};

#[tauri::command]
fn list_cameras(app: AppHandle) -> Result<Vec<Camera>, String> {
    store::load(&app)
}

/// Salva a câmera, regrava o config do go2rtc e reinicia o motor de vídeo.
/// O reinício espera o motor e pode levar alguns segundos, então roda fora da thread principal.
#[tauri::command]
async fn save_camera(app: AppHandle, camera: Camera) -> Result<Vec<Camera>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Camera>, String> {
        let mut cams = store::load(&app)?;
        let mut cam = camera;
        cam.id = store::new_id();
        cam.ip = cam.ip.trim().to_string();
        cams.push(cam);
        store::save(&app, &cams)?;
        go2rtc::restart(&app)?;
        Ok(cams)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Atualiza os dados de uma câmera existente (mantém o id), regrava o config do go2rtc e reinicia o motor.
#[tauri::command]
async fn update_camera(app: AppHandle, camera: Camera) -> Result<Vec<Camera>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Camera>, String> {
        let mut cams = store::load(&app)?;
        let cam = cams
            .iter_mut()
            .find(|c| c.id == camera.id)
            .ok_or("Câmera não encontrada")?;
        *cam = Camera {
            ip: camera.ip.trim().to_string(),
            ..camera
        };
        store::save(&app, &cams)?;
        go2rtc::restart(&app)?;
        Ok(cams)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn delete_camera(app: AppHandle, id: String) -> Result<Vec<Camera>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Camera>, String> {
        let mut cams = store::load(&app)?;
        cams.retain(|c| c.id != id);
        store::save(&app, &cams)?;
        go2rtc::restart(&app)?;
        Ok(cams)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Consulta a API do go2rtc para saber se o motor está respondendo.
#[tauri::command]
fn go2rtc_status() -> Result<serde_json::Value, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?
        .get("http://127.0.0.1:1984/api/streams")
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .map_err(|e| e.to_string())
}

/// Move a câmera Tuya (UP, DOWN, LEFT ou RIGHT) enquanto o botão estiver pressionado.
#[tauri::command]
async fn ptz_move(app: AppHandle, id: String, direction: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || tuya_ptz::move_camera(&app, &id, &direction))
        .await
        .map_err(|e| e.to_string())?
}

/// Para o movimento da câmera Tuya ao soltar o botão.
#[tauri::command]
async fn ptz_stop(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || tuya_ptz::stop_camera(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Lê se o rastreamento de movimento da câmera Tuya está ligado (protocolo local).
#[tauri::command]
async fn tracking_get(app: AppHandle, id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || tuya_local::get_tracking(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Liga ou desliga o rastreamento de movimento e devolve o estado confirmado pela câmera.
#[tauri::command]
async fn tracking_set(app: AppHandle, id: String, on: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || tuya_local::set_tracking(&app, &id, on))
        .await
        .map_err(|e| e.to_string())?
}

/// Reinicia o go2rtc quando ele para de responder.
#[tauri::command]
async fn restart_go2rtc(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || go2rtc::restart(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// Informa se o go2rtc já está instalado neste computador.
#[tauri::command]
fn go2rtc_installed(app: AppHandle) -> bool {
    go2rtc::installed(&app)
}

/// Baixa o go2rtc da release oficial para o sistema atual e inicia o motor.
/// O download é bloqueante, então roda em thread própria fora do runtime async.
#[tauri::command]
async fn install_go2rtc(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || go2rtc::install(&app))
        .await
        .map_err(|e| e.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .manage(go2rtc::Engine::default())
        .manage(tuya_ptz::Sessions::default())
        .setup(|app| {
            if let Err(e) = go2rtc::restart(app.handle()) {
                eprintln!("go2rtc: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_cameras,
            save_camera,
            update_camera,
            delete_camera,
            go2rtc_status,
            go2rtc_installed,
            install_go2rtc,
            restart_go2rtc,
            ptz_move,
            ptz_stop,
            tracking_get,
            tracking_set,
            detect::detect_camera,
            tuya::tuya_login,
            network::network_devices,
            network::scan_network,
            network::forget_device,
            network::rename_device
        ])
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o aplicativo")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                go2rtc::stop(app);
            }
        });
}
