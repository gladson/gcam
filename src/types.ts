export interface Camera {
  id: string;
  name: string;
  brand: string;
  ip: string;
  username: string;
  password: string;
  path: string;
  deviceId: string;
  tuyaHost: string;
}

export interface Detection {
  brand: string;
  openPorts: number[];
  server: string | null;
  suggestedPath: string;
  note: string;
}

export type PtzDirection = "UP" | "DOWN" | "LEFT" | "RIGHT";

export interface TuyaDevice {
  deviceId: string;
  name: string;
}

export type DeviceKind = "router" | "camera" | "tv" | "phone" | "computer" | "printer" | "iot" | "unknown";

/** Aparelho encontrado na varredura da rede local. */
export interface NetDevice {
  key: string;
  ip: string;
  mac: string | null;
  privateMac: boolean;
  vendor: string | null;
  name: string | null;
  /** Nome dado pelo usuário; vale mais que o detectado. */
  alias: string | null;
  model: string | null;
  kind: DeviceKind;
  ports: number[];
  server: string | null;
  tuyaId: string | null;
  gateway: boolean;
  thisDevice: boolean;
  online: boolean;
  /** Segundos desde 1970. */
  lastSeen: number;
}
