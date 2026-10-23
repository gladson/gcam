use crate::store::{self, Camera};
use std::{
    fs::{self, File},
    io::{self, Cursor},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const API_PORT: u16 = 1984;
const RTSP_PORT: u16 = 8554;
use tauri::{AppHandle, Manager};

/// Guarda o processo do go2rtc para reiniciar/encerrar depois.
#[derive(Default)]
pub struct Engine(pub Mutex<Option<Child>>);

/// Monta a URL de origem que o go2rtc entende para cada câmera.
pub fn source_for(c: &Camera) -> String {
    if c.brand == "tuya" {
        return format!(
            "tuya://{}?device_id={}&email={}&password={}",
            c.tuya_host.trim(),
            urlencoding::encode(&c.device_id),
            urlencoding::encode(&c.username),
            urlencoding::encode(&c.password),
        );
    }
    let auth = if c.username.is_empty() {
        String::new()
    } else {
        format!(
            "{}:{}@",
            urlencoding::encode(&c.username),
            urlencoding::encode(&c.password)
        )
    };
    format!("rtsp://{auth}{}:554{}", c.ip, c.path)
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "go2rtc.exe"
    } else {
        "go2rtc"
    }
}

/// Procura o binário em app_data/bin, depois na pasta de recursos e por fim no PATH.
fn find_binary(app: &AppHandle) -> Option<PathBuf> {
    let name = binary_name();
    if let Ok(dir) = app.path().app_data_dir() {
        let p = dir.join("bin").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("bin").join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    on_path(name)
}

/// Procura o executável no PATH do sistema.
fn on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Indica se o go2rtc já está disponível (pasta do app, recursos ou PATH).
pub fn installed(app: &AppHandle) -> bool {
    find_binary(app).is_some()
}

/// Versão do go2rtc baixada pelo app (última release oficial conferida).
const GO2RTC_VERSION: &str = "v1.9.14";

/// Arquivo da release oficial para este sistema e processador.
/// Windows e macOS vêm em .zip; no Linux o download já é o binário.
fn release_asset() -> Result<&'static str, String> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "go2rtc_win64.zip",
        ("windows", "aarch64") => "go2rtc_win_arm64.zip",
        ("macos", "x86_64") => "go2rtc_mac_amd64.zip",
        ("macos", "aarch64") => "go2rtc_mac_arm64.zip",
        ("linux", "x86_64") => "go2rtc_linux_amd64",
        ("linux", "aarch64") => "go2rtc_linux_arm64",
        ("linux", "arm") => "go2rtc_linux_arm",
        (os, arch) => return Err(format!("Download automático não disponível para {os} ({arch})")),
    })
}

/// Baixa o go2rtc da release oficial, grava em app_data/bin e inicia o motor.
/// Bloqueante: chame a partir de um comando `async` do Tauri.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let asset = release_asset()?;
    let url = format!("https://github.com/AlexxIT/go2rtc/releases/download/{GO2RTC_VERSION}/{asset}");
    let data = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.bytes())
        .map_err(|e| format!("Falha ao baixar o go2rtc: {e}"))?;

    let bin_dir = store::data_dir(app)?.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let tmp = bin_dir.join(format!("{}.part", binary_name()));
    let dest = bin_dir.join(binary_name());

    if asset.ends_with(".zip") {
        extract_binary(&data, &tmp)?;
    } else {
        fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    restart(app)
}

/// Extrai o executável do go2rtc de dentro do .zip baixado.
fn extract_binary(zip_data: &[u8], out: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip_data))
        .map_err(|e| format!("Arquivo do go2rtc inválido: {e}"))?;
    let mut entry = archive
        .by_name(binary_name())
        .map_err(|e| format!("{} não encontrado no pacote: {e}", binary_name()))?;
    let mut file = File::create(out).map_err(|e| e.to_string())?;
    io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
    Ok(())
}

