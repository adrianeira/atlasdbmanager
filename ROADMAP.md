# Roadmap

Este documento lista o que **ainda falta** no Atlas DB Manager para atingir **paridade de recursos
com o produto de referência do projeto**, que é o alvo de produto. O levantamento foi feito percorrendo o
código em busca de botões `disabled`, handlers `notImplemented`, itens de menu sem destino e
toolbars de categoria vazias.

Marque com `[x]` o que for sendo entregue. Itens com arquivo entre colchetes já têm "pontos de
entrada" no código (botão/ação existente, só falta implementar o comportamento).

> **Fora da prioridade atual:** as seções **9 (Automação)**, **11 (BI)** e
> **12 (Features cross-cutting)** não serão trabalhadas no ciclo atual. Ficam listadas aqui
> apenas como referência de paridade futura com o produto de referência.

## Entregues neste ciclo (2026-10-22)

Fora da lista original de paridade: ajustes de UX, novos recursos e polimento visual.

### Grid de dados
- [x] **Filter & Sort** (botão da toolbar) — diálogo "Filtrar e ordenar" com múltiplos operadores,
      ordenação multi-coluna, prévia do SQL e paginação ciente do filtro
      *(`src/components/FilterSortDialog.tsx`)*
- [x] **Columns** (botão da toolbar) — diálogo para mostrar/ocultar colunas do grid, com
      "Mostrar todas"/"Ocultar todas" (sempre sobra ao menos uma visível); navegação por teclado
      pula colunas ocultas; larguras ajustadas manualmente não se perdem ao alternar
      *(`src/components/ColumnsDialog.tsx`, `src/components/DataGrid.tsx`)*
- [x] Menu rápido **"⋯"** no hover do cabeçalho da coluna — Sort Ascending/Descending,
      Remove All Sorts, Add Filter (abre o diálogo Filter & Sort já com a coluna selecionada)
      *(`src/components/DataGrid.tsx`)*
- [x] **Cell Editor** (botão da toolbar, liga/desliga) — painel inferior com textarea grande,
      quebra de linha, "Formatar JSON", "Definir como NULL"; também acessível pelo menu de
      contexto da célula ("Edit in Cell Editor"); sincroniza em tempo real com a célula ativa
      *(`src/components/CellEditorPanel.tsx`)*
- [x] Barra **"Localizar dados"** (Ctrl+F) com navegação, "Highlight All" e **Substituir / Substituir tudo**
      *(`src/documents/TableDataDoc.tsx`, `src/components/DataGrid.tsx`)*
- [x] **Picker de data/hora próprio** (DateTimePopup): calendário mensal + relógio compactos, sem
      o widget nativo do Chromium *(`src/components/DateTimePopup.tsx`)*
- [x] **Ícone discreto** de calendário / relógio à direita das células date/time/timestamp
- [x] **Bloqueio automático de colunas geradas pelo banco** (`GENERATED ALWAYS AS IDENTITY` e
      `GENERATED ... STORED`) — backend ganhou `identity` e `generated` em `ColumnInfo`
      *(`crates/atlas-core/src/model.rs`, `crates/atlas-pg/src/meta.rs`, `src/components/DataGrid.tsx`)*
- [x] **Scroll automático** ao clicar "Adicionar registro" (rola o grid até a nova linha)
- [x] Botão **⏹ Parar** funcional (cancela a consulta no servidor via `pg.cancel`)
- [x] Botão **▶| Última página** funcional (`SELECT COUNT(*)` cacheado, calcula a última página)
- [x] Campo de **página editável** (digite e pressione Enter para saltar)

### Exportação de dados
- [x] **Assistente de exportação** de tabelas e views — CSV/TXT, JSON, SQL (`INSERT`) e Excel (`.xlsx`);
      respeita o **filtro, a ordenação e as colunas visíveis** do grid; escolha do arquivo pela janela
      nativa ou digitando o caminho; progresso em tempo real e **cancelamento** (a consulta é abortada
      no servidor e nenhum arquivo parcial fica no disco)
      *(`src/components/ExportDialog.tsx`, `src-tauri/src/export.rs`, `crates/atlas-export`)*
- [x] Gravação **em streaming**: 3 milhões de linhas (233 MB de CSV) em ~7,5 s com a memória do app
      estável; o arquivo é escrito em `<destino>.part` e só vira o destino ao terminar com sucesso
