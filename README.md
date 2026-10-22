# Atlas DB Manager

[English](README.en.md) · **Português**

Gerenciador de bancos de dados para desktop, nativo e multiplataforma (Windows, Linux e macOS),
feito em **Rust + Tauri 2 + React**. O foco é o trabalho do dia a dia com o banco: navegar pelos
objetos, editar dados num grid rápido, escrever SQL e projetar tabelas e views, com a segurança
de revisar o que vai ser executado.

> **Status:** em desenvolvimento (versão 0.1, pré-alfa). O primeiro provedor é o **PostgreSQL**;
> o **Firebird** (versão 1.5 em diante) está planejado.

## Funcionalidades

**Conexões**
- Conexões PostgreSQL com SSL, teste de conexão e cores/favoritos por conexão.
- Senhas guardadas no **cofre de credenciais do sistema** (Windows Credential Manager, Keychain
  do macOS, Secret Service no Linux) — nunca em arquivo nem enviadas para a interface.
- Configurações locais em SQLite no perfil do usuário, **sem precisar de administrador**.

**Navegação**
- Árvore de conexões → bancos → schemas → tabelas, views, views materializadas e funções, com
  colunas e ícones próprios para chave primária e chave estrangeira.
- Lista de objetos (detalhes, lista e ícones) e menus de contexto em ambos.

**Dados (grid)**
- Grid virtualizado com cabeçalho de nome + tipo, linhas zebradas e `(Null)` destacado.
- Edição direta com **gravação automática ao mudar de registro**; botão Aplicar/Descartar.
- Valores sempre enviados como **parâmetros** (nada concatenado no SQL) e concorrência otimista:
  o `UPDATE` confere o valor original e falha se outra pessoa alterou o registro.
- Campos obrigatórios (`NOT NULL`) bloqueados para vazio; booleanos como checkbox.
- Lookup de chave estrangeira (botão `…` na célula), paginação e limite de registros configurável.
- Menu de contexto da célula: definir NULL/vazio, gerar UUID (v4/v7), copiar, copiar como
  `INSERT`/`UPDATE`/TSV, colar, excluir registro.

**Exportação**
- Assistente de exportação de tabelas e views para **CSV/TXT, JSON, SQL (`INSERT`) e Excel (`.xlsx`)**,
  respeitando o filtro, a ordenação e as colunas visíveis do grid.
- Gravação em **streaming** (memória estável mesmo com milhões de linhas), com progresso e cancelamento;
  o arquivo só passa a existir quando a exportação termina com sucesso.

**SQL e estrutura**
- Editor SQL (CodeMirror) com destaque do dialeto PostgreSQL, autocomplete, executar,
  `EXPLAIN`, formatar e cancelar consulta no servidor.
- Designer de tabela (campos, índices, FKs, únicos, checks, triggers, comentário) com prévia do SQL.
- Designer de view (definição, opções, comentário) e abertura dos dados da view.
- Comandos de estrutura (criar/excluir/renomear/duplicar) sempre revisados antes de executar.

**Interface**
- Tema claro/escuro (acompanha o sistema), português e inglês.

## Requisitos

- [Rust](https://rustup.rs) 1.90 ou superior
- [Node.js](https://nodejs.org) 20 ou superior
- Pré-requisitos do Tauri 2 para o seu sistema
  ([guia oficial](https://v2.tauri.app/start/prerequisites/)):
  - **Windows:** Microsoft C++ Build Tools e WebView2 (já incluso no Windows 10/11).
  - **Linux (Debian/Ubuntu):**
    `sudo apt install build-essential pkg-config libssl-dev libwebkit2gtk-4.1-dev librsvg2-dev libdbus-1-dev`
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`).

## Como rodar

```bash
npm install

# Aplicativo desktop em modo de desenvolvimento
npm run tauri dev

# Só a interface no navegador (modo de demonstração, com dados fictícios)
npm run dev        # http://localhost:1420
```

## Gerar o instalador

```bash
npm run tauri build
```

Os pacotes ficam em `target/release/bundle/` (MSI/NSIS no Windows, `.deb`/`.rpm`/AppImage no
Linux, `.app`/`.dmg` no macOS).

## Testes

```bash
npx tsc --noEmit -p .        # tipos do frontend
cargo test --workspace       # testes Rust
```

Os testes contra um PostgreSQL real só rodam quando as variáveis abaixo existem (as credenciais
nunca ficam no repositório). Eles usam apenas objetos temporários:

```bash
ATLAS_TEST_PG_HOST=localhost ATLAS_TEST_PG_PORT=5432 \
ATLAS_TEST_PG_USER=postgres ATLAS_TEST_PG_PASSWORD=... \
cargo test -p atlas-pg --test live_postgres
```

O teste do cofre de credenciais é opcional: `ATLAS_TEST_KEYRING=1 cargo test -p atlas-local`.

## Estrutura

```
crates/
  atlas-core/    domínio e contratos (tipos, erros, segredo que não vaza em log)
  atlas-pg/      provedor PostgreSQL (tokio-postgres): conexão, catálogo, execução, gravação
  atlas-local/   armazenamento local (SQLite) e cofre de credenciais do sistema
  atlas-export/  gravação de resultados em CSV, JSON, SQL e Excel (streaming)
src-tauri/       aplicativo desktop (comandos Tauri, sessões, janela)
src/             interface React + TypeScript
scripts/         utilitários (ex.: geração da lista de licenças de terceiros)
```

## Onde ficam os dados locais

| Sistema | Pasta |
|---|---|
| Windows | `%LOCALAPPDATA%\dev.atlas.dbmanager\` |
| Linux | `~/.local/share/dev.atlas.dbmanager/` |
| macOS | `~/Library/Application Support/dev.atlas.dbmanager/` |

O arquivo `atlas.db` guarda conexões e preferências — **sem senhas**, que ficam no cofre do sistema.

## Licença

Distribuído sob a licença **MIT** — veja [LICENSE](LICENSE).

As dependências de terceiros mantêm as licenças dos seus autores (MIT, Apache-2.0, BSD, Zlib,
Unicode-3.0, MPL-2.0, entre outras). A lista completa está em
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) e pode ser regenerada com
`node scripts/third-party-licenses.mjs`. Os poucos componentes sob **MPL-2.0** (usados pelo
Tauri) continuam sob essa licença; ela vale arquivo a arquivo e não afeta o código do Atlas.

Os logotipos dos bancos de dados vêm do projeto [Simple Icons](https://simpleicons.org) (CC0-1.0).
PostgreSQL, Firebird e os demais nomes e logotipos são marcas dos seus respectivos donos; o uso
aqui é apenas para identificar o tipo de conexão e não implica afiliação ou endosso.
