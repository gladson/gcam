import { useEffect, useMemo, useRef, useState } from "react";
import { Badge, Button, Input, MessageBar, MessageBarBody, SearchBox, Select } from "@fluentui/react-components";
import {
  ArrowLeft24Regular,
  ArrowSync24Regular,
  Camera24Regular,
  ChevronRight24Regular,
  Delete24Regular,
  Desktop24Regular,
  Lightbulb24Regular,
  Phone24Regular,
  Print24Regular,
  QuestionCircle24Regular,
  Router24Regular,
  Tv24Regular,
} from "@fluentui/react-icons";
import { api } from "../api";
import type { Camera, DeviceKind, NetDevice } from "../types";

interface Props {
  cameras: Camera[];
  onBack: () => void;
  onAddCamera: (ip: string) => void;
}

type SortBy = "ip" | "name" | "vendor";

const ICONS: Record<DeviceKind, JSX.Element> = {
  router: <Router24Regular />,
  camera: <Camera24Regular />,
  tv: <Tv24Regular />,
  phone: <Phone24Regular />,
  computer: <Desktop24Regular />,
  printer: <Print24Regular />,
  iot: <Lightbulb24Regular />,
  unknown: <QuestionCircle24Regular />,
};

const PORT_LABELS: Record<number, string> = {
  21: "FTP",
  22: "SSH",
  23: "Telnet",
  53: "DNS",
  80: "HTTP",
  139: "NetBIOS",
  443: "HTTPS",
  445: "SMB",
  515: "LPD",
  554: "RTSP",
  631: "IPP",
  1883: "MQTT",
  3389: "RDP",
  5000: "UPnP",
  5900: "VNC",
  6668: "Tuya",
  7000: "AirPlay",
  8000: "HTTP alt/Hikvision",
  8001: "Samsung TV",
  8002: "Samsung TV",
  8008: "Chromecast",
  8009: "Chromecast",
  8080: "HTTP alt",
  8443: "HTTPS alt",
  8554: "RTSP alt",
  9100: "Impressora",
  34567: "XMEye",
  37777: "Dahua",
  62078: "iPhone/iPad",
};

const ipNumber = (ip: string) => ip.split(".").reduce((n, part) => n * 256 + Number(part), 0);

function relativeTime(seconds: number, now: number) {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "agora";
  if (diff < 3600) return `há ${Math.floor(diff / 60)} min`;
  if (diff < 86400) return `há ${Math.floor(diff / 3600)} h`;
  return `há ${Math.floor(diff / 86400)} d`;
}

function vendorText(d: NetDevice) {
  return d.vendor ?? (d.privateMac ? "MAC privado" : "");
}

/** IP com o último octeto em destaque. */
function IpText({ ip }: { ip: string }) {
  const cut = ip.lastIndexOf(".") + 1;
  return (
    <span>
      {ip.slice(0, cut)}
      <strong className="text-slate-200">{ip.slice(cut)}</strong>
    </span>
  );
}

