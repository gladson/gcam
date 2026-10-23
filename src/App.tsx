import { useCallback, useEffect, useRef, useState } from "react";
import { Badge, Button, MessageBar, MessageBarActions, MessageBarBody, MessageBarTitle } from "@fluentui/react-components";
import { Add24Regular, Camera28Regular, Wifi124Regular } from "@fluentui/react-icons";
import { api } from "./api";
import type { Camera } from "./types";
import AddCameraModal from "./components/AddCameraModal";
import CameraTile from "./components/CameraTile";
import NetworkPage from "./components/NetworkPage";

export default function App() {
  const [cameras, setCameras] = useState<Camera[]>([]);
  const [engineOk, setEngineOk] = useState(true);
  const [view, setView] = useState<"cameras" | "network">("cameras");
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<Camera | null>(null);
  // IP escolhido na lista de dispositivos para virar câmera.
  const [prefillIp, setPrefillIp] = useState("");
  // Cada troca de câmeras reinicia o go2rtc; mudar a chave recria os players com a conexão nova.
  const [engineGen, setEngineGen] = useState(0);
  const [error, setError] = useState("");
  // null enquanto verifica; false mostra o aviso de instalação.
  const [installed, setInstalled] = useState<boolean | null>(null);
  const installedRef = useRef(installed);
  installedRef.current = installed;
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState("");

  const refresh = useCallback(async () => {
    try {
      setCameras(await api.list());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Verifica se o go2rtc (motor de vídeo) está respondendo; se cair, reinicia sozinho.
  useEffect(() => {
    let alive = true;
    let failures = 0;
    let restarting = false;
    const tick = async () => {
      try {
        await api.streams();
        failures = 0;
        if (alive) setEngineOk(true);
      } catch {
        if (alive) setEngineOk(false);
        // Duas falhas seguidas (~10s) com o go2rtc instalado: reinicia e recria os players.
        if (++failures >= 2 && installedRef.current && !restarting) {
          restarting = true;
          try {
            await api.restartEngine();
            failures = 0;
            if (alive) {
              setEngineOk(true);
              setEngineGen((g) => g + 1);
            }
          } catch {
            // Tenta de novo nos próximos ciclos; o aviso na tela continua.
          } finally {
            restarting = false;
          }
        }
      }
    };
    tick();
    const timer = setInterval(tick, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  // Ao abrir o app, verifica se o go2rtc já está no computador.
  useEffect(() => {
    api
      .engineInstalled()
      .then(setInstalled)
      .catch(() => setInstalled(false));
  }, []);

  async function installEngine() {
    setInstalling(true);
    setInstallError("");
    try {
      await api.installEngine();
      setInstalled(true);
    } catch (e) {
      setInstallError(String(e));
    } finally {
      setInstalling(false);
    }
  }

  // A confirmação fica no cartão: window.confirm não funciona no webview do Tauri.
  async function remove(id: string) {
    try {
      setCameras(await api.remove(id));
      setEngineGen((g) => g + 1);
    } catch (e) {
      setError(String(e));
    }
  }

  function closeModal() {
    setShowAdd(false);
    setEditing(null);
    setPrefillIp("");
  }

  const modal = (showAdd || editing) && (
    <AddCameraModal
      camera={editing ?? undefined}
      initialIp={prefillIp || undefined}
      onClose={closeModal}
      onSaved={(list) => {
        setCameras(list);
        setEngineGen((g) => g + 1);
        closeModal();
      }}
    />
  );

  if (view === "network") {
    return (
      <div className="flex h-full flex-col text-slate-100">
        <NetworkPage
          cameras={cameras}
          onBack={() => setView("cameras")}
          onAddCamera={(ip) => {
            setPrefillIp(ip);
            setView("cameras");
            setShowAdd(true);
          }}
        />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col text-slate-100">
      <header className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-6 py-4">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Câmeras</h1>
          <p className="text-xs text-slate-400">
            {cameras.length} cadastrada{cameras.length === 1 ? "" : "s"}
          </p>
        </div>
        <div className="flex items-center gap-4">
          <Badge appearance="tint" color={engineOk ? "success" : "danger"}>
            {engineOk ? "Motor de vídeo ativo" : "Motor de vídeo parado"}
          </Badge>
          <Button appearance="secondary" icon={<Wifi124Regular />} onClick={() => setView("network")}>
            Dispositivos na rede
          </Button>
          <Button appearance="primary" icon={<Add24Regular />} onClick={() => setShowAdd(true)}>
            Adicionar câmera
          </Button>
        </div>
      </header>

      {error && (
        <MessageBar intent="error" className="mx-6 mt-4">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}

      {installed === false && (
        <MessageBar intent="warning" layout="multiline" className="mx-6 mt-4">
          <MessageBarBody>
            <MessageBarTitle>O motor de vídeo (go2rtc) não está instalado neste computador.</MessageBarTitle>
            Ele é necessário para mostrar o vídeo. O download vem da release oficial do go2rtc e é instalado
            automaticamente.
            {installError && <p className="mt-1 text-xs text-rose-300">{installError}</p>}
          </MessageBarBody>
          <MessageBarActions>
            <Button appearance="primary" onClick={installEngine} disabled={installing}>
              {installing ? "Baixando…" : "Baixar go2rtc"}
            </Button>
          </MessageBarActions>
        </MessageBar>
      )}

      {installed !== false && !engineOk && (
        <MessageBar intent="warning" className="mx-6 mt-4">
          <MessageBarBody>
            O go2rtc parou de responder. O app está tentando reiniciá-lo. Se o aviso continuar, veja o arquivo{" "}
            <code>go2rtc.log</code> na pasta de dados do app.
          </MessageBarBody>
        </MessageBar>
      )}

      <main className="flex-1 overflow-y-auto">
        {cameras.length === 0 ? (
          <div className="flex h-full min-h-[300px] flex-col items-center justify-center gap-3 text-center">
            <Camera28Regular className="text-slate-400" />
            <p className="text-lg font-medium">Nenhuma câmera cadastrada</p>
            <p className="max-w-sm text-sm text-slate-400">
              Clique em <strong>Adicionar câmera</strong>, informe o IP e o app identifica a marca sozinho.
            </p>
          </div>
        ) : (
          <div className="grid gap-5 p-6 sm:grid-cols-2 xl:grid-cols-3">
            {cameras.map((c) => (
              <CameraTile
                key={`${c.id}-${engineGen}`}
                camera={c}
                onEdit={() => setEditing(c)}
                onDelete={() => remove(c.id)}
              />
            ))}
          </div>
        )}
      </main>

      {modal}
    </div>
  );
}