- [x] CSV/TXT com delimitador, qualificador, codificação (UTF-8, UTF-8 com BOM, Windows-1252),
      quebra de linha e texto para NULL; JSON com tipos nativos (número, booleano, `jsonb` embutido);
      SQL com N linhas por `INSERT`; Excel com cabeçalho, números e booleanos nativos
- [x] Pontos de entrada: botão **Exportar** do grid, **Salvar dados como...** (menu da célula),
      **Assistente de exportação** no menu da árvore e na barra da lista de objetos

### Navegação / Árvore
- [x] **Fechar schema** no menu de contexto (colapsa + marca como não-carregado para rebuscar
      na próxima abertura) *(`src/App.tsx`)*
- [x] Ícone do schema fica **cinza** quando fechado *(`src/components/NavigationPane.tsx`,
      `src/icons.tsx`)*

### Editor SQL / Toolbar da consulta
- [x] **Dropdown próprio** (connection/database/schema) no lugar do `<select>` nativo —
      hover/selecionado controlados por CSS (o popup do Chromium pintava cinza escuro no hover
      e ignorava o CSS) *(`src/components/Dropdown.tsx`)*
- [x] Linha amarela removida: autofill do Chromium neutralizado nos dropdowns

### Visual / Tema
- [x] Cor do checkbox padronizada em **#0277BD** (accent-color global) *(`src/styles/tokens.css`)*
- [x] Borda de foco mais fina (1 px) *(`src/styles/shell.css`)*
- [x] Dropdown: item hover/selecionado em **#0277BD** com texto branco
- [x] Campo da página do rodapé em azul-claro
- [x] Dropdowns da consulta com borda cinza (fundo branco mantido)

### i18n
- [x] Novas chaves `data.find.*`, `dtp.*`, `grid.generated`, `cmd.closeSchema` (pt + en)

## 1. Conexões
- [ ] Importar/exportar conexões (arquivo de perfil), compartilhamento via nuvem
- [ ] Importar conexão a partir de URI *(`src/components/ConnectionDialog.tsx:547,556`)*
- [ ] Adicionar/remover banco manualmente no diálogo *(`src/components/ConnectionDialog.tsx:431-432`)*
- [ ] SSH tunnel
- [ ] HTTP tunnel
- [ ] Autenticação Kerberos/GSSAPI
- [ ] Grupos (pastas) de conexão na árvore
- [ ] Novos provedores: MySQL, MariaDB, SQL Server, Oracle, SQLite, MongoDB, Redis,
      Snowflake etc. (hoje: só PostgreSQL; Firebird planejado)

## 2. Árvore / Navegação
- [ ] Filtro avançado da árvore *(`src/components/NavigationPane.tsx:172`)*
- [ ] Find in Database / Find in Schema *(item `cmd.findInDatabase` sem handler)*
- [ ] Console do servidor *(item `cmd.console` sem handler)*
- [ ] Share *(`cmd.share` sem handler)*
- [ ] Edit Database / Edit Schema *(sem handler)*

## 3. Grid de dados
- [x] Filter & Sort avançado — diálogo "Filtrar e ordenar" com WHERE multi-regra
      (operadores: `=`, `≠`, `<`, `≤`, `>`, `≥`, `LIKE`, `NOT LIKE`, `IN`, `BETWEEN`,
      `IS NULL`, `IS NOT NULL`), ordenação multi-coluna ASC/DESC, prévia do SQL e paginação
      que respeita o filtro (backend extendido com `OrderBy`/`TableFilter`)
      *(`src/components/FilterSortDialog.tsx`, `crates/atlas-pg/src/exec.rs`)*
- [x] Columns (mostrar/ocultar colunas)
- [ ] Data Profiling *(sem handler)*
- [ ] Data Generation *(sem handler)*
- [ ] Create BI Workspace *(sem handler)*
- [ ] Begin Transaction manual *(sem handler)*
- [x] Cell Editor expandido para texto grande / JSON (painel inferior com textarea, quebra de
      linha, "Formatar JSON" e "Definir como NULL"; aberto pelo botão da toolbar — liga/desliga —
      ou pelo menu de contexto da célula; sincroniza em tempo real com a célula ativa)
      — BLOB e XML formatado ficam para depois *(`src/components/CellEditorPanel.tsx`)*
