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

## Entregues neste ciclo (até 2026-10-23)

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
- [x] **Iniciar transação** (botão da toolbar): abre uma transação numa conexão dedicada; o Aplicar passa a
      gravar dentro dela (cada lote sob SAVEPOINT, então um erro desfaz só o lote), a grade relê enxergando as
      alterações, e **Confirmar** (COMMIT) / **Desfazer** (ROLLBACK) encerram. Fechar a aba desfaz tudo
      *(`src-tauri/src/tx.rs`, `crates/atlas-pg/src/apply.rs`)*
- [x] Botão **⏹ Parar** funcional (cancela a consulta no servidor via `pg.cancel`)
- [x] Botão **▶| Última página** funcional (`SELECT COUNT(*)` cacheado, calcula a última página)
- [x] Campo de **página editável** (digite e pressione Enter para saltar)

- [x] Colunas **boolean** como caixa de seleção (marcada / desmarcada / esmaecida para NULL), alternada
      com um clique; ícones próprios para colunas **PK** e **FK** no cabeçalho
- [x] **"Definir como NULL"** desabilitado em colunas `NOT NULL`; comentário da coluna (do banco) como
      dica no cabeçalho
- [x] Edição de célula salva ao sair do campo quando passa na validação

### Janelas e diálogos
- [x] Cabeçalho único nos diálogos (assistentes, Opções, Histórico, Snippets, Localizar, Dump, Colunas, Filtro e
      ordenação): fundo branco, sem borda inferior, ícone do aplicativo e título sem negrito
      *(`.modal-header` em `src/styles/components.css`)*
- [x] **Botão X no cabeçalho** de todos os diálogos (assistentes de exportação e importação, Opções, Histórico, Snippets,
      Localizar, Dump, Colunas, Filtro e ordenação e as opções avançadas); fica desabilitado enquanto uma exportação,
      importação ou dump está rodando
- [x] **Botões no padrão** (`btn`, cinza, com o principal em destaque) em Filtro e ordenação, Colunas, editor de célula e
      seletor de data/hora, que usavam uma classe sem estilo fora da barra de busca da grade
- [x] **Confirmações sem mostrar o SQL** (como na referência): excluir/esvaziar/truncar perguntam com o nome do objeto
      (`Excluir tabela "x"?`), o Aplicar do grid resume "N inserido(s), N alterado(s), N excluído(s)" e os designers
      confirmam só com a pergunta e os avisos (recriar função/view materializada, perda de dados). O que não é
      destrutivo roda direto depois do formulário: novo banco/schema, renomear, duplicar tabela, atualizar view
      materializada, Editar banco/schema (renomear banco continua perguntando, e em produção pede o nome). O SQL
      segue visível onde a própria referência mostra: abas de prévia dos designers e do Filtro e ordenação
      *(`src/components/MessageDialog.tsx`, `askDdl` em `src/App.tsx`)*

### Query Builder *(prioridade máxima)*
- [x] **Janela do sistema à parte** (não um diálogo dentro da janela principal): maximiza pela tela inteira, independe do
      tamanho da janela principal e mantém a principal utilizável; uma aba abre no máximo uma janela (o botão traz a
      aberta para a frente) e o SELECT volta pelo evento `qb-build` *(`src-tauri/src/builder.rs`, `QueryBuilderWindow.tsx`)*
- [x] **Janela do Query Builder** aberta pelo botão da aba de consulta: árvore de schemas/tabelas/views, canvas
      com cartões de tabela (arrastar da árvore ou duplo clique, mover pelo título, `Delete` remove), campos
      marcáveis (inclusive `*`), SQL ao vivo com palavras-chave coloridas, **Build** e **Build and Run**
      (insere o SELECT no editor e, se houver texto, executa só o novo trecho) e painéis laterais ocultáveis
      *(`src/components/querybuilder/`, `src/sql/querybuilder.ts`)*
- [x] **Relações**: JOINs criados sozinhos pelas chaves estrangeiras ao soltar a tabela; ligar campo a campo
      arrastando; "Field Relation" / "Table Relation"; tipo INNER/LEFT/RIGHT/FULL e remoção na aba FROM
- [x] Abas **SELECT** (apelido, agregações COUNT/SUM/AVG/MIN/MAX, DISTINCT, ordem), **FROM**, **WHERE** (AND/OR,
      `=`, `<>`, `<`, `>`, LIKE, ILIKE, IN, BETWEEN, IS NULL...), **GROUP BY**, **HAVING**, **ORDER BY** e **LIMIT/OFFSET**
- [x] **Reabrir a partir do SQL do editor**: lê a seleção (ou o texto todo) se for um SELECT simples (tabelas com
      apelido, JOINs, WHERE/HAVING, GROUP BY, ORDER BY, LIMIT/OFFSET, agregações), monta o desenho e, no Build,
      substitui aquele trecho; fora desse formato avisa e começa do zero
      *(`src/sql/querybuilderParse.ts`)*
- [x] **Apelido da tabela** (duplo clique no título abre "Edit Alias"; o título mostra `tabela ( apelido )`),
      **menu de clique direito** no cartão (apelido, marcar/desmarcar campos, remover) e na relação (tipo de JOIN,
      remover), rótulo do tipo na linha (LEFT/RIGHT/FULL)
- [x] **Posição dos cartões lembrada**: ao dar Build, o desenho é guardado pelo texto do SELECT e volta igual ao
      reabrir o builder a partir dele (últimas 100 consultas, no armazenamento local)