export default function NetworkPage({ cameras, onBack, onAddCamera }: Props) {
  const [devices, setDevices] = useState<NetDevice[]>([]);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<SortBy>("ip");
  const [open, setOpen] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now() / 1000);
  // Dispositivo sendo renomeado e o texto digitado.
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  async function scan() {
    setScanning(true);
    setError("");
    try {
      setDevices(await api.scanNetwork());
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
      setNow(Date.now() / 1000);
    }
  }

  // Mostra a última varredura salva e já começa uma nova.
  // A ref evita varrer duas vezes quando o StrictMode monta o efeito de novo no dev.
  const started = useRef(false);
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    api
      .networkDevices()
      .then(setDevices)
      .catch(() => {})
      .finally(scan);
  }, []);

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now() / 1000), 30000);
    return () => clearInterval(timer);
  }, []);

  async function forget(key: string) {
    try {
      setDevices(await api.forgetDevice(key));
    } catch (e) {
      setError(String(e));
    }
  }

  async function rename(key: string) {
    try {
      setDevices(await api.renameDevice(key, draft.trim()));
      setRenaming(null);
    } catch (e) {
      setError(String(e));
    }
  }

  const cameraFor = (d: NetDevice) =>
    cameras.find((c) => c.ip === d.ip || (!!d.tuyaId && c.deviceId === d.tuyaId));

  // Nome dado pelo usuário, depois o da câmera cadastrada, depois o detectado.
  const displayName = (d: NetDevice) => d.alias || cameraFor(d)?.name || d.name || "Genérico";

  const subnet = useMemo(() => {
    const ref = devices.find((d) => d.thisDevice) ?? devices[0];
    return ref ? `${ref.ip.split(".").slice(0, 3).join(".")}.0/24` : "";
  }, [devices]);

  const online = devices.filter((d) => d.online).length;
  const lastScan = devices.reduce((max, d) => Math.max(max, d.lastSeen), 0);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = devices.filter((d) =>
      [d.alias, d.name, d.ip, d.vendor, d.model, d.mac].some((v) => v?.toLowerCase().includes(q)),
    );
    // Sem nome ou fabricante vai para o fim da lista.
    const text = (v: string | null) => v?.toLowerCase() ?? "￿";
    return list.sort((a, b) => {
      if (sortBy === "name") return text(a.alias ?? a.name).localeCompare(text(b.alias ?? b.name));
      if (sortBy === "vendor") return text(a.vendor).localeCompare(text(b.vendor));
      return ipNumber(a.ip) - ipNumber(b.ip);
    });
  }, [devices, query, sortBy]);

  return (
    <div className="flex flex-col h-full">
      <header className="relative flex flex-wrap items-center justify-between gap-3 px-6 py-4 border-b border-white/10">
        <div className="flex items-center gap-3">
          <Button
            appearance="subtle"
            icon={<ArrowLeft24Regular />}
            onClick={onBack}
            title="Voltar para as câmeras"
            aria-label="Voltar para as câmeras"
          />
          <div>
            <h1 className="text-xl font-semibold tracking-tight">Dispositivos na rede</h1>
            <p className="text-xs text-slate-400">{subnet ? `Rede ${subnet}` : "Rede local"}</p>
          </div>
        </div>
        <Button appearance="primary" icon={<ArrowSync24Regular />} onClick={scan} disabled={scanning}>
          {scanning ? "Procurando…" : "Atualizar"}
        </Button>
        {scanning && <div className="absolute inset-x-0 bottom-0 h-0.5 animate-pulse bg-indigo-400" />}
      </header>

      {error && (
        <MessageBar intent="error" className="mx-6 mt-4">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}

      <div className="flex flex-wrap items-center gap-3 px-6 pt-4">
        <SearchBox
          value={query}
          onChange={(_, d) => setQuery(d.value)}
          placeholder="Buscar por nome, IP, fabricante, modelo ou MAC"
          className="min-w-[220px] flex-1 basis-64"
        />
        <Select value={sortBy} onChange={(_, d) => setSortBy(d.value as SortBy)}>
          <option value="ip">Ordenar por IP</option>
          <option value="name">Ordenar por nome</option>
          <option value="vendor">Ordenar por fabricante</option>
        </Select>
      </div>

      <div className="flex items-center justify-between px-6 py-3 text-sm text-slate-400">
        <span>
          {online} dispositivo{online === 1 ? "" : "s"} de {devices.length}
        </span>
        {lastScan > 0 && <span>{relativeTime(lastScan, now)}</span>}
      </div>

      <main className="flex-1 px-6 pb-6 overflow-y-auto">
        {devices.length === 0 ? (
          <p className="py-16 text-sm text-center text-slate-400">
            {scanning ? "Procurando dispositivos na rede…" : "Nenhum dispositivo encontrado."}
          </p>
        ) : (
          <ul className="overflow-hidden border divide-y divide-white/10 rounded-2xl border-white/10 bg-slate-900">
            {visible.map((d) => {
              const cam = cameraFor(d);
              const expanded = open === d.key;
              const canAdd =
                !cam &&
                (d.kind === "camera" ||
                  (d.kind === "iot" && !!d.tuyaId) ||
                  d.ports.includes(554) ||
                  d.ports.includes(6668));
              return (
                <li key={d.key} className={d.online ? "" : "opacity-50"}>
                  <button
                    onClick={() => setOpen(expanded ? null : d.key)}
                    className="flex items-center w-full gap-4 px-4 py-3 text-left hover:bg-white/5"
                  >
                    <span className="relative text-slate-300">
                      {ICONS[cam ? "camera" : d.kind]}
                      {d.online && (
                        <span className="absolute -bottom-0.5 -right-1 h-2.5 w-2.5 rounded-full border-2 border-slate-900 bg-emerald-400" />
                      )}
                    </span>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center justify-between gap-3">
                        <p className="flex items-center min-w-0 gap-2">
                          <span className="font-medium truncate">{displayName(d)}</span>
                          {d.thisDevice && <span className="text-xs shrink-0 text-slate-400">(este computador)</span>}
                          {d.gateway && (
                            <Badge appearance="tint" color="brand" size="small" className="shrink-0">
                              Roteador
                            </Badge>
                          )}
                          {cam && (
                            <Badge appearance="tint" color="success" size="small" className="shrink-0 truncate">
                              Cadastrada: {cam.name}
                            </Badge>
                          )}
                        </p>
                        <span className="text-sm shrink-0 text-slate-300">{vendorText(d)}</span>
                      </div>
                      <div className="flex items-center justify-between gap-3 text-sm text-slate-400">
                        <IpText ip={d.ip} />
                        <span className="truncate">{d.model ?? ""}</span>
                      </div>
                    </div>
                    <ChevronRight24Regular
                      className={`text-slate-500 transition-transform ${expanded ? "rotate-90" : ""}`}
                    />
                  </button>

                  {expanded && (
                    <div className="px-4 py-3 space-y-3 text-sm bg-slate-950/50">
                      {cam ? (
                        <p className="text-xs text-slate-400">
                          O nome desta câmera muda em <strong>Editar</strong>, no cartão dela.
                        </p>
                      ) : renaming === d.key ? (
                        <form
                          onSubmit={(e) => {
                            e.preventDefault();
                            rename(d.key);
                          }}
                          className="flex flex-wrap items-center gap-2"
                        >
                          <Input
                            autoFocus
                            value={draft}
                            onChange={(_, data) => setDraft(data.value)}
                            onKeyDown={(e) => e.key === "Escape" && setRenaming(null)}
                            placeholder={d.name || "Nome do dispositivo"}
                            className="min-w-[200px] flex-1"
                          />
                          <Button type="submit" appearance="primary" size="small">
                            Salvar
                          </Button>
                          <Button type="button" appearance="subtle" size="small" onClick={() => setRenaming(null)}>
                            Cancelar
                          </Button>
                          <p className="w-full text-xs text-slate-500">Deixe em branco para voltar ao nome detectado.</p>
                        </form>
                      ) : (
                        <Button
                          size="small"
                          onClick={() => {
                            setDraft(d.alias ?? d.name ?? "");
                            setRenaming(d.key);
                          }}
                        >
                          Renomear
                        </Button>
                      )}
                      <dl className="grid grid-cols-[auto,1fr] gap-x-4 gap-y-1">
                        {d.alias && (
                          <>
                            <dt className="text-slate-400">Nome detectado</dt>
                            <dd>{d.name ?? "—"}</dd>
                          </>
                        )}
                        <dt className="text-slate-400">MAC</dt>
                        <dd className="font-mono">
                          {d.mac ?? "—"}
                          {d.privateMac && <span className="ml-2 text-xs text-amber-300">aleatório/privado</span>}
                        </dd>
                        <dt className="text-slate-400">Fabricante</dt>
                        <dd>{d.vendor ?? "—"}</dd>
                        <dt className="text-slate-400">Modelo</dt>
                        <dd>{d.model ?? "—"}</dd>
                        {d.server && (
                          <>
                            <dt className="text-slate-400">Servidor</dt>
                            <dd className="break-all">{d.server}</dd>
                          </>
                        )}
                        {d.tuyaId && (
                          <>
                            <dt className="text-slate-400">Tuya ID</dt>
                            <dd className="font-mono">{d.tuyaId}</dd>
                          </>
                        )}
                        {!d.online && (
                          <>
                            <dt className="text-slate-400">Visto por último</dt>
                            <dd>{new Date(d.lastSeen * 1000).toLocaleString("pt-BR")}</dd>
                          </>
                        )}
                      </dl>

                      <div>
                        <p className="mb-1 text-slate-400">Portas abertas</p>
                        {d.ports.length === 0 ? (
                          <p className="text-xs text-slate-500">Nenhuma das portas conhecidas respondeu.</p>
                        ) : (
                          <div className="flex flex-wrap gap-1.5">
                            {d.ports.map((p) => (
                              <Badge key={p} appearance="outline" color="informative" size="small">
                                {p}
                                {PORT_LABELS[p] && <span className="text-slate-400"> · {PORT_LABELS[p]}</span>}
                              </Badge>
                            ))}
                          </div>
                        )}
                      </div>

                      {(canAdd || !d.online) && (
                        <div className="flex gap-2">
                          {canAdd && (
                            <Button size="small" appearance="primary" onClick={() => onAddCamera(d.ip)}>
                              Adicionar como câmera
                            </Button>
                          )}
                          {!d.online && (
                            <Button
                              size="small"
                              appearance="subtle"
                              icon={<Delete24Regular />}
                              onClick={() => forget(d.key)}
                            >
                              Esquecer
                            </Button>
                          )}
                        </div>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </main>
    </div>
  );
}
