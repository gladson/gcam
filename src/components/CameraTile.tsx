import { useEffect, useRef, useState } from "react";
import { Button, Switch } from "@fluentui/react-components";
import { ArrowDown24Filled, ArrowLeft24Filled, ArrowRight24Filled, ArrowUp24Filled, Delete24Regular, Edit24Regular } from "@fluentui/react-icons";
import { api } from "../api";
import type { Camera, PtzDirection } from "../types";
import { brandLabel } from "../presets";

interface Props {
  camera: Camera;
  onEdit: () => void;
  onDelete: () => void;
}

const ARROWS: { dir: PtzDirection; icon: JSX.Element; pos: string }[] = [
  { dir: "UP", icon: <ArrowUp24Filled />, pos: "col-start-2 row-start-1" },
  { dir: "LEFT", icon: <ArrowLeft24Filled />, pos: "col-start-1 row-start-2" },
  { dir: "RIGHT", icon: <ArrowRight24Filled />, pos: "col-start-3 row-start-2" },
  { dir: "DOWN", icon: <ArrowDown24Filled />, pos: "col-start-2 row-start-3" },
];

const ENGINE_ORIGIN = "http://127.0.0.1:1984";
// Só WebRTC: é o único modo em que o WebView do macOS mostra o H.265 das câmeras Tuya
// (pelo MSE os dados chegam mas a imagem fica preta). Câmeras H.264 também tocam por ele.
// Os congelamentos do WebRTC são tratados pelo vigia de quadros do player.html.
export const PLAYER_MODE = "webrtc";

/** Estado que o player (player.html, servido pelo go2rtc) manda para o app. */
interface PlayerMessage {
  type: "cm-player";
  src: string;
  status: "playing" | "stalled" | "reconnecting";
  failures: number;
}

export default function CameraTile({ camera, onEdit, onDelete }: Props) {
  // Player do app servido pelo próprio go2rtc (mesma origem do WebSocket). Ele mesmo percebe a imagem
  // congelada e reconecta; aqui só mostramos o aviso e recriamos o stream quando falha de novo.
  const src = `${ENGINE_ORIGIN}/player.html?src=${encodeURIComponent(camera.id)}&mode=${PLAYER_MODE}`;
  const frame = useRef<HTMLIFrameElement>(null);
  const [ptzError, setPtzError] = useState("");
  const isTuya = camera.brand === "tuya";
  // null enquanto o estado do rastreamento ainda não foi lido da câmera.
  const [tracking, setTracking] = useState<boolean | null>(null);
  const [trackingBusy, setTrackingBusy] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [reconnecting, setReconnecting] = useState(false);

  // O player reconecta sozinho; ao sair o último player, o go2rtc encerra a sessão com a câmera
  // e a reconexão abre uma nova. Aqui só acompanha o estado para mostrar o aviso.
  useEffect(() => {
    function onMessage(ev: MessageEvent) {
      if (ev.origin !== ENGINE_ORIGIN || ev.source !== frame.current?.contentWindow) return;
      const msg = ev.data as PlayerMessage;
      if (msg?.type !== "cm-player" || msg.src !== camera.id) return;
      setReconnecting(msg.status !== "playing");
    }
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [camera.id]);

  useEffect(() => {
    if (!isTuya) return;
    let alive = true;
    api
      .trackingGet(camera.id)
      .then((on) => alive && setTracking(on))
      .catch((e) => alive && setPtzError(String(e)));
    return () => {
      alive = false;
    };
  }, [camera.id, isTuya]);

  async function toggleTracking() {
    if (tracking === null) return;
    setTrackingBusy(true);
    setPtzError("");
    try {
      setTracking(await api.trackingSet(camera.id, !tracking));
    } catch (e) {
      setPtzError(String(e));
    } finally {
      setTrackingBusy(false);
    }
  }

  // Enquanto o botão está pressionado a câmera se move; ao soltar, para.
  function press(dir: PtzDirection) {
    setPtzError("");
    api.ptzMove(camera.id, dir).catch((e) => setPtzError(String(e)));
  }

  function release() {
    api.ptzStop(camera.id).catch((e) => setPtzError(String(e)));
  }

  return (
    <div className="overflow-hidden rounded-2xl border border-white/10 bg-slate-900 shadow-xl">
      <div className="relative aspect-video bg-black">
        <iframe ref={frame} title={camera.name} src={src} className="h-full w-full border-0" allow="autoplay" />
        {reconnecting && (
          <span className="absolute bottom-2 left-2 rounded-md bg-black/70 px-2 py-0.5 text-xs text-amber-300">
            Vídeo travou, reconectando…
          </span>
        )}
      </div>
      <div className="flex items-center justify-between gap-3 p-3">
        <div className="min-w-0">
          <p className="truncate font-medium">{camera.name}</p>
          <p className="truncate text-xs text-slate-400">
            {brandLabel(camera.brand)} · {camera.ip}
          </p>
          {isTuya && (
            <Switch
              className="mt-1"
              checked={tracking === true}
              disabled={tracking === null || trackingBusy}
              onChange={toggleTracking}
              label={
                tracking === null
                  ? "Rastreamento: lendo…"
                  : trackingBusy
                    ? "Rastreamento: aguarde…"
                    : "Rastreamento de movimento"
              }
            />
          )}
          {ptzError && <p className="mt-1 text-xs text-rose-300">{ptzError}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {isTuya && (
            <div className="grid grid-cols-3 grid-rows-3 gap-0.5">
              {ARROWS.map((a) => (
                <Button
                  key={a.dir}
                  appearance="subtle"
                  size="small"
                  title={`Mover ${a.dir.toLowerCase()}`}
                  aria-label={`Mover ${a.dir.toLowerCase()}`}
                  icon={a.icon}
                  onPointerDown={() => press(a.dir)}
                  onPointerUp={release}
                  onPointerLeave={release}
                  className={a.pos}
                />
              ))}
            </div>
          )}
          <Button size="small" appearance="subtle" icon={<Edit24Regular />} onClick={onEdit}>
            Editar
          </Button>
          {confirmingDelete ? (
            <Button
              size="small"
              appearance="primary"
              onClick={onDelete}
              onBlur={() => setConfirmingDelete(false)}
            >
              Confirmar?
            </Button>
          ) : (
            <Button
              size="small"
              appearance="subtle"
              icon={<Delete24Regular />}
              onClick={() => setConfirmingDelete(true)}
            >
              Remover
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