- [x] Cartão com muitas colunas: mostra até 12 linhas e rola por dentro (a linha da relação acompanha o campo e
      prende na borda quando ele rola para fora); posição dos cartões também volta quando o texto do SELECT mudou
      mas as tabelas são as mesmas
- [x] **Redimensionar o cartão** pela alça do canto inferior direito (largura e altura, mínimo 120 x 56 px; duplo
      clique na alça volta ao tamanho automático); o tamanho é guardado junto com a posição
- [x] Verificado: FK composta (uma linha por coluna e `ON ... AND ...`), 400 tabelas na árvore (~0,3 s), 30 cartões
      no canvas, views, outro schema, tema escuro e PostgreSQL 9.6 e 18
- [x] Views reabertas de um SELECT aparecem como views (cor e ícone); redimensionar também pelas bordas direita e
      inferior; o Build não apaga o que foi escrito no editor enquanto a janela estava aberta (acrescenta ao fim);
      fechar a aba de consulta fecha a janela do builder dela; janela separada verificada no PostgreSQL 9.6 e 18
- [x] Ajustes pela referência: apelido da tabela com `AS` (`clients AS teste`), seta nas duas pontas da relação, botão
      `+` ao lado do último campo do SELECT e aviso claro quando a tabela do SELECT não existe no schema da aba
- [x] **Table Relation** como no original: a linha liga o centro de cada cartão (bolinhas nas pontas) e leva no meio o
      ícone de diagrama de Venn do tipo de JOIN (INNER, LEFT, RIGHT, FULL); clicar no ícone abre o menu do tipo

### Data Transfer
- [x] **Janela do sistema à parte** (Tools → Data Transfer...), no desenho do original: **origem** (conexão, banco, schema) e **destino**
      (conexão ou arquivo), painéis de informação (servidor, porta e versão), botão de trocar origem e destino, árvore de **objetos**
      (tabelas, views, funções, sequências e tipos, com contagem e marcação em lote), **resumo** e **andamento** com registro
      `[DTF]`/`[ERR]`, contadores e cronômetro, Salvar/Carregar perfil e Opções (estrutura e/ou dados)
      *(`src/components/transfer/`, `src-tauri/src/transfer.rs`, `crates/atlas-pg/src/transfer.rs`)*
- [x] **Modo Advanced por tabela**: nome no destino, campos escolhidos (origem → destino, com ordem), todas as linhas ou **linhas por lote**
      (cada lote numa transação) ou **filtros** `WHERE` (um por recordset, com o **gerador** de faixas e a **prévia do SQL**)
- [x] Motor: estrutura pelo planejador do Dump SQL com seleção objeto a objeto (`DumpScope::Selection`), dados em lotes pelo motor de
      importação (parâmetros com cast; transação por lote ou por tabela), restrições, índices, FKs, views e valores das sequências
      depois dos dados; **entre schemas com nomes diferentes**, entre servidores e entre versões (9.6 ↔ 18: `CREATE SEQUENCE ... AS`
      é adaptado); "apagar antes de criar", "continuar em erro" (senão estrutura e pós-dados são tudo ou nada) e cancelamento que
      desfaz a tabela em andamento; 1,5 milhão de linhas em 14 s
- [x] As janelas do sistema à parte (Data Transfer e Query Builder) nascem ocultas, com o fundo na cor do tema, e só aparecem
      quando a página termina de carregar: sem a tela preta inicial
- [ ] Destino Arquivo só grava SQL (as opções avançadas por tabela não valem nele); outros formatos de arquivo
- [ ] Objetos de bancos em outras versões que o planejador não cobre (partições, identidade e colunas geradas no 9.6, privilégios)
- [x] Escolher uma conexão **fechada** na própria janela: ela abre sozinha (senha do cofre; pede a senha só se não houver ou for
      recusada), lista os bancos e schemas e mostra a versão do servidor; a janela principal não precisa estar com ela aberta
- [ ] A janela principal ainda mostra fechada a conexão que o Data Transfer abriu (o contrário não acontece)

### Importação de dados
- [x] **Assistente de importação em 7 etapas** (formato → arquivos → opções da origem → tabela de destino →
      campos → modo → resumo com log e barra de progresso), vários arquivos/planilhas de uma vez (cada um
      vira uma tabela), navegação `<<` `< Voltar` `Avançar >` `>>`, **Avançado** e perfis salvos
      *(`src/components/import/`, `src-tauri/src/import.rs`, `crates/atlas-import`, `crates/atlas-pg/src/import.rs`)*
- [x] Formatos: **TXT, CSV (delimitador, qualificador, codificação), JSON, XML, Excel (`.xls`/`.xlsx`) e dBase
      (`.dbf`)**; Access, ODBC e Paradox aparecem desabilitados ("ainda não disponível")
- [x] Origem: linha dos nomes, primeira/última linha de dados, ordem/delimitadores de data e hora, símbolo
      decimal, pré-visualização ao vivo; tipos sugeridos pelo conteúdo para tabela nova
- [x] Destino: tabela existente ou **nova** (criada na mesma transação), mapeamento automático por nome,
      campos-chave, colunas identity/geradas tratadas
- [x] **Seis modos**: Acrescentar, Atualizar, Acrescentar/Atualizar, Acrescentar sem atualizar, Excluir e Copiar;
      INSERT estendido com limite de tamanho, vazio como NULL, continuar em erro (SAVEPOINT por registro) ou
      desfazer a tabela inteira; cancelamento desfaz a tabela em andamento
