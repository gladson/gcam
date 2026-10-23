# gcam

App desktop (Tauri 2 + React + Tailwind + Fluent UI) para cadastrar câmeras por IP. Ele detecta a marca, salva usuário e senha, e mostra o vídeo usando o **go2rtc** por baixo.

Roda em Windows, macOS e Linux.

## Recursos

- Cadastro de câmeras **RTSP** (Genérica, Dahua/Intelbras, Hikvision, TP-Link Tapo) e **Tuya / Smart Life** pela nuvem
- Vídeo ao vivo por WebRTC, com reconexão automática quando o sinal cai
- Controle **PTZ** (setas) e ligar/desligar o **rastreamento de movimento** em câmeras Tuya
- Scanner de rede, com os aparelhos da sub-rede, fabricante, nome e portas abertas

## 1. Pré-requisitos

- **Node.js 20+**
- **Rust** (https://rustup.rs)
- **Dependências do Tauri** para o seu sistema: https://v2.tauri.app/start/prerequisites/
  - Linux: pacotes `webkit2gtk-4.1`, `libappindicator3`, entre outros (a página acima lista todos)
  - macOS: Xcode Command Line Tools (`xcode-select --install`)
  - Windows: WebView2 (já vem no Windows 10/11) e Build Tools do Visual Studio

## 2. Binário do go2rtc

Ao abrir o app, se o go2rtc não estiver instalado, aparece um aviso com o botão **Baixar go2rtc**. Ele baixa a versão oficial (https://github.com/AlexxIT/go2rtc/releases) para o seu sistema e instala no local abaixo.
Também dá para instalar manualmente nesse mesmo local:

| Sistema | Local |
|---------|-------|
| Windows | `%APPDATA%\com.gladson.cameramanager\bin\go2rtc.exe` |
| macOS   | `~/Library/Application Support/com.gladson.cameramanager/bin/go2rtc` |
| Linux   | `~/.local/share/com.gladson.cameramanager/bin/go2rtc` |

Também funciona se o `go2rtc` estiver no PATH do sistema. No macOS/Linux, dê permissão com `chmod +x`.

## 3. Ícones

O ícone do app é a câmera do [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons) (MIT), gerado em `src-tauri/icons/`. Os ícones da interface vêm de `@fluentui/react-icons`. Para trocar o ícone do app, use uma imagem PNG quadrada (1024×1024) e rode:

```bash
npm install
npm run tauri icon caminho/para/logo.png
```

## 4. Rodar

```bash
npm install
npm run tauri dev      # desenvolvimento
npm run tauri build    # gera o instalador para o seu sistema
```

## 5. Testes e comandos úteis

```bash
npm run build              # checa os tipos do frontend e gera o build do Vite
cd src-tauri && cargo test # testes unitários do Rust (Tuya e scanner de rede)
npm run build:mac          # instalador para macOS (também build:win e build:linux)
npm run clean              # apaga node_modules, dist, src-tauri/gen e src-tauri/target
```

**Limitações**

- **macOS:** o build é só para Apple Silicon (arm64). Em Mac Intel, rode o app direto do código com o passo [4. Rodar](#4-rodar) (`npm run tauri dev` ou `npm run tauri build`).
- **Sem assinatura:** os instaladores não são assinados. O macOS (Gatekeeper) e o Windows (SmartScreen) mostram aviso de app não verificado na primeira abertura.
- **Sem atualização automática:** o app não se atualiza sozinho. Quem usa precisa baixar a release nova.

## Como funciona

- **Detecção de marca** (`src-tauri/src/detect.rs`): verifica quais portas respondem (80, 554, 6668…)
  e lê o cabeçalho `Server` da câmera. Porta 6668 sozinha indica câmera Tuya.
- **Configuração automática** (`src-tauri/src/go2rtc.rs`): a cada câmera salva ou removida, o app
  regrava o `go2rtc.yaml` e reinicia o go2rtc.
- **Dados** (`src-tauri/src/store.rs`): as câmeras ficam em `cameras.json` na pasta de dados do app.
- **Vídeo**: o player do próprio go2rtc é embutido em cada cartão, em `http://127.0.0.1:1984`.
- **Dispositivos na rede** (`src-tauri/src/network.rs`): o botão no topo abre uma lista com todos os aparelhos da sub-rede /24 do computador. O app lê a tabela ARP e pergunta o nome de cada aparelho por mDNS, NetBIOS e DNS reverso no roteador. Também usa SSDP/UPnP para ler modelo e fabricante, escuta o broadcast das câmeras Tuya e testa portas conhecidas (RTSP, HTTP, Tuya…). O fabricante vem do MAC, pela base do Wireshark. Ela é baixada na primeira varredura, fica na pasta de dados do app e é atualizada a cada 30 dias. Aparelhos com RTSP ou Tuya têm o botão **Adicionar como câmera**, que abre o cadastro com o IP preenchido.
- **Tuya** (`src-tauri/src/tuya_ptz.rs`, `src-tauri/src/tuya_local.rs`): as setas de PTZ falam com a nuvem da Tuya. O botão de rastreamento lê e muda a configuração direto na câmera, pelo protocolo local Tuya 3.4 (porta 6668), sem passar pela nuvem.

## Atenção

- Usuário e senha das câmeras são guardados **em texto puro** em `cameras.json`, na pasta de dados
  do seu usuário. Não compartilhe essa pasta.
- Para câmeras **Tuya**, use a conta do app **Tuya Smart** (contas Smart Life não funcionam com o go2rtc). O vídeo passa pela nuvem da Tuya e precisa de internet.
- O **rastreamento de movimento** usa a rede local, então a câmera Tuya precisa estar na mesma rede do computador e ter o IP cadastrado.
- Para corrigir uma câmera, use **Editar** no cartão. Nome e caminho RTSP aparecem só na edição.
- A detecção de marca é uma heurística. Se errar, troque a marca no formulário antes de salvar.
- Porta `8554` (RTSP) e `1984` (API/player do go2rtc) ficam só em `127.0.0.1`.

## Licença

Distribuído sob a licença [MIT](LICENSE).
