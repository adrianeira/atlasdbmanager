# Identidade visual do Atlas

Arquivos de marca do projeto. Esta pasta **é versionada** (diferente de `capturas/`), então só entra
aqui o que for nosso ou de uso livre.

## Arquivos

| Arquivo | O que é |
|---|---|
| `logo-mini.png` | Logo original (PNG com fundo transparente). **É a fonte.** |
| `splachscreen.png` | Imagem original da tela de abertura. **É a fonte.** |
| `logo-1024.png` | *Gerado.* Logo quadrado 1024×1024, transparente, com respiro; base de todos os ícones |

Para trocar o logo ou a splash, substitua os arquivos de origem (mesmos nomes) e rode a geração abaixo.

## Onde cada um é usado

- **Ícone do app** (janela, barra de tarefas, instalador, atalho): `src-tauri/icons/`, gerado de `logo-1024.png`.
- **Tela de abertura:** `public/splash.jpg` (imagem, sem barra nem texto) + `public/splash.html`/`splash.js` (barra e texto animados por cima; ajuste `.progress` no CSS se mudar a imagem). Fica até as conexões serem lidas (mín. 1,5 s, máx. 20 s).
- **Janela "Sobre" e favicon:** `public/logo.png` (256×256).

## Regerar

```bash
pip install pillow
python scripts/brand-assets.py .                       # logo-1024.png, public/logo.png, public/splash.jpg
npx tauri icon branding/logo-1024.png -o <pasta-temporaria>
```

Copie os **arquivos** da pasta temporária para `src-tauri/icons/` (as pastas `android/` e `ios/` não são usadas).
O `build.rs` observa `icons/`, então o executável regrava o ícone na próxima compilação.

Dica: deixe um respiro de uns 10% nas bordas do logo; alguns sistemas (macOS, Android) recortam os cantos.