- [ ] Importar de Access/ODBC/Paradox e "Adicionar URL"

### Exportação de dados
- [x] **Assistente de exportação em etapas** (formato → tabelas e arquivos → campos → resumo com log e
      barra de progresso), com **várias tabelas de uma vez** (um arquivo por tabela), seleção em lote
      (tudo / nada / inverter / só tabelas / só views), pasta de destino para todas, **Avançado**
      (opções do formato), navegação `<<` `< Voltar` `Avançar >` `>>` e cronômetro
      *(`src/components/export/`, `src-tauri/src/export.rs`, `crates/atlas-export`)*
- [x] Formatos: **TXT, CSV, JSON, XML, HTML, SQL (`INSERT`), Excel (`.xlsx`), Excel 97-2003 (`.xls`) e dBase
      (`.dbf`)**; só `.accdb` aparece desabilitado ("ainda não disponível")
- [x] `.xls` (BIFF8 em arquivo composto OLE2) em streaming, com os limites do formato (65.536 linhas, 256 colunas:
      acima disso a exportação é recusada sem deixar arquivo parcial); `.dbf` (dBase III) com campos tipados de
      largura fixa (inteiros, decimais com N casas, lógico, data, texto em Windows-1252 de 1 a 254 bytes)
      *(`crates/atlas-export/src/xls.rs`, `dbf.rs`)*
- [x] **Perfis de exportação**: Salvar perfil / Abrir... (formato, opções, tabelas, arquivos e campos),
      guardados no armazenamento local
- [x] Do grid, o assistente já vem com o **filtro, a ordenação e as colunas visíveis** daquela tabela
- [x] Progresso em tempo real, **Parar** (a consulta é abortada no servidor) e nenhum arquivo parcial
      no disco: o arquivo é escrito em `<destino>.part` e só vira o destino ao terminar com sucesso
- [x] Gravação **em streaming**: 3 milhões de linhas (233 MB de CSV) em ~7,5 s com a memória do app
      estável
- [x] CSV/TXT com delimitador, qualificador, codificação (UTF-8, UTF-8 com BOM, Windows-1252),
      quebra de linha e texto para NULL; JSON com tipos nativos (número, booleano, `jsonb` embutido);
      SQL com N linhas por `INSERT`; Excel com cabeçalho, números e booleanos nativos
- [x] **Exportar o resultado de uma consulta** do editor SQL: botão **Exportar** ao lado das abas de
      resultado e "Salvar dados como..." no menu da célula; a consulta é executada de novo em streaming
      (sem o limite de linhas da tela), no schema escolhido, com escolha de campos e, no SQL, o nome da
      tabela de destino; precisa ser um único comando que devolva linhas
- [x] Pontos de entrada: botão **Exportar** do grid, **Salvar dados como...** (menu da célula),
      **Assistente de exportação** no menu da árvore e na barra da lista de objetos

### Views materializadas (PostgreSQL 9.3+)
- [x] Menu da árvore (abrir, projetar, nova, excluir, renomear, exportar, **Atualizar dados** e **Atualizar
      dados (concorrente)**) e pasta com "Nova view materializada"
- [x] **Abrir dados** (grade somente leitura, com filtro/ordenação/colunas/exportação como numa tabela)
- [x] **Designer**: definição (SELECT) com prévia e Explain, dono, comentário, "Preencher com dados" na criação
      e **Atualizar dados** na barra; mudar o SELECT recria a view (`DROP` + `CREATE`) e a confirmação avisa que os
      índices e as views dependentes precisam ser refeitos
      *(`src/documents/ViewDesignerDoc.tsx`, `src/sql/view.ts`, `crates/atlas-pg/src/meta.rs`)*
- Ainda não há: gerenciar os índices da view materializada no designer.

### Funções e procedimentos
- [x] Funções com o **mesmo nome e argumentos diferentes** (sobrecarga) aparecem separadas na árvore e na lista
- [x] **Designer**: nome, parâmetros (modo IN/OUT/INOUT/VARIADIC, tipo, DEFAULT), retorno, linguagem, volatilidade,
      STRICT, SECURITY DEFINER, dono, comentário e corpo com editor SQL; confirmação antes de gravar
      (`CREATE OR REPLACE`; mudar parâmetros ou retorno vira `DROP` + `CREATE`, com aviso; renomear é `ALTER ... RENAME`)
- [x] **Executar** a função/procedimento gravado, com formulário de parâmetros (vazio = DEFAULT ou NULL) e o
      resultado na grade; **Nova função** e **Excluir função** pelos menus da árvore
- [x] Procedimentos (`CREATE PROCEDURE`, PostgreSQL 11+) com `CALL`
      *(`src/documents/FunctionDesignerDoc.tsx`, `src/sql/func.ts`, `crates/atlas-pg/src/meta.rs`)*

### Histórico de consultas
- [x] Cada execução do editor SQL (com êxito ou erro) vai para o banco local: conexão, banco, schema, horário,
      duração, linhas ou mensagem de erro e o SQL como foi digitado
- [x] Botão **Histórico** no editor: busca no texto, filtro por conexão, só favoritas, só com erro; **usar no editor**
      (substitui) ou **acrescentar**, copiar, excluir, **favoritar** e **limpar** (os favoritos ficam)
