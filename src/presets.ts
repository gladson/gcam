export interface Brand {
  id: string;
  label: string;
  /** Caminho RTSP padrão (vazio quando a marca usa a nuvem). */
  path: string;
}

export const BRANDS: Brand[] = [
  { id: "generic", label: "Genérica (RTSP)", path: "/cam/realmonitor?channel=1&subtype=0" },
  { id: "dahua", label: "Dahua / Intelbras", path: "/cam/realmonitor?channel=1&subtype=0" },
  { id: "hikvision", label: "Hikvision", path: "/Streaming/Channels/101" },
  { id: "tapo", label: "TP-Link Tapo", path: "/stream1" },
  { id: "tuya", label: "Tuya / Smart Life (nuvem)", path: "" },
];

/** Regiões da conta Tuya (mesmos servidores da tela de login do go2rtc). */
export const TUYA_REGIONS = [
  { host: "protect-eu.ismartlife.me", label: "EU Central" },
  { host: "protect-we.ismartlife.me", label: "EU East" },
  { host: "protect-us.ismartlife.me", label: "US West" },
  { host: "protect-ue.ismartlife.me", label: "US East" },
  { host: "protect.ismartlife.me", label: "China" },
  { host: "protect-in.ismartlife.me", label: "India" },
];

export function brandLabel(id: string): string {
  return BRANDS.find((b) => b.id === id)?.label ?? id;
}

export function defaultPath(id: string): string {
  return BRANDS.find((b) => b.id === id)?.path ?? "/";
}