- [ ] Visualizações alternativas: Image view, Hex view, Web view *(Form view já existe)*
- [ ] Edição de BLOB *(hoje só mostra "(BLOB) N bytes")*
- [ ] Modo "mostrar valor da FK em vez da chave" *(lookup já existe)*
- [ ] Agrupamento / sumário no rodapé do grid

## 4. Editor SQL
- [ ] Snippet manager *(`src/documents/QueryDoc.tsx` — `query.snippet` sem handler)*
- [ ] Ask AI *(`query.askAi` sem handler)*
- [ ] Split horizontal/vertical *(`src/documents/QueryDoc.tsx:292-293`)*
- [ ] Histórico de consultas
- [ ] Favoritos de SQL
- [ ] Transaction panel, Parameters window
- [ ] Beautify / Minify além de "Formatar"
- [ ] Plano de execução gráfico *(hoje `EXPLAIN` só em texto)*

## 5. Designers
- [ ] Trigger designer completo *(hoje só lista básica)*
- [ ] Rule designer *(PostgreSQL)*
- [ ] Partition designer
- [ ] Inherits browser *(`src/documents/TableDesignerDoc.tsx:617`)*
- [ ] Preview de dados no designer

## 6. Tipos de objeto não implementados
- [ ] Materialized View (designer + refresh)
- [ ] Function / Procedure designer com debugger de PL/pgSQL
- [ ] Type (ENUM, composite, domain)
- [ ] Sequence (manage, reset, cache)
- [ ] Trigger / Event Trigger
- [ ] Extension
- [ ] Foreign Server / Foreign Table
- [ ] Publication / Subscription (replicação lógica)
- [ ] Role / User manager
- [ ] Tablespace

## 7. Importar / Exportar
- [ ] Import Wizard *(`act.importWizard` sem handler)* — CSV, Excel, JSON, XML, DBF, Access…
- [x] Export Wizard de tabela/view: CSV/TXT, JSON, SQL e Excel (ver "Exportação de dados")
- [ ] Exportar o **resultado de uma consulta** do editor SQL
- [ ] Exportar **várias tabelas** de uma vez; mais formatos (XML, HTML, Markdown, DBF…)
- [ ] Salvar e reutilizar perfis de exportação; exportação agendada
- [ ] Dump SQL completo *(hoje só item de menu)*
- [ ] Data Transfer entre conexões / bancos
- [ ] Data Synchronization / Structure Synchronization

## 8. Backup / Restore
- [ ] Backup/Restore nativos (pg_dump / pg_restore embutidos)
- [ ] Backup agendado
- [ ] Extract SQL de um backup

## 9. Automação *(fora de prioridade neste ciclo)*
- [ ] Batch jobs (encadear consultas, backups, transferências)
- [ ] Agendamento cron-like
- [ ] Notificações por e-mail

## 10. Modelagem (Model)
- [ ] ER Diagram (criar, importar via "Reverse Schema to Model")
- [ ] Forward engineer (model → SQL)
- [ ] Compare models

## 11. BI *(fora de prioridade neste ciclo)*
- [ ] Workspace de dashboards (widgets, filtros, parâmetros, publicação)

## 12. Features cross-cutting *(fora de prioridade neste ciclo)*
- [ ] AI integrado (chat, geração de SQL, explicação, refactor)
- [ ] Sincronização em nuvem (conexões, consultas, snippets, modelos)
- [ ] Colaboração: comentários, histórico de versões
- [ ] Compare Database (schema diff)
- [ ] Server Monitor (métricas, locks, slow queries)
- [ ] Query Builder visual
- [ ] Pivot Table
- [ ] Charts a partir de consulta
- [ ] Dark mode manual *(hoje só segue o sistema)*
- [ ] Tela de Preferências / Options
- [ ] Atalhos de teclado configuráveis
- [ ] Fullscreen por painel *(vários botões `fullscreen` desabilitados)*

## 13. Toolbar / rodapé — itens menores
- [ ] Conta (login do serviço em nuvem) *(`src/components/MainToolbar.tsx:100`)*
- [ ] Beautify de célula
- [ ] Hex / Binary editor próprio