- [x] Retenção de 2.000 consultas (preferência `history.max`; favoritas nunca saem) e opção **Gravar histórico**
      para desligar (as consultas ficam como digitadas no banco local do computador)
      *(`crates/atlas-local`, `src/components/HistoryDialog.tsx`)*

### Snippets
- [x] Botão **Trecho de código** do editor: gerenciador com lista e filtro, criar, editar, salvar e excluir; **inserir
      no cursor** (duplo clique ou botão) com **pontos de tabulação** (`${1:padrão}`: Tab pula para o próximo, o
      mesmo número em dois lugares preenche os dois); a seleção do editor abre como snippet novo
- [x] 8 snippets de partida (select, insert, update, delete, create table, CTE, explain...) gravados uma única vez
      *(`src/components/SnippetDialog.tsx`, `crates/atlas-local`)*

### Sequências
- [x] Abre pela lista de objetos (barra **Outros** → Sequência): duplo clique ou **Projetar**; **Nova** e **Excluir** pelo
      menu de contexto; sequências novas e existentes no mesmo designer
- [x] **Designer**: nome, tipo (`AS smallint/integer/bigint`, PostgreSQL 10+), incremento, mínimo, máximo, inicial,
      cache, cíclica, **pertence a** (`OWNED BY tabela.coluna`), dono e comentário; mostra o **valor atual e o próximo**
- [x] **Reiniciar** (`RESTART [WITH n]`) e **Definir valor** (`setval`) com confirmação; só o que mudou vai
      no `ALTER SEQUENCE`; compatível com o PostgreSQL 9.6 (lê a própria relação) e 10+ (`pg_sequences`)
      *(`src/documents/SequenceDesignerDoc.tsx`, `src/sql/sequence.ts`, `crates/atlas-pg/src/meta.rs`)*

### Tipos definidos pelo usuário
- [x] A lista **Outros → Tipo** mostra enum, composto, domínio e faixa do schema; duplo clique abre o designer; **Novo**
      e **Excluir** (`DROP TYPE` / `DROP DOMAIN`) pelo menu de contexto
- [x] **ENUM**: acrescentar valores (`ADD VALUE`, fora de transação como o PostgreSQL exige antes do 12) e renomear
      (`RENAME VALUE`, 10+); **composto**: adicionar, remover, renomear e trocar o tipo de atributos; **domínio**:
      padrão, `NOT NULL` e restrições `CHECK`; renomear, dono e comentário; faixa só para leitura
      *(`src/documents/TypeDesignerDoc.tsx`, `src/sql/typedef.ts`, `crates/atlas-pg/src/meta.rs`)*

### Extensões
- [x] A lista **Outros → Extensão** mostra as extensões instaladas no banco (nome, versão, schema, dono, comentário), a mesma
      em qualquer schema; duplo clique abre o designer; **Nova** e **Excluir** (`DROP EXTENSION`) pelo menu de contexto
- [x] **Nova extensão**: escolhe entre as que o servidor oferece e ainda não estão instaladas (`pg_available_extensions`),
      com versão (padrão se vazio), schema (fixo quando o arquivo de controle da extensão o define) e `CASCADE` (9.6+)
- [x] **Extensão instalada**: atualizar a versão (só as com caminho de atualização, `pg_extension_update_paths`), mover de
      schema (só extensão relocável) e comentário; dono só para leitura
      *(`src/documents/ExtensionDesignerDoc.tsx`, `src/sql/extension.ts`, `crates/atlas-pg/src/meta.rs`)*

### Usuários e papéis
- [x] Botão **Papel/Usuário** da barra: lista os papéis do servidor; duplo clique ou **Editar**; **Novo** e **Excluir**
      (`DROP ROLE`) pelo menu de contexto
- [x] **Designer**: nome, senha, login, superusuário, criar bancos/papéis, herança, replicação, `BYPASSRLS` (9.5+),
      limite de conexões, validade, comentário e **participações** (membro de / membros) com `GRANT`/`REVOKE`;
      a **senha nunca é lida nem aparece na revisão ou na prévia do SQL** (vai ao servidor; na tela, asteriscos)
- [x] Aviso ao renomear: o PostgreSQL apaga a senha do papel renomeado
      *(`src/documents/RoleDesignerDoc.tsx`, `src/sql/role.ts`, `crates/atlas-pg/src/meta.rs`)*

### Gatilhos (aba da tabela)
- [x] Lê o gatilho **completo** do servidor: momento (BEFORE/AFTER/INSTEAD OF), eventos, `UPDATE OF` colunas, `FOR EACH
      ROW/STATEMENT`, `WHEN`, função (schema.nome), argumentos, habilitado e comentário
- [x] **Grava** no banco (antes só valia para tabela nova): criar, alterar (DROP + CREATE do gatilho, mantendo
      comentário e estado), excluir, **habilitar/desabilitar** (`ENABLE/DISABLE TRIGGER`) e comentar; a função é escolhida
      entre as funções `trigger` dos schemas carregados
- [x] Gatilho incompleto (sem nome, evento ou função) é barrado com aviso; gatilhos de restrição e com tabelas de
      transição aparecem só para leitura
      *(`src/documents/TableDesignerDoc.tsx`, `src/sql/ddl.ts`, `crates/atlas-pg/src/meta.rs`)*

### Localizar no banco / schema
- [x] **Localizar no banco de dados...** (menu do banco) e **Localizar no schema...** (menu do schema): procura em
      **nomes** de objetos (tabelas, views, views materializadas, sequências, tipos, funções), **nomes de colunas**,
      **definições** (views, views materializadas e corpo de funções) e **comentários** (de objetos e de colunas)