/// Player próprio servido pelo go2rtc (mesma origem do vídeo, então o go2rtc aceita a conexão
/// sem liberar a API para outros sites). Ele percebe quando a imagem congela e reconecta sozinho.
const PLAYER_FILES: [(&str, &str); 3] = [
    ("player.html", include_str!("../player/player.html")),
    ("video-rtc.js", include_str!("../player/video-rtc.js")),
    ("video-stream.js", include_str!("../player/video-stream.js")),
];

/// Grava os arquivos do player na pasta www dos dados do app e devolve o caminho dela.
fn write_player(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = store::data_dir(app)?.join("www");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, content) in PLAYER_FILES {
        fs::write(dir.join(name), content).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Grava o go2rtc.yaml com todas as câmeras cadastradas.
fn write_config(app: &AppHandle, cams: &[Camera]) -> Result<PathBuf, String> {
    // Aspas simples no YAML: o caminho do Windows tem barras invertidas, que nas duplas viram escape.
    let www = write_player(app)?.to_string_lossy().replace('\'', "''");
    let mut yaml = format!(
        "api:\n  listen: \"127.0.0.1:1984\"\n  static_dir: '{www}'\n\
         rtsp:\n  listen: \"127.0.0.1:8554\"\n\
         webrtc:\n  listen: \":8555\"\n",
    );
    if cams.is_empty() {
        yaml.push_str("streams: {}\n");
    } else {
        yaml.push_str("streams:\n");
        for c in cams {
            yaml.push_str(&format!("  {}: \"{}\"\n", c.id, source_for(c)));
        }
    }
    let path = store::data_dir(app)?.join("go2rtc.yaml");
    fs::write(&path, yaml).map_err(|e| e.to_string())?;
    Ok(path)
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// Espera até `cond` ficar verdadeira, por no máximo `secs` segundos.
fn wait_until(cond: impl Fn() -> bool, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    cond()
}

/// Encerra um go2rtc de uma sessão anterior do app, registrado em go2rtc.pid.
fn kill_stale(pid: u32) {
    #[cfg(unix)]
    {
        // Confere o nome antes: o PID pode ter sido reaproveitado por outro programa.
        let is_go2rtc = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("go2rtc"))
            .unwrap_or(false);
        if is_go2rtc {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status();
    }
}

/// Para o go2rtc atual (se houver), regrava o config e inicia de novo.
pub fn restart(app: &AppHandle) -> Result<(), String> {
    stop(app);
    if !wait_until(|| !port_open(API_PORT) && !port_open(RTSP_PORT), 5) {
        return Err(format!(
            "As portas {API_PORT} e {RTSP_PORT} estão em uso por outro programa, por exemplo outro go2rtc aberto. Feche-o e reinicie o app."
        ));
    }
    let cams = store::load(app)?;
    let cfg = write_config(app, &cams)?;
    let dir = store::data_dir(app)?;

    let log = File::create(dir.join("go2rtc.log")).map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let bin = find_binary(app).unwrap_or_else(|| PathBuf::from(binary_name()));

    let mut child = Command::new(&bin)
        .arg("-config")
        .arg(&cfg)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .map_err(|e| format!("Não consegui iniciar o go2rtc ({}). Detalhe: {e}", bin.display()))?;

    // Espera a API responder. Se o processo cair antes, avisa em vez de deixar o vídeo mudo.
    let started = Instant::now();
    while !port_open(API_PORT) && started.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "O go2rtc encerrou ao iniciar ({status}). Veja o arquivo go2rtc.log na pasta de dados do app."
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }

    fs::write(dir.join("go2rtc.pid"), child.id().to_string()).map_err(|e| e.to_string())?;
    let state = app.state::<Engine>();
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = Some(child);
    Ok(())
}

pub fn stop(app: &AppHandle) {
    let state = app.state::<Engine>();
    let child = state.0.lock().ok().and_then(|mut guard| guard.take());
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    // Processo de uma sessão anterior que não foi encerrada (ex.: o app fechou à força).
    if let Ok(dir) = store::data_dir(app) {
        let pid_file = dir.join("go2rtc.pid");
        if let Some(pid) = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            kill_stale(pid);
        }
        let _ = fs::remove_file(&pid_file);
    }
}
