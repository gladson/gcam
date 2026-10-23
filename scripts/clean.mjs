// Remove o que é gerado pelo projeto: dependências, build e caches.
// Mantém o código e os lockfiles (package-lock.json, bun.lock, Cargo.lock), que garantem a mesma versão ao reinstalar.
import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// Sem argumentos: qualquer texto a mais (ex.: `npm run clean -- --help`) não pode apagar nada.
if (process.argv.length > 2) {
  console.log("Uso: npm run clean   (apaga node_modules, dist, caches e src-tauri/target; não aceita argumentos)");
  process.exit(1);
}

const root = process.cwd();

const targets = [
  "node_modules",
  "dist",
  ".vite",
  // Gerado pelo Tauri a cada build; volta sozinho no próximo `tauri dev`.
  "src-tauri/gen",
  // Build do Rust: é a maior pasta (vários GB).
  "src-tauri/target",
];

// Cache do TypeScript (tsconfig.tsbuildinfo) na raiz.
function findTsbuildinfo(dir) {
  return readdirSync(dir)
    .filter((name) => name.endsWith(".tsbuildinfo"))
    .map((name) => join(dir, name));
}

function size(path) {
  const info = statSync(path);
  if (!info.isDirectory()) return info.size;
  return readdirSync(path).reduce((sum, name) => sum + size(join(path, name)), 0);
}

function format(bytes) {
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit ? 1 : 0)} ${units[unit]}`;
}

const paths = targets
  .map((t) => join(root, t))
  .filter(existsSync)
  .concat(findTsbuildinfo(root));

if (paths.length === 0) {
  console.log("Nada para limpar.");
  process.exit(0);
}

let freed = 0;
for (const path of paths) {
  const bytes = size(path);
  rmSync(path, { recursive: true, force: true });
  freed += bytes;
  console.log(`removido ${relative(root, path)} (${format(bytes)})`);
}
console.log(`Total liberado: ${format(freed)}. Para reinstalar: npm install`);