- [x] Opções: diferenciar maiúsculas, **expressão regular** e restringir ao schema; texto simples vale literalmente
      (`%` e `_` não são curingas); até 500 resultados com aviso de corte
- [x] Cada resultado mostra o trecho em que achou; **duplo clique abre** o objeto (dados, designer da tabela para
      colunas, designer de função, sequência ou tipo), carregando o schema se preciso
      *(`src/components/FindDialog.tsx`, `crates/atlas-pg/src/find.rs`)*

### Dump SQL
- [x] **Despejar arquivo SQL** nos menus do banco, do schema e da tabela, com **Estrutura + dados** ou **Somente
      estrutura**: extensões, schemas, tipos (enum, composto, domínio, faixa), sequências (com `OWNED BY` e valor atual),
      funções e procedimentos, tabelas (incluindo `UNLOGGED`, herança, particionamento, identidade e colunas geradas),
      restrições, índices, views e views materializadas (em ordem de dependência, com `REFRESH`), chaves estrangeiras,
      gatilhos e comentários
- [x] Dados em **`COPY ... FROM stdin`** (rápido, restaura com o `psql`) ou em **`INSERT`** (qualquer cliente); opções de
      `DROP ... IF EXISTS` antes de criar e de incluir donos; tudo lido de **um único instantâneo** consistente
- [x] Em streaming (2 milhões de linhas, 80 MB, em ~2 s), com progresso, cancelamento e escrita em `.part` (sem arquivo
      parcial); validado restaurando o dump no PostgreSQL 18 com os mesmos dados (soma de verificação) e por um teste de
      ida e volta (despejar, restaurar num banco novo e despejar de novo dá o mesmo arquivo)
- Não incluídos: privilégios (`GRANT`), opções de armazenamento, tabelas estrangeiras e agregações
      *(`crates/atlas-pg/src/dump.rs`, `src-tauri/src/dump.rs`, `src/components/DumpDialog.tsx`)*

### Opções
- [x] **Ferramentas → Opções...** com cinco abas: **Geral** (idioma, tema claro/escuro/sistema, legendas da barra), **Editor
      SQL** (tamanho da fonte, linhas mostradas por resultado), **Dados da tabela** (registros por página), **Histórico**
      (gravar, quantas consultas guardar, limpar) e **Dados locais** (caminho e versão do banco local, copiar caminho)
- [x] Valores fora do limite são corrigidos ao sair do campo; **Restaurar padrões**; as opções ficam no banco local
      (`ui.options`) e valem na hora
      *(`src/components/OptionsDialog.tsx`, `src/options.ts`)*

### Navegação / Árvore
- [x] **Layout da área de objetos como na referência**: a aba Objects não desenha mais uma segunda barra vertical ao lado do
      divisor do painel; sem schema ou conexão escolhidos a área fica em cinza e sem a linha sob a barra de botões
      (com itens, a lista continua branca e com a linha) *(`src/styles/shell.css`, `ObjectsView.tsx`)*
- [x] **Filtro da árvore em todos os níveis**: bancos, schemas e objetos são filtrados pelo texto (os pais de quem combina
      continuam visíveis) e o trecho encontrado fica destacado em amarelo; antes só filtrava conexões fechadas e objetos
      *(`buildVisibleTree` em `src/model.ts`, `NavigationPane.tsx`)*
- [x] **Fechar schema** no menu de contexto (colapsa + marca como não-carregado para rebuscar
      na próxima abertura) *(`src/App.tsx`)*
- [x] Ícone do schema fica **cinza** quando fechado *(`src/components/NavigationPane.tsx`,
      `src/icons.tsx`)*

- [x] **Novo schema...** no menu de contexto do banco (cria e recarrega a árvore)
- [x] **Editar banco de dados...** (menu do banco): nome, dono, limite de conexões, "permitir conexões", "usar como
      modelo" (PostgreSQL 9.5+) e comentário; codificação, collation, ctype e tablespace aparecem só para leitura
      (o PostgreSQL não os troca). Aplicar grava direto (`ALTER`/`COMMENT` e, por último, o `RENAME TO`); renomear
      pergunta antes, roda pelo banco inicial da conexão, fecha as sessões do Atlas no banco e, em conexões de
      produção, pede o nome para confirmar. O banco usado pela própria conexão não pode ser renomeado (campo
      bloqueado, com aviso)
      *(`src/components/EditDatabaseDialog.tsx`, `src/sql/dbedit.ts`, `crates/atlas-pg/src/meta.rs`)*
- [x] **Comentário como dica na árvore**: bancos e schemas com comentário no servidor o mostram ao passar o mouse
      sobre o nome (como já era com tabelas, views e colunas); a lista de bancos e a de schemas passaram a trazer o
      comentário e ele é relido ao editar ou atualizar *(`crates/atlas-pg/src/meta.rs`, `src/model.ts`)*
- [x] **Editar schema...** (menu do schema): nome, dono e comentário; Aplicar grava direto (em produção, renomear pede
      o nome para confirmar)
      *(`src/components/EditSchemaDialog.tsx`, `src/sql/dbedit.ts`)*

### Editor SQL / Toolbar da consulta
- [x] **Zoom do editor SQL**: Ctrl + roda do mouse, Ctrl + / Ctrl - e Ctrl 0 (50% a 400%), igual em todos os editores
      e lembrado entre sessões *(`src/editorZoom.ts`)*
