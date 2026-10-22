# Identidade visual do Atlas

Coloque aqui os arquivos de marca do projeto. Esta pasta **é versionada** (diferente de `capturas/`),
então só ponha o que for seu ou de uso livre.

## O que colocar

| Arquivo | Para quê | Formato recomendado |
|---|---|---|
| `logo.png` (ou `logo.svg`) | Ícone do aplicativo: janela, barra de tarefas, instalador, atalho | PNG quadrado de **1024×1024**, fundo **transparente**; ou SVG |
| `splash.png` | Tela de abertura (aparece enquanto o app carrega) | PNG, por exemplo **800×500**; qualquer proporção serve |
| `logo-escuro.png` *(opcional)* | Variante para fundo escuro, se o logo principal sumir nele | mesmo formato do `logo.png` |

Outros nomes funcionam; me diga quais usou.

## O que acontece com eles

- `logo.png` → o comando `npm run tauri icon branding/logo.png` gera todos os tamanhos e formatos em
  `src-tauri/icons/` (`.ico` do Windows, `.icns` do macOS, PNGs do Linux e do instalador).
- `splash.png` → entra na janela de abertura do app.

Deixe um respiro de uns 10% nas bordas do logo: alguns sistemas (macOS, Android) recortam os cantos.
