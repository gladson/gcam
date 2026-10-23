import { invoke } from "@tauri-apps/api/core";
import type { Camera, Detection, NetDevice, PtzDirection, TuyaDevice } from "./types";

/** Chamadas para o backend Rust (src-tauri). */
export const api = {
  list: () => invoke<Camera[]>("list_cameras"),
  save: (camera: Camera) => invoke<Camera[]>("save_camera", { camera }),
  update: (camera: Camera) => invoke<Camera[]>("update_camera", { camera }),
  remove: (id: string) => invoke<Camera[]>("delete_camera", { id }),
  detect: (ip: string) => invoke<Detection>("detect_camera", { ip }),
  tuyaLogin: (region: string, email: string, password: string) =>
    invoke<TuyaDevice[]>("tuya_login", { region, email, password }),
  streams: () => invoke<Record<string, unknown>>("go2rtc_status"),
  ptzMove: (id: string, direction: PtzDirection) => invoke<void>("ptz_move", { id, direction }),
  ptzStop: (id: string) => invoke<void>("ptz_stop", { id }),
  trackingGet: (id: string) => invoke<boolean>("tracking_get", { id }),
  trackingSet: (id: string, on: boolean) => invoke<boolean>("tracking_set", { id, on }),
  restartEngine: () => invoke<void>("restart_go2rtc"),
  engineInstalled: () => invoke<boolean>("go2rtc_installed"),
  installEngine: () => invoke<void>("install_go2rtc"),
  networkDevices: () => invoke<NetDevice[]>("network_devices"),
  scanNetwork: () => invoke<NetDevice[]>("scan_network"),
  forgetDevice: (key: string) => invoke<NetDevice[]>("forget_device", { key }),
  renameDevice: (key: string, alias: string) => invoke<NetDevice[]>("rename_device", { key, alias }),
};