- [x] Sem fundo na linha ativa do editor (só o número da linha atual fica em destaque) e **números de linha
      centralizados na calha** inteira, com a coluna de dobrar código fixa (14 px) fora do zoom
      *(`src/components/SqlEditor.tsx`)*
- [x] **Dropdown próprio** (connection/database/schema) no lugar do `<select>` nativo —
      hover/selecionado controlados por CSS (o popup do Chromium pintava cinza escuro no hover
      e ignorava o CSS) *(`src/components/Dropdown.tsx`)*
- [x] Linha amarela removida: autofill do Chromium neutralizado nos dropdowns

- [x] Os seletores de conexão, banco e schema do editor se filtram em cascata (trocar o servidor recarrega
      os bancos) sem piscar o indicador de carregamento
- [x] O seletor de schema **relê a lista do servidor ao abrir** (pega schemas criados fora do app), sem
      indicador de carregamento e sem perder o que a árvore já leu
- [x] O **schema escolhido é aplicado à execução** (`search_path`), então tabelas sem prefixo são
      encontradas; a execução descreve os tipos das colunas só depois de executar, sem derrubar
      transações do usuário

### Visual / Tema
- [x] Cor do checkbox padronizada em **#0277BD** (accent-color global) *(`src/styles/tokens.css`)*
- [x] Borda de foco mais fina (1 px) *(`src/styles/shell.css`)*
- [x] Dropdown: item hover/selecionado em **#0277BD** com texto branco
- [x] Bordas dos controles e divisórias do tema claro em **#dbdbdb** (tokens `--c-border`/`--c-border-strong`)
- [x] Campo **Default** do designer de tabela com lista própria (`ComboInput`), com a seta alinhada à borda
- [x] Campo da página do rodapé em azul-claro
- [x] Dropdowns da consulta com borda cinza (fundo branco mantido)

### Identidade visual
- [x] Logo e ícones do Atlas (janela, barra de tarefas, instalador), logo na janela **Sobre** e favicon
      *(`branding/`, `src-tauri/icons/`, `scripts/brand-assets.py`)*
- [x] **Tela de abertura (splash)** com barra de progresso e texto animados (pt/en), tempo mínimo de 3 s
      e só exibida depois de carregada *(`public/splash.html`, `public/splash.js`, `src-tauri/src/lib.rs`)*

### i18n
- [x] Novas chaves `data.find.*`, `dtp.*`, `grid.generated`, `cmd.closeSchema` (pt + en)

### Data Synchronization
Tools → Data Synchronization abre uma janela do sistema (como o Data Transfer): origem/destino com conexão que abre sozinha,
grade de tabelas com tabela de destino, mapeamento de chave e de campos, modos Comparar e visualizar / Comparar e implantar.
O resultado mostra inserir/atualizar/excluir/iguais por tabela, marcação por categoria e por linha, filtro e painéis
origem × destino; depois vêm o Script de implantação (copiar), a implantação (transação, continuar em erro, savepoints) com
o Registro, perfis salvos e opções padrão. Testado ao vivo entre PG 9.6 e PG 18 (comparação, script, implantação e nova
comparação sem diferenças).

### Structure Synchronization
Tools → Structure Synchronization abre uma janela do sistema (como o Data Transfer): origem e destino com conexão que abre
sozinha e o painel Information. A comparação cobre tabelas (colunas, restrições, índices, gatilhos e as opções da tabela),
views, views materializadas, sequências, funções/procedimentos e tipos, e mostra a árvore agrupada por operação ou por tipo,
com caixa de marcação por objeto, grupo e parte, o DDL dos dois lados com as linhas diferentes realçadas e a prévia do script.
O script sai em fases (derrubar dependentes, tipos/sequências/funções, tabelas, restrições e índices, chaves estrangeiras,
views, exclusões), com a opção de reordenar ou desligar comandos, copiar, abrir numa aba de consulta, perfis salvos,
"Continuar em caso de erro" e "Incluir o comando no registro". Entre versões diferentes (9.6 ↔ 18) as definições se comparam
sem espaços, parênteses e qualificadores, `EXECUTE PROCEDURE`/`FUNCTION` e `AS tipo` das sequências se ajustam ao destino.
Testado ao vivo nos quatro sentidos (9.6→9.6, 18→18, 9.6→18, 18→9.6): comparar, implantar e comparar de novo sem diferenças.
Também sincroniza as opções de armazenamento da tabela (`reloptions`, `LOGGED`/`UNLOGGED`), tipos compostos
(`ADD/DROP/ALTER ATTRIBUTE`), domínios (`ALTER DOMAIN`), tabelas estrangeiras, colunas geradas (`SET EXPRESSION` no PostgreSQL 17,
`DROP EXPRESSION` ou recriar a coluna nos demais) e, nas Options e desligados por padrão, donos (`OWNER TO`) e privilégios
(`GRANT`/`REVOKE`, com `GRANT OPTION` e o padrão do servidor para funções e tipos novos).
Fora desta entrega: privilégios por coluna, opções de armazenamento de índices e views, e a mudança do tipo base de um domínio e do
subtipo de um tipo faixa (o script avisa por comentário).

