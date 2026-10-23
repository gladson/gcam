import { useEffect, useState, type FormEvent } from "react";
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  MessageBar,
  MessageBarBody,
  Select,
} from "@fluentui/react-components";
import { api } from "../api";
import { BRANDS, TUYA_REGIONS, brandLabel, defaultPath } from "../presets";
import type { Camera, Detection, TuyaDevice } from "../types";

const emptyCamera: Camera = {
  id: "",
  name: "",
  brand: "generic",
  ip: "",
  username: "",
  password: "",
  path: defaultPath("generic"),
  deviceId: "",
  tuyaHost: "protect-us.ismartlife.me",
};

interface Props {
  /** Câmera a editar. Sem ela, o formulário adiciona uma nova. */
  camera?: Camera;
  /** IP já conhecido (vindo da lista de dispositivos na rede); a detecção roda ao abrir. */
  initialIp?: string;
  onClose: () => void;
  onSaved: (cameras: Camera[]) => void;
}

export default function AddCameraModal({ camera, initialIp, onClose, onSaved }: Props) {
  const editing = !!camera;
  const [form, setForm] = useState<Camera>(camera ?? { ...emptyCamera, ip: initialIp ?? "" });
  const [detection, setDetection] = useState<Detection | null>(null);
  // Ao editar uma Tuya, a lista começa com a câmera atual até o usuário entrar de novo na conta.
  const [tuyaDevices, setTuyaDevices] = useState<TuyaDevice[]>(
    camera?.brand === "tuya" && camera.deviceId ? [{ deviceId: camera.deviceId, name: camera.name }] : [],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const set = <K extends keyof Camera>(key: K, value: Camera[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const isTuya = form.brand === "tuya";
  const ipOk = /^(\d{1,3}\.){3}\d{1,3}$/.test(form.ip.trim());

  async function detect() {
    if (!ipOk) return;
    setBusy(true);
    setError("");
    try {
      const d = await api.detect(form.ip.trim());
      setDetection(d);
      setForm((f) => ({
        ...f,
        brand: d.brand,
        path: d.suggestedPath || defaultPath(d.brand),
      }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (initialIp && !editing) detect();
    // Só ao abrir: o IP veio pronto da lista de dispositivos.
  }, []);

  function changeBrand(brand: string) {
    setForm((f) => ({ ...f, brand, path: defaultPath(brand) }));
  }

  // Trocar região, e-mail ou senha invalida a lista de câmeras já carregada.
  function changeTuyaField<K extends "tuyaHost" | "username" | "password">(key: K, value: string) {
    set(key, value);
    setTuyaDevices([]);
    set("deviceId", "");
  }

  async function tuyaLogin() {
    if (!form.username.trim() || !form.password) {
      setError("Informe o e-mail e a senha da conta Tuya.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const devices = await api.tuyaLogin(form.tuyaHost, form.username.trim(), form.password);
      setTuyaDevices(devices);
      // Ao editar, mantém a câmera atual se ela ainda estiver na conta.
      const keep = devices.find((d) => d.deviceId === form.deviceId);
      set("deviceId", (keep ?? devices[0]).deviceId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    // Enter nos campos Tuya antes do login faz o login, em vez de salvar.
    if (isTuya && tuyaDevices.length === 0) {
      return tuyaLogin();
    }
    if (!isTuya && !ipOk) {
      setError("Informe um IP válido, por exemplo 192.168.0.6");
      return;
    }
    if (isTuya && !form.deviceId) {
      setError("Entre na conta Tuya e escolha a câmera.");
      return;
    }
    setBusy(true);
    const ip = form.ip.trim();
    const device = tuyaDevices.find((d) => d.deviceId === form.deviceId);
    const payload: Camera = {
      ...form,
      ip,
      path: form.path || defaultPath(form.brand),
      name: form.name.trim() || device?.name || `${brandLabel(form.brand)} ${ip}`,
    };
    try {
      const cameras = editing ? await api.update(payload) : await api.save(payload);
      onSaved(cameras);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface>
        <form onSubmit={submit}>
          <DialogBody>
            <DialogTitle>{editing ? "Editar câmera" : "Adicionar câmera"}</DialogTitle>
            <DialogContent className="flex flex-col gap-4">
              <p className="text-xs text-slate-400">
                {editing
                  ? "Altere o que precisar e salve. O vídeo reinicia com os novos dados."
                  : "Informe o IP e a marca. O restante é configurado automaticamente."}
              </p>

              <Field label={`Endereço IP${isTuya ? " (opcional para Tuya)" : ""}`}>
                <div className="flex gap-2">
                  <Input
                    placeholder="192.168.0.6"
                    value={form.ip}
                    onChange={(_, d) => set("ip", d.value)}
                    // Ao editar, não redetecta ao sair do campo: isso trocaria o caminho que o usuário ajustou.
                    onBlur={editing ? undefined : detect}
                  />
                  <Button type="button" onClick={detect} disabled={!ipOk || busy}>
                    Detectar
                  </Button>
                </div>
              </Field>

              <Field label="Marca">
                <Select value={form.brand} onChange={(_, d) => changeBrand(d.value)}>
                  {BRANDS.map((b) => (
                    <option key={b.id} value={b.id}>
                      {b.label}
                    </option>
                  ))}
                </Select>
              </Field>

              {detection && (
                <MessageBar intent="info" layout="multiline">
                  <MessageBarBody>
                    <p>
                      Marca: <strong>{brandLabel(detection.brand)}</strong>
                    </p>
                    <p>Portas abertas: {detection.openPorts.join(", ") || "nenhuma"}</p>
                    {detection.server && <p>Servidor: {detection.server}</p>}
                    <p>{detection.note}</p>
                  </MessageBarBody>
                </MessageBar>
              )}

              {isTuya ? (
                <div className="flex flex-col gap-3">
                  <div className="grid grid-cols-2 gap-3">
                    <Field label="Região">
                      <Select value={form.tuyaHost} onChange={(_, d) => changeTuyaField("tuyaHost", d.value)}>
                        {TUYA_REGIONS.map((r) => (
                          <option key={r.host} value={r.host}>
                            {r.label}
                          </option>
                        ))}
                      </Select>
                    </Field>
                    <Field label="E-mail da conta">
                      <Input
                        autoComplete="off"
                        value={form.username}
                        onChange={(_, d) => changeTuyaField("username", d.value)}
                      />
                    </Field>
                    <Field label="Senha">
                      <Input
                        type="password"
                        autoComplete="new-password"
                        value={form.password}
                        onChange={(_, d) => changeTuyaField("password", d.value)}
                      />
                    </Field>
                    <div className="flex items-end">
                      <Button type="button" onClick={tuyaLogin} disabled={busy} className="w-full">
                        {tuyaDevices.length > 0 ? "Entrar de novo" : "Entrar na conta"}
                      </Button>
                    </div>
                  </div>

                  {tuyaDevices.length > 0 && (
                    <Field label={`Câmera (${tuyaDevices.length} na conta)`}>
                      <Select value={form.deviceId} onChange={(_, d) => set("deviceId", d.value)}>
                        {tuyaDevices.map((d) => (
                          <option key={d.deviceId} value={d.deviceId}>
                            {d.name || d.deviceId}
                          </option>
                        ))}
                      </Select>
                    </Field>
                  )}

                  <p className="text-xs text-slate-400">
                    Use a conta do app Tuya Smart (não Smart Life). O vídeo passa pela nuvem da Tuya.
                  </p>
                </div>
              ) : (
                <>
                  {editing && (
                    <>
                      <Field label="Nome">
                        <Input value={form.name} onChange={(_, d) => set("name", d.value)} />
                      </Field>
                      <Field label="Caminho RTSP">
                        <Input value={form.path} onChange={(_, d) => set("path", d.value)} />
                      </Field>
                    </>
                  )}
                  <Accordion collapsible defaultOpenItems={editing && camera?.username ? "credenciais" : undefined}>
                    <AccordionItem value="credenciais">
                      <AccordionHeader>Usuário e senha (só se a câmera exigir)</AccordionHeader>
                      <AccordionPanel>
                        <div className="grid grid-cols-2 gap-3 pt-2">
                          <Field label="Usuário">
                            <Input
                              autoComplete="off"
                              value={form.username}
                              onChange={(_, d) => set("username", d.value)}
                            />
                          </Field>
                          <Field label="Senha">
                            <Input
                              type="password"
                              autoComplete="new-password"
                              value={form.password}
                              onChange={(_, d) => set("password", d.value)}
                            />
                          </Field>
                        </div>
                      </AccordionPanel>
                    </AccordionItem>
                  </Accordion>
                </>
              )}

              {error && (
                <MessageBar intent="error">
                  <MessageBarBody>{error}</MessageBarBody>
                </MessageBar>
              )}
            </DialogContent>

            <DialogActions>
              <Button type="button" appearance="secondary" onClick={onClose}>
                Cancelar
              </Button>
              <Button type="submit" appearance="primary" disabled={busy}>
                {busy ? "Aguarde…" : editing ? "Salvar alterações" : "Salvar câmera"}
              </Button>
            </DialogActions>
          </DialogBody>
        </form>
      </DialogSurface>
    </Dialog>
  );
}
