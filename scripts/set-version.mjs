// Grava a versão do app no tauri.conf.json antes do build. Usado pelo workflow de release.
import { readFileSync, writeFileSync } from "node:fs";

const version = process.env.VERSION;
if (!version) throw new Error("Defina a variável VERSION");

const file = "src-tauri/tauri.conf.json";
const conf = JSON.parse(readFileSync(file, "utf8"));
conf.version = version;
writeFileSync(file, JSON.stringify(conf, null, 2) + "\n");
console.log(`Versão do app: ${version}`);