## Limitações conhecidas
- Exportar o resultado de uma consulta exige um único comando que devolva linhas.
- Exportação de tabela sem filtro/ordenação usa a ordem natural do servidor.

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
- [x] **Filtro avançado da árvore**: o funil ao lado da busca abre o menu **Contém** / **Começa com** e **Diferenciar maiúsculas**
      (lembrado entre sessões); o destaque em amarelo segue a regra escolhida e a barra de status conta os itens que o filtro
      esconde (`11 Databases (2 items hidden)` vira `2 Databases (11 items hidden)`); com um banco selecionado mostra os schemas
      *(`filterRange`/`makeMatcher` em `src/model.ts`, `NavigationPane.tsx`)*
- [x] Find in Database / Find in Schema (ver "Localizar no banco" em Entregues)
- [ ] Console do servidor *(item `cmd.console` sem handler)*
- [ ] Share *(`cmd.share` sem handler)*
- [x] Edit Database / Edit Schema (ver "Navegação / Árvore" em Entregues)

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
- [x] Begin Transaction manual (ver "Grid de dados" em Entregues)
- [x] Cell Editor expandido para texto grande / JSON (painel inferior com textarea, quebra de
      linha, "Formatar JSON" e "Definir como NULL"; aberto pelo botão da toolbar — liga/desliga —
      ou pelo menu de contexto da célula; sincroniza em tempo real com a célula ativa)
      — BLOB e XML formatado ficam para depois *(`src/components/CellEditorPanel.tsx`)*
- [ ] Visualizações alternativas: Image view, Hex view, Web view *(Form view já existe)*
- [ ] Edição de BLOB *(hoje só mostra "(BLOB) N bytes")*
- [ ] Modo "mostrar valor da FK em vez da chave" *(lookup já existe)*
- [ ] Agrupamento / sumário no rodapé do grid

## 4. Editor SQL
- [x] Snippet manager (ver "Snippets" em Entregues)
- [ ] Ask AI *(`query.askAi` sem handler)*
- [ ] Split horizontal/vertical *(`src/documents/QueryDoc.tsx:292-293`)*
- [x] Histórico de consultas e favoritos (ver "Histórico de consultas" em Entregues)
- [ ] Transaction panel, Parameters window
- [ ] Beautify / Minify além de "Formatar"
- [ ] Plano de execução gráfico *(hoje `EXPLAIN` só em texto)*

## 5. Designers
- [x] Trigger designer completo (aba Gatilhos da tabela; ver "Gatilhos" em Entregues)
- [ ] Rule designer *(PostgreSQL)*
- [ ] Partition designer
- [ ] Inherits browser *(`src/documents/TableDesignerDoc.tsx:617`)*
- [ ] Preview de dados no designer

## 6. Tipos de objeto não implementados
- [x] Materialized View: designer, abrir dados, atualizar dados (ver "Views materializadas" em Entregues)
- [x] Function / Procedure designer (ver "Funções e procedimentos" em Entregues)
- [ ] Debugger de PL/pgSQL
- [x] Type (ENUM, composto, domínio): designer (ver "Tipos" em Entregues)
- [x] Sequence: designer, reiniciar, definir valor (ver "Sequências" em Entregues)
- [ ] Event Trigger (gatilhos de evento do banco; os gatilhos de tabela já estão no designer)
- [x] Extension: lista, instalar, atualizar, mover de schema e remover (ver "Extensões" em Entregues)
- [ ] Foreign Server / Foreign Table
- [ ] Publication / Subscription (replicação lógica)
- [x] Role / User manager: atributos, senha e participações (ver "Usuários e papéis" em Entregues); o gerenciador de
      privilégios por objeto segue em aberto
- [ ] Tablespace

## 7. Importar / Exportar
- [x] Import Wizard em etapas: TXT, CSV, JSON, XML, Excel e DBF (ver "Importação de dados"); Access/ODBC/Paradox pendentes
- [x] Export Wizard de tabelas/views em etapas, com várias tabelas e perfis (ver "Exportação de dados")
- [ ] Formatos que faltam: Access (`.accdb`; não há biblioteca livre que o grave, depende do driver do Windows), Markdown
- [ ] Exportação agendada (a estrutura junto com os dados já sai pelo Dump SQL)
- [x] Dump SQL completo (ver "Dump SQL" em Entregues)
- [x] Data Transfer entre conexões e bancos (ver "Data Transfer" em Entregues)
- [x] Data Synchronization entre conexões e bancos (ver "Data Synchronization" em Entregues)
- [x] Structure Synchronization entre conexões e bancos (ver "Structure Synchronization" em Entregues)

## 8. Backup / Restore
- [x] **Novo backup do schema ou do banco inteiro** (menu do banco, do schema e categoria Backup) no formato da referência: abas **Geral**
      (servidor, banco/schema e comentário), **Seleção de objetos** (tabelas, views, views materializadas e sequências por
      nome, com os schemas de todo o banco no backup do banco; funções e tipos entram sempre), **Avançado** (nome do arquivo por padrão `%Y%m%d%H%M%S`) e **Log de mensagens**
      (objetos, objetos e registros processados, tempo, saída do `pg_dump` e barra de progresso), com cancelamento
- [x] Os backups ficam guardados por conexão/banco/schema na pasta de dados do app (`.dump` + comentário), ou na pasta
      escolhida em **Avançado → Salvar o backup em** (o app registra o arquivo e ele continua na lista); aparecem na categoria
      **Backup** (nome, data, tamanho, comentário), inclusive com só o banco selecionado (backups do banco inteiro, marcados
      "(banco inteiro)", aparecem em todos os schemas dele); **Excluir backup** na lista (só apaga backups registrados)
- [x] **Restaurar backup** (num banco da conexão, com excluir antes de criar, sem donos/privilégios, transação única) e
      **Extrair SQL** de um backup; aceita também arquivo `.dump`/`.tar` de fora (`pg_restore`) e script `.sql` (`psql`)
- Usa o `pg_dump`/`pg_restore`/`psql` do PostgreSQL instalado (procura no PATH e nos locais usuais; pasta configurável no
  diálogo); **não vão embutidos no app**. Ainda não há: perfis (**Save Profile**), "Lock all tables", escolher funções/tipos
  individualmente (o `pg_dump` não permite), total de registros exato (usa a estimativa das tabelas), objetos globais
  (roles) no backup do banco e teste automatizado contra um servidor (só foi verificado que compila)
      *(`src-tauri/src/backup.rs`, `src/components/BackupDialog.tsx`, `RestoreDialog.tsx`)*
- [ ] Backup agendado

## 9. Automação *(fora de prioridade neste ciclo)*
- [ ] Batch jobs (encadear consultas, backups, transferências)
- [ ] Agendamento cron-like
- [ ] Notificações por e-mail

## 10. Modelagem (Model)
- [ ] ER Diagram de **modelo** (criar, importar via "Reverse Schema to Model"); o modo Exibir → ER Diagram da área de
      objetos está na seção 14
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
- [x] Query Builder visual (ver "Query Builder" em Entregues)
- [ ] Pivot Table
- [ ] Charts a partir de consulta
- [x] Tema claro/escuro manual (menu Exibir e tela Opções; padrão: seguir o sistema)
- [x] Tela de Preferências / Options (ver "Opções" em Entregues)
- [ ] Atalhos de teclado configuráveis
- [ ] Fullscreen por painel *(vários botões `fullscreen` desabilitados)*

## 13. Toolbar / rodapé — itens menores
- [ ] Conta (login do serviço em nuvem) *(`src/components/MainToolbar.tsx:100`)*
- [ ] Beautify de célula
- [ ] Hex / Binary editor próprio

## 14. Área de objetos / menu Exibir
Hoje o menu Exibir tem só Lista, Detalhe e Ícones (`viewMode` em `src/App.tsx`). O que falta para igualar o da referência:
- [x] **ER Diagram** como modo de exibição da área de objetos (Exibir → ER Diagram, ou o botão na barra de status; vale
      para a lista de tabelas de um schema): desenha as tabelas do schema como cartões (colunas com tipo e ícone de
      **PK** e **FK**) e as chaves estrangeiras como linhas com cotovelo, com o nome da FK sobre elas. Os cartões se
      arrastam (a posição é lembrada por schema), entram já arrumados (tabelas ligadas ficam vizinhas) e o canvas rola;
      clicar seleciona (os botões Abrir/Projetar tabela e o menu de contexto valem), duplo clique abre a tabela.
      **Clicar numa linha** seleciona a chave estrangeira: a linha engrossa com bolinhas nas pontas, os dois cartões
      ganham contorno e as colunas da chave ficam destacadas; passar o mouse mostra o nome da FK e a tabela que a
      referencia ("Referenciando: ..."). Os rótulos levam o ícone de corrente e ficam ao lado da linha, perto da tabela
      que tem a chave, e dizem se a referência é **obrigatória** (todas as colunas da chave são `NOT NULL`) ou
      **opcional** (alguma aceita nulo); a dica repete. Com a linha selecionada, uma **alça** no meio dela (ou arrastar a própria linha) **reposiciona**
      o trecho do meio: a parte vertical do cotovelo ou a altura/posição da linha reta, sempre entre os dois cartões;
      o caminho fica salvo por schema e o **duplo clique** na linha volta ao automático. Arrastar a **bolinha de uma
      ponta** mostra os 4 pontos de ligação do cartão (topo, direita, base, esquerda) e a linha passa a sair/entrar pelo
      mais próximo, refazendo-se ao vivo; o lado escolhido também fica salvo. A rota não desvia de outros cartões que
      estejam no caminho. As
      opções **Show Grid**, **Snap to Grid** e **Show Foreign Key Names** ficam no menu Exibir e são lembradas; o botão
      Atualizar relê o schema. Os dados vêm de uma consulta só (`pg_schema_diagram`); sem servidor, usa as tabelas da
      demonstração. Não é a Modelagem da seção 10: aqui é a visão do schema como ele está no banco
      *(`src/components/ErDiagram.tsx`, `src/er.ts`, `crates/atlas-pg/src/meta.rs`)*
- Ainda não há no ER Diagram: zoom, ferramentas de ligar (criar chave estrangeira pelo desenho), Preview e Size da barra
  inferior da referência, chaves para tabelas de outro schema (não têm cartão), tabelas com mais de 40 colunas mostram as
  40 primeiras e a linha "mais N colunas"
- [ ] **Flatten Object List**: lista única dos objetos do schema, sem as pastas
- [x] **Sort**: ordenação da lista de objetos por qualquer coluna, crescente ou decrescente (também pelo clique no cabeçalho)
- [x] **Choose Columns**: escolher as colunas exibidas na lista de objetos (por categoria; o nome nunca some)
- [x] **Show Hidden Items**: mostra os bancos que a conexão oculta (templates e sem permissão de conexão) e os schemas de sistema; relê a árvore ao alternar
- [ ] **Show Table Profiles**: perfil de cada tabela (resumo) na lista de objetos
