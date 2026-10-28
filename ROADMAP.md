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

## Entregues neste ciclo (até 2026-10-24)

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
- [ ] Destino Arquivo só grava SQL *(as opções avançadas por tabela não valem nele; **outros formatos de arquivo não estão confirmados na referência**: a captura mostra só o rádio "File", sem os formatos, então não foram feitos)*
- [ ] Objetos de bancos em outras versões que o planejador não cobre (partições, identidade e colunas geradas no 9.6, privilégios)
- [x] Escolher uma conexão **fechada** na própria janela: ela abre sozinha (senha do cofre; pede a senha só se não houver ou for
      recusada), lista os bancos e schemas e mostra a versão do servidor; a janela principal não precisa estar com ela aberta
- [x] A janela principal passa a mostrar **aberta** a conexão que o Data Transfer (ou outra janela) abriu: o Rust avisa (`session-opened`) e ela lê bancos e papéis sem
      reabrir a sessão; testado abrindo uma sessão sem passar pela janela principal (o ícone da conexão muda e as outras seguem fechadas)

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
- [ ] Importar de Access, ODBC e Paradox *(**deixados de fora por enquanto**, por decisão: são formatos antigos e dependem de driver do Windows; o caminho é converter para CSV fora do Atlas)*
- [x] Importar por URL: botão "Adicionar URL..." no assistente (http/https, até 200 MB, baixa para a pasta temporária e segue como arquivo local; a lista mostra o endereço)

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
- [x] **Aba "Find in Database/Schema"** (Tools → Find in Database..., e o menu do banco e do schema na árvore, que já abrem com o
      escopo marcado), no desenho da referência: painel à esquerda com **Conexão, Banco e Schema** (schema vazio = o banco todo),
      **Find what**, **Look In** (Data ou Structure), **Search Mode** (Contains, Whole Word, Prefix, Regular Expression),
      **Case Insensitive** (marcado por padrão; desmarcado diferencia maiúsculas) e **Include Objects** (Tables, Views, Functions, Queries, Indexes, Triggers, Materialized Views);
      resultado à direita e barra de status "Found N objects with M occurrences"
- [x] **Structure**: árvore por objeto, "client_notes (3)", com o que casou em cada um (Table Name, Field Name, Foreign Key Name,
      Index Name, Trigger Name, View Body, Function Name, Function Body...) e o trecho achado em vermelho; o corpo de views e
      funções gera **uma linha por ocorrência**, com o trecho ao redor; reproduz o exemplo da referência (5 objetos, 10 ocorrências)
- [x] **Data**: quantas linhas de cada tabela têm o texto em alguma coluna (lida como texto), com "Number of Matched Records";
      `\` como escape (`%` e `_` valem como curingas no LIKE), botão **Stop** que interrompe a consulta no servidor e aviso das
      tabelas sem permissão; **duplo clique numa tabela abre uma consulta nova, já executada, com o SELECT e a condição do texto
      procurado** (uma condição `ILIKE '%texto%'` por coluna) e o registro listado, como na referência
- [x] Duplo clique num achado abre o objeto (dados, designer da tabela para campo, designer de função); testado no PG 9.6 e 18
      *(`src/documents/FindDoc.tsx`, `crates/atlas-pg/src/find.rs`)*
- Limites: a caixa **Queries** aparece desabilitada (as consultas salvas ainda não são objetos para procurar); os índices que
  sustentam a chave primária ou restrições não entram em "Index Name" (a referência também não os lista); a busca em dados lê
  todas as tabelas do escopo, então em bancos grandes use o Stop

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
- [x] **Ferramentas → Opções...** no desenho da referência: lista de categorias à esquerda e o conteúdo à direita. Categorias com
      efeito real: **General** (idioma, tema, legendas da barra, permitir abrir várias abas do mesmo objeto), **Code Completion**
      (usar completar código, selecionar a primeira sugestão), **SQL Editor** (números de linha, dobra de código, destaque de
      parênteses e de sintaxe, quebra de linha, largura do Tab, espaços no Tab, tamanho da fonte, linhas por resultado),
      **Table Data** (limitar registros ao abrir, registros por página, sombreamento das linhas: nenhum, a cada 2 ou a cada 3),
      mais **History** e **Local Data** do Atlas; todas valem na hora, sem reiniciar
- [x] Valores fora do limite são corrigidos ao sair do campo; **Restaurar padrões**; as opções ficam no banco local (`ui.options`)
      *(`src/components/OptionsDialog.tsx`, `src/options.ts`)*
- [x] **SQL Editor → Font and Colors**: fonte do editor (lista de fontes comuns ou qualquer instalada) e cores de Common, Keyword, String,
      Number, Comment e Background, com botão para voltar à cor do tema; valem na hora, também nos temas claro e escuro
- [x] **Table Data → Display Format**: padrões de **Date**, **Time** e **DateTime** (`yyyy yy MM dd HH hh mm ss tt f..`), com a prévia ao vivo, e
      **separador de milhar** / separador decimal do sistema; só a exibição muda (copiar, editar e exportar usam o valor real) *(`src/format.ts`)*
- [x] **Tabs → On Startup** (abrir só a aba Objects, ou **continuar de onde parou**: reabre as consultas com o texto, a conexão, o banco e o
      schema) e **Auto Recovery** (guardar o texto das consultas abertas a cada N segundos, 30 por padrão); a lista de abas também é
      gravada a cada mudança e ao fechar *(`session.tabs` nas preferências locais)*
- [x] **Continuar de onde parou também reabre tabelas, views e views materializadas**, presas ao **banco**: abrir só a conexão não traz nada; ao abrir o banco elas voltam (na aba que estava ativa).
      Fechar o banco ou a conexão fecha as abas de tabela dele (pergunta antes se há alterações não gravadas) mas as guarda para voltarem quando o banco abrir de novo; só o **X** da aba faz a tabela ser esquecida
- [ ] Da referência sem equivalente no Atlas: Tabs → "Open new tab in" (o Atlas tem uma só janela de abas), Connectivity (proxy: na referência vale
      só para ativação, atualizador, nuvem e IA, que o Atlas não tem), File Locations (o Atlas guarda tudo numa pasta só, mostrada em "Local Data"),
      Environment (SQL*Plus, Mongo, OCI) e AI. Advanced (log de diagnóstico, várias instâncias, registrar "Abrir com" e protocolo URI) também fica de fora: são integrações com o Windows que não dá para testar aqui e não têm função no Atlas
- [x] **Records → Grid Font** (fonte e tamanho da grade; a largura automática das colunas acompanha), **Auto begin transaction** (a tabela abre já dentro de uma transação, com Commit/Rollback) e **General → perguntar antes de fechar uma consulta com texto não guardado** (desligado, fecha direto; com transação aberta sempre pergunta) *(`src/options.ts`)*
- Padrão do sombreamento: a cada 2 linhas (a referência vem com "Every Third Row")

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

### Data Generation
Tools → Data Generation abre uma janela do sistema (como o Data Transfer): destino (conexão, banco e schema, com a conexão abrindo
sozinha), árvore de tabelas e campos com a quantidade de linhas por tabela (1000 por padrão) e o gerador de cada coluna,
"Table Generation Order" (as tabelas referenciadas saem na frente), prévia de 100 linhas com "Regenerate", execução com
registro (`[DGR]`) e contadores (objetos, registros gerados e inseridos, erros, tempo), perfis salvos e as opções
"Continuar em caso de erro", "Esvaziar as tabelas antes", "Usar transação" e "Vários comandos de inserção".
O gerador sugerido sai do tipo da coluna, da chave e do padrão: sequência (que continua do maior valor existente e deixa a
sequência serial no lugar certo), inteiro, decimal (respeitando precisão e escala), texto (frases, palavras ou letras, dentro do
tamanho da coluna), data, data e hora, booleano, UUID, lista (os rótulos de um enum), valor fixo, nulo e referência (um valor que
já existe na tabela da chave estrangeira, então as linhas filhas nunca ficam órfãs); colunas que aceitam nulo ganham a
porcentagem de nulos. Testado ao vivo em 9.6 e 18 (descrever, prévia, gerar, falha com e sem "continuar", esvaziar antes).
Fora desta entrega: chaves estrangeiras compostas (cada coluna sorteia o valor sozinha), tipos sem gerador próprio (que saem
nulos ou vazios até o usuário escolher um) e o painel de propriedades por coluna, que foi desenhado sem captura da referência.

### Data Dictionary
Tools → Data Dictionary abre uma janela do sistema com as cinco etapas da referência: (1) árvore de conexões, bancos e schemas
com caixas de marcação e o painel de informações do item (conexões fechadas abrem ao expandir); (2) objetos por schema (Server /
Database / Schema Information, Tables, Views e Functions, com escolha item a item) e a ordem de servidores, bancos e schemas
(setas e busca); (3) cinco modelos de capa (laranja, vermelho, preto e branco, azul e verde); (4) layout com as abas Capa,
Sumário, Cabeçalho/Rodapé, Conteúdo e Papel, com a pré-visualização da página ao lado (zoom, página inteira) e os
"Restore Defaults"; (5) arquivo de saída (data e hora no nome, sobrescrever, continuar em erro). A geração mostra os
contadores, o log `[DDI]`, a barra de progresso e o botão Abrir (arquivo ou pasta).
O PDF é escrito pelo novo crate `atlas-docs`, sem dependências: capa, sumário numerado com pontilhado, introdução com a
lista de servidores, e por tabela os campos (chave, Auto Increment, Not Null, padrão), índices, chaves estrangeiras, checks e
gatilhos; views e funções com a definição; cabeçalho com o caminho, rodapé e número de página; papéis A3, A4, A5, Letter e Legal,
retrato ou paisagem e margens. A tela e o PDF usam o mesmo desenho (a prévia é o SVG da mesma página).
Fora desta entrega: imagem própria de fundo e logotipo da capa, fontes incorporadas (o PDF usa as padrão: Helvetica, Times e
Courier, então caracteres fora do Latin-1 saem como "?") e a prévia do conteúdo real nas abas de layout (usa uma tabela de exemplo).

### Server Monitor
Tools → Server Monitor → PostgreSQL... abre uma aba "Server Monitor - PostgreSQL" (reaproveitada se já existir) com a lista de
conexões e caixas de marcação: marcar uma conexão fechada abre a conexão, e dá para acompanhar vários servidores ao mesmo tempo.
Três abas: **Process List** (Server, Id, User, Host, Port, db, Command, State com o modo do bloqueio, Time e Info com o objeto ou a
transação bloqueada, uma linha por bloqueio como na referência), com Refresh, **End Process** (confirma e usa
`pg_terminate_backend`) e Sort Ascending/Descending pela coluna clicada; **Variables** (`pg_settings` com DateStyle, IntervalStyle
e TimeZone primeiro) e **Status** (blocos lidos e acertados em cache, conexões, commits e rollbacks do banco), com uma coluna por
servidor e busca. Atualiza sozinho a cada 5 segundos e a barra de status mostra conexões marcadas e número de processos.
Testado ao vivo em 9.6 e 18 (motor e tela, com os dois servidores juntos). Fora desta entrega: os outros SGBDs do submenu da
referência e o gráfico de uso, que a referência não mostra.

### History Log
Tools → History Log (Ctrl+L) abre uma aba "History Log" que mostra tudo o que o Atlas manda aos servidores, no formato da
referência: `[2026-10-23 16:56:45.377][PostgreSQL 9.6][32600][PGSQL]`, o comando e `Time: 0.003s`, com o erro logo depois
(`ERROR:`, `LINE n:` com o cursor, `DETAIL:` e `HINT:`). Um cliente com registro (`LoggedClient`) no lugar do cliente do driver
anota cada comando (consultas, DDL, DML, transações, resultados do editor e exportações, inclusive as consultas de catálogo da própria
árvore), em memória, no máximo 4000 entradas e 8 MB. Senhas em `PASSWORD '...'` saem mascaradas e comandos enormes são cortados.
A aba tem Clear, Word Wrap, Show Errors Only (o comando que falhou e o erro), Pause, busca, numeração de linhas, rolagem
automática no fim e "Last Refresh Time" na barra de status. O menu Tools passou a seguir a ordem da referência, com Find in
Database/Schema.
Testado ao vivo em 9.6 e 18 (registro, erro com posição, senha mascarada e pausa) e na tela.
Fora desta entrega: a gravação em arquivo e as inserções em lote do import e do Data Transfer, que usam instruções preparadas e
não aparecem no registro.

### Console
Tools → Console (F6) e o menu de contexto da conexão e do banco abrem a aba "PostgreSQL 18 - Console", um terminal no estilo do
`psql` com o prompt `atlas_demo=#` (`>` para quem não é superusuário, `-#` nas linhas de continuação). O comando só roda quando
termina em `;` fora de aspas, `$tag$` e comentários, então dá para digitar em várias linhas; os comandos com `\` rodam na hora.
O texto sai do Rust, formatado como o `psql`: tabelas alinhadas com cabeçalho centralizado e números à direita, `(n rows)`,
etiquetas (`INSERT 0 1`, `CREATE TABLE`...), erros com `LINE n:` e o cursor, `DETAIL:` e `HINT:`. Comandos: `\?`, `\q`, `\c banco`
(troca de verdade o banco da aba), `\conninfo`, `\l`, `\dn`, `\dt`, `\dv`, `\dm`, `\di`, `\ds`, `\df`, `\du`, `\d nome` (colunas,
índices, checks, chaves estrangeiras, "Referenced by" e gatilhos), `\timing` e `\x` (saída expandida). Setas para cima e para baixo
percorrem o histórico e Ctrl+C descarta o que está sendo digitado. Tudo o que o console executa aparece no History Log.
Testado ao vivo em 9.6 e 18 (SQL, etiquetas, erro, `\dt`, `\d`, `\l`, `\c`, `\timing`, `\x`) e na tela.
Também tem: `\g` e `\gx` (terminam o comando no lugar do `;`), `\watch [segundos]` (repete o último comando até o Ctrl+C), `\i arquivo`,
`\copy` de e para arquivo local (`tabela [(colunas)]` ou `(consulta)`, com as opções do `COPY`, e a contagem do servidor), variáveis
(`\set`, `\unset`, `\echo` e `:nome`, `:'nome'`, `:"nome"` fora de aspas, comentários e `$$`), as variantes com `+` (`\d+`, `\dt+`, `\l+`,
`\dn+`, `\du+`) e o Tab: completa comandos com barra, palavras-chave (no caso de quem digitou), schemas, tabelas, views, funções e
colunas (`tabela.` e `schema.tabela.`), pelo prefixo em comum ou listando as opções.
Fora desta entrega: `\copy` de/para stdin e stdout, `\o` (saída para arquivo), `\ir`, variáveis especiais (`ON_ERROR_STOP`...),
comandos `\` dentro de um arquivo do `\i` e o autocompletar dentro de subconsultas com alias.

### Foreign Server
Others → Foreign Server lista os servidores externos do banco (nome, wrapper, dono e comentário) e o designer cria e altera:
nome (com `RENAME`), wrapper (escolhido entre os instalados no banco; não muda depois), tipo e versão, dono, **opções** (grade
chave/valor), **mapeamentos de usuário** (um por usuário ou `PUBLIC`, cada um com as próprias opções; o valor de uma opção de
senha aparece mascarado) e comentário, com a prévia do SQL (`CREATE SERVER`/`CREATE USER MAPPING` ou só o que mudou em
`ALTER SERVER ... OPTIONS (ADD/SET/DROP)` e `ALTER USER MAPPING`). Excluir pelo menu de contexto remove antes os mapeamentos do
servidor e continua recusando se houver tabela externa que o usa. Aparece em "Novo → Outros → Foreign Server...".
Testado ao vivo no PG 9.6 pela tela (criar com opções, mapeamento e senha, reler, renomear, alterar opções, criar mapeamento e
excluir com dois mapeamentos); no teste automatizado a parte do servidor só roda com superusuário (o usuário de teste não é).
Fora desta entrega: privilégios do servidor (`GRANT USAGE ON FOREIGN SERVER`) e a lista de wrappers como objeto próprio.

### Foreign Table
As tabelas externas (`relkind 'f'`) aparecem na lista de Tabelas com ícone próprio (também na árvore). **Design Table** abre o designer
de tabela externa: nome (com `RENAME`), servidor (escolhido entre os do banco; não muda depois), dono, **colunas** (nome, tipo, não
nulo; incluir, remover, renomear, trocar tipo), **opções** (grade chave/valor, a mesma do Foreign Server) e comentário, com a prévia do SQL
(`CREATE FOREIGN TABLE ... SERVER ... OPTIONS` ou só o que mudou em `ALTER FOREIGN TABLE`). Excluir usa `DROP FOREIGN TABLE`; esvaziar,
truncar e duplicar ficam desativados para elas. Abrir a tabela mostra os dados como numa tabela comum. Aparece também em
"Novo → Tabela → Foreign...". Testado ao vivo no PG 9.6 pela tela (listar, alterar coluna/opção, criar, excluir).
Fora desta entrega: partições externas e `IMPORT FOREIGN SCHEMA`.

## Limitações conhecidas
- Exportar o resultado de uma consulta exige um único comando que devolva linhas.
- Exportação de tabela sem filtro/ordenação usa a ordem natural do servidor.

> Itens marcados "não confirmado na referência" saíram de uma lista de funções que um gerenciador de PostgreSQL costuma ter, não de
> uma captura. Só entram na esteira depois que aparecerem numa captura de `capturas/`.

## 1. Conexões
- [x] **Importar / Exportar conexões** (Arquivo): arquivo JSON `atlas-connections` com as conexões, **sem senhas**; ao importar viram
      conexões novas (nome repetido ganha " (2)") e a senha é pedida na primeira vez *(`src-tauri/src/local.rs`)*
- [ ] Compartilhamento de conexões via nuvem
- [x] **URI da conexão**: botão **URI...** no formulário (mostra e aceita `postgresql://usuario:senha@host:porta/banco?sslmode=...`; a senha não
      aparece de volta) e **Nova conexão com URI...** no primeiro passo; `allow`/`prefer` valem como sem TLS *(`src/connectionUri.ts`)*
- [x] **Aba Databases** do diálogo de conexão: "Use custom database list", adicionar/remover banco da lista e **Auto Open**; com a lista
      ligada só esses bancos aparecem na árvore e os marcados abrem ao conectar. A aba **Remarks** também passou a ser guardada
      *(`src/components/ConnectionDialog.tsx`, opções do perfil)*
- [x] **SSH tunnel** (aba SSH): senha ou **chave privada** (arquivo + frase), porta local aberta pelo túnel (`crates/atlas-ssh`, cliente `russh`);
      a conexão com o banco, o backup/restore e todos os bancos abertos passam por ela. A **chave do servidor é guardada no primeiro uso** e uma
      chave diferente depois bloqueia a conexão (`ssh_known_hosts` na pasta dos dados); a senha/frase fica no **cofre** (`<id>:ssh`), nunca no perfil
      nem em arquivo. Testado com um servidor SSH em processo (5 testes: senha, chave, chave nova/alterada, erro, vários clientes) e pela tela
      (teste de conexão, salvar, reconectar com a senha do cofre, segundo banco). `cargo run -p atlas-ssh --example dev_server` sobe um servidor
      SSH só para desenvolvimento. Limite: com `verify-full` o nome do host do certificado é conferido contra `127.0.0.1`
- [ ] HTTP tunnel *(não implementado: na referência ele só funciona com um script PHP instalado no servidor web, com protocolo próprio do fornecedor;
      sem esse servidor não há como conectar, então não há aba HTTP no diálogo para não prometer o que não funciona)*
- [ ] Autenticação Kerberos/GSSAPI *(**não viável agora**: o driver PostgreSQL usado não tem GSS/SSPI e fazer à mão exigiria um ambiente Kerberos para testar, que não temos)*
- [x] **Grupos (pastas) de conexão na árvore**: Novo grupo (menu da raiz ou do grupo), Renomear, Excluir (as conexões voltam para fora de
      grupo), **Mover para o grupo** no menu da conexão (e novo grupo na hora) e Nova conexão já dentro do grupo; grupos vazios e o grupo de
      cada conexão ficam guardados; a busca e o filtro de estrela mostram só os grupos que têm conexão visível. Ainda sem arrastar e soltar
> **Vários bancos.** Regra: o fluxo do PostgreSQL está validado e **não muda**; cada banco novo entra ao lado (crate e comandos `<banco>_*` próprios; as telas chamam os mesmos `pg_*` e o roteador de `src/providerRoute.ts` manda a chamada ao banco da conexão, pelo id dela). Ordem: PostgreSQL (feito) → **SQLite** → SQL Server → MySQL → Firebird → MariaDB → Oracle (fatia vertical feita, ver abaixo) → MongoDB e Redis. Testes dos outros bancos em Docker (um de cada vez); o Firebird já roda na máquina.
- [x] **SQLite**: conexão por arquivo (formulário já existia), árvore (banco e schema `main`, Tables, Views, Queries), abrir tabela e view (ordem pela chave, filtro, paginação), editar, inserir, excluir e **duplicar registro** (a chave `INTEGER PRIMARY KEY` numera sozinha), tabela sem chave pelo `rowid` (só quantas linhas foram marcadas), booleano como caixa de seleção (`0`/`1` no arquivo), consulta SQL com vários comandos, erro de sintaxe com posição, cancelar, **transação manual** (Begin Transaction, Commit, Rollback), editor com o dialeto SQLite, chaves estrangeiras ligadas *(`crates/atlas-sqlite`, `src-tauri/src/sqlite.rs`, `src/providerRoute.ts`, `src/sql/sqlitePlan.ts`)*
- [x] **Auto incremento no designer de tabela** (caixa "Auto Increment" na grade de campos; só tipos inteiros; marca não nulo e limpa o padrão; vale na criação e na alteração, ligando e desligando). PostgreSQL: 10 em diante `GENERATED BY DEFAULT AS IDENTITY`, 9.6 `serial`/`bigserial` (coluna existente: sequência própria + `setval` pelo maior valor); validado ao vivo no 9.6 e no 18 *(`src/sql/ddl.ts`, `src/documents/TableDesignerDoc.tsx`)*
- [ ] **Critério de pronto de todo banco novo:** auto incremento feito no designer e testado ao vivo (criar tabela, inserir sem informar a chave, ligar/desligar numa coluna existente quando o banco permite). SQLite `INTEGER PRIMARY KEY AUTOINCREMENT` (chave de uma coluna inteira), SQL Server `IDENTITY(1,1)`, MySQL/MariaDB `AUTO_INCREMENT`, Firebird `GENERATED BY DEFAULT AS IDENTITY`
- [x] **SQLite — designer de tabela** (nova tabela e alterar): abas Fields, Indexes, Foreign Keys, Uniques, Checks e SQL Preview; tipos do SQLite; **auto incremento** = chave única de uma coluna `integer` (`INTEGER PRIMARY KEY AUTOINCREMENT`; desmarcar troca o tipo para `int`, porque `integer` com chave sempre numera). Coluna nova simples sai por `ALTER TABLE ADD COLUMN`, renomear por `RENAME COLUMN`, índices por `CREATE/DROP INDEX`; qualquer outra mudança **refaz a tabela** (cria, copia os dados, apaga, renomeia; views, chaves de outras tabelas e gatilhos seguem valendo). Os `CHECK` passam a ser lidos do `CREATE TABLE`. Validado ao vivo: criar com auto incremento, ligar e desligar numa tabela com dados *(`src/sql/sqliteDdl.ts`, `crates/atlas-sqlite`)*. Limites: tabela `WITHOUT ROWID` e colunas geradas perdem essa característica ao refazer; gatilhos não se editam
- [x] **SQLite — rótulos de chave estrangeira, Data Profiling, Table Profile e Find in Database** (estrutura: tabelas, campos, índices, gatilhos, views e corpo das views; dados: `REGEXP` definida em toda conexão, também vale nas consultas do usuário) *(`crates/atlas-sqlite/src/{profile,find}.rs`)*
- [x] **SQLite — exportação** (assistente de exportação e consulta: CSV, TXT, JSON, XML, HTML, SQL, Excel, DBF; números e booleanos tipados; conexão só de leitura em streaming; cancelável)
- [x] **SQLite — importação** (os seis modos, tabela nova com os tipos inferidos, continuar em erro por linha, cancelável; conexão própria e uma transação por importação; booleano `0`/`1`)
- [x] **SQLite — backup e restauração** (cópia compactada do arquivo com `VACUUM INTO`, com seleção de objetos, comentário, pasta e padrão de nome; lista na categoria Backup; restaurar troca o conteúdo do banco pelo do arquivo; sem as ferramentas do PostgreSQL)
- [x] **SQLite — Dump SQL File e duplicar tabela** (script com `CREATE` + `INSERT` que o `sqlite3` e o Atlas executam de volta; duplicar estrutura e dados com chaves, UNIQUE, CHECK e índices)
- [x] **SQLite — designer de view** (nova view e alterar a definição; o SQLite não tem `CREATE OR REPLACE VIEW`, então alterar apaga e cria de novo; sem dono, comentário nem regras; renomear view não existe no SQLite)
- [x] **SQLite — console** (terminal no estilo do `sqlite3`, resultados alinhados como no `psql`, comandos `.tables`, `.schema`, `.indexes`, `.timing`, `.expanded`; Tab completa tabelas, colunas e palavras-chave) e menus sem os itens do PostgreSQL
- [x] **SQLite, SQL Server e MySQL — ferramentas do menu Tools**: **Server Monitor** (processos, variáveis e contadores; SQLite mostra os `PRAGMA` e os números do arquivo), **Data Dictionary** (PDF com tabelas, views e rotinas), **Data Generation** (geradores do PostgreSQL, `IDENTITY_INSERT` no SQL Server), **Data Transfer** (entre bancos, servidores ou arquivos do **mesmo produto**; estrutura pelo Dump SQL de cada banco, dados em lotes, SQLite por `ATTACH`), **Data Synchronization** (insere, atualiza e exclui pela chave) e **Structure Synchronization** (colunas, chaves, restrições, índices, gatilhos, views e rotinas; o SQLite recria a tabela quando precisa); origem e destino de uma transferência ou comparação são sempre do mesmo produto, como no PostgreSQL
- [x] **Compartilhar** em todos os bancos (o endereço leva o produto: `pgsql`, `mssql`, `mysql`, `sqlite`) e **gatilhos do MySQL** na Structure Synchronization (`SHOW CREATE TRIGGER`)
- [x] **Entre produtos diferentes** (PostgreSQL, SQL Server, MySQL e SQLite, em qualquer par): **Data Transfer**, **Data Synchronization** e **Structure Synchronization**. Um modelo neutro traduz os tipos e os valores (booleano, binário, data e hora, decimais) e escreve o DDL na linguagem do destino; levam estrutura (tabelas, colunas, chave primária, únicas, chaves estrangeiras, índices simples, valores padrão simples e numeração automática), **`CHECK` simples e views com `SELECT` simples** (um tradutor de expressões troca aspas, schema e as funções comuns; o que usa algo do produto — `LIMIT`, `UNION`, janela, função própria — não atravessa e o log avisa) e dados. **Não atravessam** gatilhos e rotinas (`PL/pgSQL`, `T-SQL` e o SQL do MySQL são linguagens de programação diferentes; o SQLite nem tem rotinas) nem a *alteração* de uma view que existe nos dois lados. No SQL Server, trocar a numeração automática de uma coluna recria a tabela copiando os dados e refazendo as chaves que apontam para ela
- [x] Reverso para modelo: **fora do Atlas** *(o que ele faz é o mesmo do ER Diagram, que já existe nos quatro bancos; o item saiu dos menus do banco e do schema)*
- [x] **Todos os bancos — Query Builder com o dialeto de cada um** (aspas e colchetes dos nomes, `TOP`/`OFFSET ... FETCH` no SQL Server, `LIMIT` no MySQL e SQLite, sem `ILIKE` fora do PostgreSQL, sem `FULL JOIN` no MySQL; o texto de volta para o desenho lê crases e colchetes), **Execute SQL File atômico** fora do PostgreSQL (SQL Server separa os lotes `GO`) e **SQLite: criar arquivo novo** na tela de conexão
- [x] **SQL Server (fatia vertical)** — driver `crates/atlas-mssql` (TDS via `tiberius`; uma conexão por banco, `USE` implícito) e comandos `mssql_*` ao lado dos do PostgreSQL. Conexão pela tela de conexões, árvore (bancos, schemas, Tables, Views, Functions/procedimentos, Queries), abrir tabela/view (ordem pela chave, filtro, paginação `OFFSET/FETCH`), editar, inserir e excluir (PK; tabela sem chave com `TOP (n)` e contagem exata), `bit` como caixa de seleção, `IDENTITY` não editável, datas e `decimal` como o servidor mostra, consultas com vários result sets, transação manual, editor com o dialeto T-SQL. **Designer de tabela** (criar e alterar no lugar: colunas, tipos, chave, UNIQUE, CHECK, chaves estrangeiras, índices) com **auto incremento = `IDENTITY(1,1)`** (tipo inteiro, uma coluna por tabela; o SQL Server não liga nem desliga `IDENTITY` numa coluna que já existe, então a tela recusa isso). Validado ao vivo contra o SQL Server 2022 (Docker `atlas-mssql`, porta 1433, variáveis `ATLAS_TEST_MSSQL_*`)
- [x] **SQL Server — rótulos de chave estrangeira, diagrama ER, Find in Database** (estrutura: tabelas, campos, FKs, índices, gatilhos, views e procedimentos com o corpo; dados: `LIKE`/igualdade, sem expressão regular), **exportação**, **importação** (seis modos, tabela nova, continuar em erro por linha com `SAVE TRANSACTION`), **Data Profiling**, **Table Profile**, **designer de view** (`CREATE OR ALTER VIEW`) e **renomear** (`sp_rename`). DDL sem parâmetros roda como lote simples (o `sp_executesql` não aceita `CREATE VIEW`).
- [x] **SQL Server — console** (estilo `sqlcmd`: resultados alinhados, lotes com `GO`, `.tables`, `.schemas`, `.databases`, `.timing`, `.expanded`, Tab) e **duplicar tabela** (estrutura e dados; `IDENTITY_INSERT` quando há identidade)
- [x] **SQL Server - Dump SQL File** (DDL lido do catalogo com IDENTITY, chaves, UNIQUE, CHECK, FKs, indices, views e procedimentos; dados em INSERT com N'...' e IDENTITY_INSERT; lotes com GO; reexecutado num banco novo devolve tudo), **backup e restauracao** (BACKUP/RESTORE DATABASE, arquivo no disco do servidor, lista pelo historico do msdb; o SQL Server nao faz backup de um schema so), **cancelar consulta** (KILL da sessao), **tunel SSH e senha do cofre** (como no PostgreSQL) e **procedimentos e funcoes** (o texto abre numa aba de consulta como CREATE OR ALTER; novo e excluir)
- [x] **SQL Server — excluir backup** (apaga o arquivo no servidor com `xp_delete_file` e o registro no `msdb`), **extrair SQL de um backup** (restaura num banco temporário, grava o script e apaga o banco) e **designer de rotinas** (`CREATE OR ALTER`)
- [x] **MySQL (fatia vertical)** — driver `crates/atlas-mysql` (`mysql_async`; banco = schema, `KILL QUERY` para parar) e comandos `mysql_*`: conexão (senha do cofre e túnel SSH), árvore (tabelas, views, funções, consultas), leitura e edição na grade (com ou sem chave primária), transação manual e editor SQL com dialeto MySQL
- [x] **MySQL — designer de tabela** (nova tabela e alterar: campos, índices, FKs, uniques, checks e SQL Preview; tipos do MySQL com UNSIGNED); **auto incremento** = `AUTO_INCREMENT` em coluna inteira que seja chave, uma por tabela; validado ao vivo (criar, inserir sem informar o id, desligar e ligar numa coluna existente)
- [x] **MySQL — rótulos de FK, diagrama ER, Find in Database** (estrutura e dados; dados com `LIKE`, igualdade ou `REGEXP`, cancelável), **Data Profiling e Table Profile** (tamanhos pelo catálogo)
- [x] **MySQL — exportação e importação** (todos os formatos; seis modos, tabela nova com os tipos inferidos, continuar em erro por linha com `SAVEPOINT`, cancelável)
- [x] **MySQL — designer de view** (`CREATE OR REPLACE VIEW`), **renomear, duplicar e truncar** tabela, renomear view, **Dump SQL File** (`SHOW CREATE` + `INSERT`, reexecutado pelo cliente `mysql`), **console** (estilo `mysql`) e **funções/procedimentos** (abrir o texto em aba de consulta, criar modelo, apagar), **novo e excluir banco**
- [x] **MySQL — backup, restauração e extrair SQL** (o script do dump compactado em `.dump`, com `DROP` antes de cada objeto, comentário, pasta e seleção de objetos; restaurar roda o script no banco; extrair grava o SQL), **editar banco** (conjunto de caracteres e collation) e **designer de rotinas** (texto da função ou procedimento, gravar com `DROP` + `CREATE` e recriação da versão anterior se falhar); corrigido erro de comando posterior engolido em scripts com vários comandos
- [x] **Firebird (fatia vertical)** — driver `crates/atlas-firebird` (cliente em **Rust puro**, sem `fbclient.dll`: serve do Firebird 2.5 ao 5; usa uma cópia ajustada do `rsfbclient-rust` em `vendor/`, que mantém `NUMERIC`, `DATE` e `TIME` exatos) e comandos `firebird_*`: conexão (senha do cofre e túnel SSH), árvore (tabelas, views, procedimentos e funções, geradores; sem schemas, o arquivo é o banco), leitura e edição na grade (`ROWS m TO n`, parâmetros `?`, sem chave por `RDB$DB_KEY`), estrutura pelas tabelas `RDB$` (colunas, chave, índices, FKs, uniques, checks, gatilhos, identidade no 3+), transação manual, vários comandos por texto (separador com `SET TERM`, corpos `BEGIN ... END`) e editor SQL; validado ao vivo no 2.5.3 e no 4.0.2
- [x] **Firebird — designer de tabela** (nova e alterar: campos, índices, FKs, uniques, checks e SQL Preview; nomes novos em minúsculas viram maiúsculos); **auto incremento** = `GENERATED BY DEFAULT AS IDENTITY` no 3+ e **gerador + gatilho `BEFORE INSERT`** no 2.5 e em coluna que já existe (o gerador parte do maior valor); tirar a identidade pede o Firebird 4; validado ao vivo no 2.5.3 e no 4.0.2 (criar, inserir sem informar o id, ligar e desligar numa coluna existente)
- [x] **Firebird — escolher o arquivo do banco** por botão `…` na tela de conexão (filtros `.fdb` e `.gdb`; o Firebird abre os dois pelo conteúdo, não pela extensão; vale para servidor na própria máquina, em servidor remoto digita-se o caminho ou o alias)
- [x] **Firebird — views, rotinas e banco**: designer de view (`CREATE OR ALTER VIEW`), designer de rotinas (procedimentos e, no 3+, funções; texto lido das tabelas `RDB$`, gravado com `CREATE OR ALTER`), duplicar, esvaziar e excluir tabela, **novo banco** pelo botão `Novo...` da tela de conexão (`CREATE DATABASE` no servidor); o Firebird não renomeia tabela nem view, então esses itens não aparecem
- [x] **Firebird — exportação e importação** (todos os formatos; seis modos, tabela nova com os tipos do assistente traduzidos, continuar em erro por linha com `SAVEPOINT`, cancelável; sem `INSERT` de várias linhas, cada linha é um comando), **Find in Database** (estrutura: tabelas, campos, índices, gatilhos, views e rotinas com o corpo; dados: `CONTAINING`/`LIKE`, igualdade, `STARTING WITH` e `SIMILAR TO`), **Data Profiling, Table Profile, diagrama ER e rótulos de FK**
- [x] **Firebird — Dump SQL, backup, restauração e extrair SQL** (script próprio, sem o `gbak`: geradores, tabelas, views, dados, depois chaves estrangeiras, índices, rotinas e gatilhos entre `SET TERM`; o contador de geradores e identidades vai junto), **console** (`.tables`, `.schema`, `.indexes`...) e **Server Monitor** (`MON$ATTACHMENTS`, encerrar conexão; configurações e contadores de `MON$DATABASE`)
- [x] **Firebird — ferramentas do menu Tools**: Data Generation (sem `INSERT` de várias linhas: blocos `EXECUTE BLOCK`), Data Dictionary, **Data Transfer, Data Synchronization e Structure Synchronization** com outros Firebird (estrutura pelo script do dump, com gatilhos e rotinas) e **entre produtos** (PostgreSQL, SQL Server, MySQL, SQLite: tabelas, colunas, chaves, índices, `CHECK` e views; a numeração automática vira identidade no 3+ e gerador + gatilho no 2.5; nomes de restrição e índice têm até 31 caracteres)
- [x] **Firebird — gatilhos**: aparecem na pasta Functions (nome e tabela) e abrem no designer de rotinas com o texto do `CREATE OR ALTER TRIGGER` (gravar e excluir); a estrutura da tabela mostra os gatilhos dela
- [x] **Firebird — geradores e transferência 3+ → 2.5**: pasta de geradores na árvore (criar, definir o valor atual por `SET GENERATOR` e excluir); transferir de um Firebird 3 ou 4 para um 2.5 troca a identidade por gerador + gatilho `BEFORE INSERT`
- [x] **Várias versões de cada servidor testadas ao vivo** (2026-10-26) com a suíte completa de PostgreSQL, MySQL e Firebird. A versão do servidor é detectada na conexão, sem o usuário informar. O MySQL antigo aceita `CHECK` mas o servidor o descarta, então o teste só o exige onde ele existe. O SQLite vai embutido no programa.
- [ ] **Firebird — limites conhecidos**: o Firebird não renomeia tabela nem view (precisaria recriar o objeto); o `SIMILAR TO` do Find usa a sintaxe de expressão regular do SQL (não a do Rust)
- [x] **Oracle (fatia vertical)** — driver `crates/atlas-oracle` sobre o `oracle-rs` em **Rust puro** (não precisa do Oracle Instant Client; 12.1 em diante), com uma cópia ajustada em `vendor/oracle-rs` (ver o `ATLAS-CHANGES.md`, 12 ajustes: antes deles qualquer erro do servidor derrubava a sessão e a mensagem `ORA-` se perdia, toda consulta parava em 100 linhas, valores repetidos viravam `NULL`, linhas largas e `LONG` não eram lidos e não havia como parar uma consulta) e comandos `oracle_*`: conexão por serviço (`FREEPDB1`, `XEPDB1`; `sid:NOME` para SID), senha do cofre, túnel SSH e TLS, árvore (o "banco" é o serviço, os schemas são os usuários; Tables, Views e Queries), leitura e edição na grade (`OFFSET ... FETCH`, parâmetros `:n`, tabela sem chave pelo `ROWID`), transação manual, editor SQL com vários comandos e blocos PL/SQL (`/` numa linha só termina o bloco), erros com o código `ORA-` e a sessão intacta, dados e tipos (`NUMBER` sem perda, `DATE`/`TIMESTAMP`, `CLOB`, `LONG`, `BLOB` em hexa; `XMLType` como texto), **parar a consulta** sem privilégios, **posição do erro** no texto e catálogo `ALL_*` (colunas, chave primária, únicas, estrangeiras, `CHECK`, índices, gatilhos). Testado ao vivo no **Oracle 26ai Free**, no **21c Express** e no **18 Express** (Docker): `cargo test -p atlas-oracle` com as variáveis `ATLAS_TEST_ORACLE_*`
- [x] **Oracle — designer de tabela**: nova tabela e alterar (campos, índices, FKs, uniques, checks e SQL Preview; tipos do Oracle); **auto incremento** = `GENERATED BY DEFAULT AS IDENTITY` na tabela nova e na coluna nova; numa coluna que **já existe** o Oracle não converte em identidade (`ORA-30673`), então vira uma sequência própria (a partir do maior valor) com `DEFAULT seq.NEXTVAL`, e o designer reconhece as duas formas; validado ao vivo no 21c e no 26ai (criar, inserir sem informar o id, ligar e desligar numa coluna existente, `DROP IDENTITY`, adicionar, renomear, mudar tipo e remover coluna)
- [x] **Oracle — verificado na janela do aplicativo** (26ai): nova conexão (o formulário traz "Service name"), teste de conexão, árvore (serviço, schemas, Tables/Views/Queries), lista de tabelas, **diagrama ER** (`schema_diagram`, com tabelas, chaves e relações), grade (150 registros com `CLOB`, `DATE`, `TIMESTAMP` e acentos; inserir com identidade vazia, alterar célula, excluir com confirmação, transação manual com Commit e Rollback e isolamento entre sessões), **Filtrar e ordenar** (o filtro agora é do dialeto: sem `::tipo` fora do PostgreSQL e com `TO_DATE`/`TO_TIMESTAMP` no Oracle), editor SQL (consulta, erro `[ORA-00904]` com a posição, **Parar** numa consulta longa, script com `CREATE TABLE`, `INSERT`, bloco PL/SQL e dois resultados) e designer de tabela (abrir, ligar e desligar o auto incremento numa coluna existente, criar tabela nova com identidade). Variável de ligação sem valor (`:id` no editor, ou `::tipo`) é recusada antes de enviar com `ORA-01008`: o servidor ficaria esperando os dados e a conexão travaria
- [ ] **Oracle — o que ainda falta**: exportar e importar, Dump SQL, backup e restauração, console, Find in Database, Data Profiling, rótulos de FK, designer de view e de rotinas (funções, procedimentos, pacotes), sequências, gatilhos, novo/editar banco e usuário, Server Monitor e as ferramentas do menu Tools (Data Transfer, Data Synchronization, Structure Synchronization, Data Generation, Data Dictionary); Query Builder verificado ao vivo. Limites conhecidos: parar a consulta (marcador de interrupção, sem privilégios) não vale com TLS, onde cai no `ALTER SYSTEM CANCEL SQL`; `XMLType` só aparece como texto ao abrir a tabela (uma consulta escrita à mão com `XMLType` precisa de `XMLSERIALIZE`); consultas com variável de ligação não são suportadas (o Atlas não manda valores); o Oracle faz commit sozinho a cada DDL, então alterar uma tabela existente não é desfeito se um passo falhar no meio (a tabela nova criada pelo designer é apagada se um passo depois do `CREATE TABLE` falhar); o cache de instruções do driver fica desligado (a reexecução de uma consulta com LOB pelo cache perdia linhas)
- [ ] MariaDB, MongoDB, Redis *(**pendentes por decisão** (2026-10-26): os bancos que o Atlas tem bastam por enquanto; a ordem sugerida, quando voltar, é MongoDB, Redis e por último o MariaDB, que quase repete o MySQL)*

## 2. Árvore / Navegação
- [x] **Filtro avançado da árvore**: o funil ao lado da busca abre o menu **Contém** / **Começa com** e **Diferenciar maiúsculas**
      (lembrado entre sessões); o destaque em amarelo segue a regra escolhida e a barra de status conta os itens que o filtro
      esconde (`11 Databases (2 items hidden)` vira `2 Databases (11 items hidden)`); com um banco selecionado mostra os schemas
      *(`filterRange`/`makeMatcher` em `src/model.ts`, `NavigationPane.tsx`)*
- [x] Find in Database / Find in Schema (ver "Localizar no banco / schema" em Entregues)
- [x] Console do servidor (ver "Console" em Entregues)
- [x] **Share** (menu de uma conexão e de um banco): janela "Share via Atlas URI" com o endereço `atlas://...` para copiar, **Include hosts & ports** e **Include password template** (campo de senha em branco; a senha de verdade nunca vai) e **Copy Atlas URI** *(`src/shareUri.ts`, `src/components/ShareDialog.tsx`)*
- [ ] Receber um endereço Atlas (colar para criar a conexão ou abrir o banco) e registrar o protocolo `atlas://` no sistema *(sem captura e sem demanda por enquanto; hoje o endereço só é gerado)*
- [x] Edit Database / Edit Schema (ver "Navegação / Árvore" em Entregues)

## 3. Grid de dados
- [x] Filter & Sort avançado — diálogo "Filtrar e ordenar" com WHERE multi-regra
      (operadores: `=`, `≠`, `<`, `≤`, `>`, `≥`, `LIKE`, `NOT LIKE`, `IN`, `BETWEEN`,
      `IS NULL`, `IS NOT NULL`), ordenação multi-coluna ASC/DESC, prévia do SQL e paginação
      que respeita o filtro (backend extendido com `OrderBy`/`TableFilter`)
      *(`src/components/FilterSortDialog.tsx`, `crates/atlas-pg/src/exec.rs`)*
- [x] Columns (mostrar/ocultar colunas)
- [x] **Data Profiling** (botão da tabela): painel para escolher **Todos os registros** ou **Adicionar filtro** (montar pelo diálogo de filtro, ou texto `<Campo> = <Valor> AND ...`, ordenação `<Campo> ASC, <Campo> DESC` e **Sem limite / Os primeiros N**); **Iniciar perfil** calcula, no servidor, por coluna: nulos, distintos, únicos, menor e maior valor e os 100 valores mais frequentes. A grade ganha, sob cada nome de coluna, a barra de nulos, as barras dos valores e "N distintos, M únicos" (dica com as contagens); embaixo, **Estatísticas da coluna** e **Distribuição dos valores** da coluna selecionada; clicar numa barra põe o valor **em destaque** (Ctrl soma): linhas e colunas ficam realçadas e **Valor em destaque ▲▼** pula entre as linhas; **Sair do perfil** volta à grade *(`crates/atlas-pg/src/profile.rs`, `src/components/DataProfiling.tsx`)*
- [x] Data Profiling: o botão **Settings** da referência (reabre o painel do filtro) foi feito e **retirado a pedido** (o painel de antes do perfil basta; para mudar o filtro, saia do perfil e comece de novo); os botões de layout da barra do perfil não foram feitos (sem conteúdo conhecido)
- [ ] Data Profiling: a grade mostra a página atual, não os registros do filtro do perfil
- [x] Data Generation (ver "Data Generation" em Entregues)
- [x] Create BI Workspace: botão **oculto** da tabela (não há BI no Atlas)
- [x] Begin Transaction manual (ver "Grid de dados" em Entregues)
- [x] Cell Editor expandido para texto grande / JSON (painel inferior com textarea, quebra de
      linha, "Formatar JSON" e "Definir como NULL"; aberto pelo botão da toolbar — liga/desliga —
      ou pelo menu de contexto da célula; sincroniza em tempo real com a célula ativa)
      — BLOB e XML formatado ficam para depois *(`src/components/CellEditorPanel.tsx`)*
- [x] **Cell Editor com Texto, Hexa, Imagem e Web**: a célula ativa vira texto editável, vista em hexa (16 bytes por linha com ASCII), imagem
      (PNG, JPEG, GIF, BMP, WEBP, ICO e SVG, também texto `data:image/...;base64`) ou HTML num quadro isolado (sem scripts); **Carregar...** coloca um
      arquivo na célula (colunas `bytea` guardam os bytes; colunas de texto aceitam arquivo UTF-8, até 64 MB) e **Salvar...** grava a célula em arquivo
      (idêntico ao original, conferido por MD5); `bytea` que é UTF-8 também se edita como texto *(`src/blob.ts`, `CellEditorPanel.tsx`)*
- [x] **Valor da chave estrangeira**: botão "FK Values" na barra da tabela troca a chave pela descrição da linha referenciada (coluna `nome`,
      `name`, `descricao`... ou a primeira de texto que não pareça chave), só em chaves de uma coluna e só na exibição (editar, copiar e o lookup "…"
      seguem com a chave); busca só as chaves da página, com parâmetros, e lembra a escolha *(`pg_fk_labels`, `crates/atlas-pg/src/exec.rs`)*
- [ ] Agrupamento / sumário no rodapé do grid *(**não existe na referência**: a captura do grid não mostra linha de resumo; foi feito e **removido** a pedido)*

## 4. Editor SQL
- [x] Snippet manager (ver "Snippets" em Entregues)
- [ ] Ask AI *(botão **oculto** da barra da consulta a pedido, enquanto não houver IA integrada; o texto `query.askAi` segue nas traduções)*
- [x] **Divisão do editor**: os dois botões do canto da consulta alternam entre resultados **abaixo** (padrão) e **ao lado** do editor, com divisor
      arrastável nos dois sentidos; a escolha fica lembrada
- [x] Histórico de consultas e favoritos (ver "Histórico de consultas" em Entregues)
- [x] **Transação manual na consulta**: botão "Begin Transaction" abre uma conexão dedicada; tudo que roda nela fica pendente até **Commit** (Confirmar)
      ou **Rollback** (Reverter), com faixa de aviso e seletores de conexão/banco travados; fechar a aba reverte e ela pergunta antes (verificado: outra
      conexão não enxerga o que está pendente) *(`pg_tx_execute`, `QueryDoc.tsx`)*
- [x] **Janela de parâmetros** (`:nome`): ao executar, os parâmetros da consulta (ou da seleção) pedem Tipo (Texto, Número, NULL ou Expressão) e valor,
      que ficam lembrados na aba; `::cast`, textos, comentários e blocos `$$` não contam, e o texto vira literal com as aspas escapadas *(`src/sql/params.ts`)*
- [x] **Minify SQL** (menu Query e barra da consulta): tira comentários e junta tudo numa linha, preservando textos, identificadores entre aspas e blocos
      `$$`; Beautify e Minify valem para a seleção (ou o texto todo); **Converter para maiúsculas/minúsculas** do menu Format também passaram a funcionar
- [x] **Explain** (botão e menu Query): aba "Explain 1" depois dos resultados, com **Visual** (árvore de nós com custo acima e tabela/tipo abaixo; o nome fica vermelho
      quando o custo próprio do nó passa do **limite de custo (%)**; zoom, salvar como imagem SVG e localizar com anterior/próximo), **JSON**, **Text**, **Statistics**
      (por tipo de nó: quantidade, custo e %, batendo com os números da referência) e **Info** *(`src/explain.ts`, `src/components/ExplainView.tsx`)*
- [x] **Summary** (aba depois de Message): comandos processados, com êxito e com erro, início, fim e tempo total, e uma linha por comando com mensagem e tempos de consulta e de leitura;
      **Show Errors Only**. Com vários comandos juntos os tempos por comando aparecem como "—" (o servidor entrega as respostas de uma vez); um erro vale para o texto inteiro, então é uma linha só
- [x] **Pin** (resultado ou plano): fixa a aba, que continua ao rodar outra consulta, com × para desafixar *(`src/components/SummaryView.tsx`, `src/documents/QueryDoc.tsx`)*

## 5. Designers
- [x] Trigger designer completo (aba Gatilhos da tabela; ver "Gatilhos" em Entregues)
- [x] Rules: já são uma aba do designer de tabela (como na referência); não há designer à parte
- [ ] Partition designer *(**não confirmado na referência**)*
- [ ] Inherits browser *(`src/documents/TableDesignerDoc.tsx:617`; **não confirmado na referência**)*
- [ ] Preview de dados no designer *(**não existe na referência**: o designer de tabela tem só Fields, Indexes, Foreign Keys, Uniques, Checks, Excludes, Rules, Triggers, Options, Comment e SQL Preview)*

## 6. Tipos de objeto não implementados
- [x] Materialized View: designer, abrir dados, atualizar dados (ver "Views materializadas" em Entregues)
- [x] Function / Procedure designer (ver "Funções e procedimentos" em Entregues)
- [ ] Debugger de PL/pgSQL *(**não confirmado na referência**: não aparece em nenhuma captura de funções)*
- [x] Type (ENUM, composto, domínio): designer (ver "Tipos" em Entregues)
- [x] Sequence: designer, reiniciar, definir valor (ver "Sequências" em Entregues)
- [ ] Event Trigger (gatilhos de evento do banco; os gatilhos de tabela já estão no designer; **não confirmado na referência**)
- [x] Extension: lista, instalar, atualizar, mover de schema e remover (ver "Extensões" em Entregues)
- [x] Foreign Server (ver "Foreign Server" em Entregues)
- [x] Foreign Table (ver "Foreign Table" em Entregues)
- [ ] Publication / Subscription (replicação lógica; **não confirmado na referência**)
- [x] Role / User manager: atributos, senha e participações (ver "Usuários e papéis" em Entregues); o gerenciador de
      privilégios por objeto segue em aberto
- [x] **Tablespace** (Others): lista (nome, dono, pasta e comentário) e designer que cria (`CREATE TABLESPACE ... LOCATION ... WITH (...)`), renomeia, troca o
      dono, altera as opções de custo (`SET`/`RESET`) e comenta, com a prévia do SQL; a pasta só vale ao criar; excluir (`DROP TABLESPACE`, fora de transação)
      não é oferecido para os do sistema (`pg_default`, `pg_global`). Testado ao vivo no PG 9.6 (criar numa pasta real, alterar, renomear, excluir)
      *(`crates/atlas-pg/src/meta.rs`, `TablespaceDesignerDoc.tsx`)*

## 7. Importar / Exportar
- [x] Import Wizard em etapas: TXT, CSV, JSON, XML, Excel e DBF (ver "Importação de dados"); Access/ODBC/Paradox pendentes
- [x] Export Wizard de tabelas/views em etapas, com várias tabelas e perfis (ver "Exportação de dados")
- [ ] Formatos que faltam: Access (`.accdb`) e Markdown *(**deixados de fora por enquanto**, por decisão: o `.accdb` depende do driver do Windows e o Markdown não aparece em nenhuma captura da referência)*
- [x] **Tarefas agendadas** (Ferramentas → Tarefas agendadas): **backup** (banco ou schema) e **exportação** de uma tabela (qualquer formato do assistente) que o Atlas roda
      sozinho **enquanto está aberto**: todo dia num horário, a cada N minutos/horas/dias ou uma vez; nome do arquivo com a data (`%Y%m%d...`), pasta escolhida,
      Executar agora, ativar/desativar, próxima e última execução com o resultado e erro mostrado em diálogo; tarefa vencida com o app fechado roda uma
      vez ao abrir; guardadas nas preferências locais. Testado de verdade: o agendador disparou sozinho uma exportação CSV, e um backup rodou pelo
      Executar agora e gerou o arquivo com o comentário *(`src/tasks.ts`, `taskRun.ts`, `TasksDialog.tsx`)*. Não há captura da referência (lá isso é a Automação):
      o desenho é próprio; não roda com o app fechado
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
- [x] Backup agendado (ver "Tarefas agendadas" em Exportar)

## 9. Automação *(fora de prioridade neste ciclo)*
- [ ] Batch jobs (encadear consultas, backups, transferências)
- [ ] Agendamento cron-like
- [ ] Notificações por e-mail

## 10. Modelagem (Model)
> **Decisão:** a modelagem sai deste projeto. Vai ser um **projeto separado**, inspirado em um modelador de dados dedicado (criar o modelo, importar de um banco, gerar o SQL, comparar modelos). O Atlas fica só com o ER Diagram da área de objetos (seção 14).
- [x] ER Diagram de **modelo** (criar, importar via "Reverse Schema to Model"): **fora do Atlas** (projeto separado); o modo Exibir → ER Diagram da área de objetos está na seção 14
- [x] Forward engineer (model → SQL): **fora do Atlas** (projeto separado)
- [x] Compare models: **fora do Atlas** (projeto separado)

## 11. BI *(fora de prioridade neste ciclo)*
- [ ] Workspace de dashboards (widgets, filtros, parâmetros, publicação)

## 12. Features cross-cutting *(fora de prioridade neste ciclo)*
- [ ] AI integrado (chat, geração de SQL, explicação, refactor)
- [ ] Sincronização em nuvem (conexões, consultas, snippets, modelos)
- [ ] Colaboração: comentários, histórico de versões
- [x] Server Monitor (ver "Server Monitor" em Entregues)
- [x] Query Builder visual (ver "Query Builder" em Entregues)
- [ ] Pivot Table
- [ ] Charts a partir de consulta
- [x] A barra de botões dos documentos (dados da tabela, designers, consulta) **corta e rola** quando não cabe, em vez de desenhar os ícones dos botões
      que sobram por cima do painel de informações; a roda do mouse rola a barra na horizontal *(`.object-toolbar` em `shell.css`)*
- [x] **Ajuda → Sobre** com o logotipo, a **versão** (lida do `package.json`, a mesma do instalador), o **autor**, a licença, o **link do GitHub**
      (abre no navegador padrão; só endereços `https://github.com/...` são aceitos) e a tecnologia *(`open_external` em `src-tauri/src/local.rs`)*
- [x] Tema claro/escuro manual (só na tela Opções → General, como na referência; padrão: seguir o sistema). Tema, idioma e legendas da barra saíram do menu Exibir para não haver duas portas para a mesma coisa
- [x] Tela de Preferências / Options (ver "Opções" em Entregues)
- [ ] Atalhos de teclado configuráveis
- [x] **Tela cheia** do documento (botão no canto da barra de ferramentas de consultas, dados da tabela e todos os designers): o
      documento cobre a janela do app; Esc, o mesmo botão ou trocar de aba saem *(`src/components/FullscreenButton.tsx`)*

## 13. Toolbar / rodapé — itens menores
- [ ] Conta (login do serviço em nuvem): o avatar do canto da barra principal foi **removido** (não fazia nada); volta junto com a nuvem
- [ ] Beautify de célula
- [ ] Hex / Binary editor próprio *(a vista em hexa só de leitura já existe no Cell Editor; falta editar os bytes por ela)*

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
      opções **Show Grid**, **Snap to Grid** (grade de 5 px) e **Show Foreign Key Names** ficam no menu Exibir e são lembradas; o botão
      Atualizar relê o schema. Os dados vêm de uma consulta só (`pg_schema_diagram`); sem servidor, usa as tabelas da
      demonstração. Não é a Modelagem da seção 10: aqui é a visão do schema como ele está no banco
      *(`src/components/ErDiagram.tsx`, `src/er.ts`, `crates/atlas-pg/src/meta.rs`)*
- **Zoom** (botões − / +, clique no % volta a 100%, Ctrl + roda do mouse; de 25% a 300%) e **tela cheia** (botão na barra inferior, Esc sai)
  no ER Diagram; o arrasto de cartões, linhas e pontas acompanha o zoom
- Selecionar várias tabelas (Ctrl+A ou Ctrl+clique) e **arrastar o grupo junto**, mantendo o desenho entre elas; o grupo para na borda
  esquerda/superior quando a primeira tabela encosta
- Ainda não há no ER Diagram: ferramentas de ligar (criar chave estrangeira pelo desenho), Preview e Size da barra
  inferior da referência, chaves para tabelas de outro schema (não têm cartão), tabelas com mais de 40 colunas mostram as
  40 primeiras e a linha "mais N colunas"
- [x] **Flatten Object List** (Exibir): a árvore mostra tabelas, views, views materializadas e funções direto sob o schema, sem as pastas; a escolha fica
      lembrada, a seta para a esquerda volta ao schema e a lista de objetos à direita não muda. Não há captura deste item, então o desenho é o mais simples
      possível *(`buildVisibleTree` em `src/model.ts`)*
- [x] **Sort**: ordenação da lista de objetos por qualquer coluna, crescente ou decrescente (também pelo clique no cabeçalho)
- [x] **Choose Columns**: escolher as colunas exibidas na lista de objetos (por categoria; o nome nunca some)
- [x] **Show Hidden Items**: mostra os bancos que a conexão oculta (templates e sem permissão de conexão) e os schemas de sistema; relê a árvore ao alternar
- [x] **Table Profile** (botão da barra da tabela, que já existia na referência): janela só de leitura com dono, tipo, persistência, tablespace, segurança por
      linha, identidade de réplica e comentário; linhas estimadas e tamanho da tabela, dos índices e total; quantas colunas, índices, chaves primárias e
      estrangeiras, únicas, verificações e gatilhos; e uso e manutenção (linhas vivas e mortas, leituras, último vacuum e analyze). Testado nos PG 9.6 e 18
      *(`pg_table_profile`, `TableProfileDialog.tsx`)*. O item "Show Table Profiles" do menu Exibir não aparece em nenhuma captura, então não foi feito à parte
- [x] **A tabela lembra a visão**: ao abrir de novo (inclusive depois de fechar o app), volta com a ordenação, o filtro, as colunas escondidas, as larguras ajustadas e o "Limitar registros"/registros por página que o usuário deixou, por conexão, banco, schema e tabela. Um filtro ou ordenação que não vale mais
      (coluna renomeada ou apagada) é descartado e a tabela abre normal; o botão Filter & Sort fica marcado enquanto há filtro ou ordenação *(`src/tableView.ts`, preferência `tv:<chave>`)*
- [x] **Reordenar colunas** arrastando o cabeçalho (barra azul mostra onde cai); a ordem entra na visão lembrada da tabela e na exportação
- [x] **Consultas ligadas a um banco se comportam como as tabelas**: fechar o banco ou a conexão as fecha e guarda (com o texto); voltam quando o banco abre, e ao reabrir o app só com o banco aberto. Pergunta antes se há transação aberta; o X esquece a consulta
- [x] **Lista de sugestões do editor como a da referência**: ícone por tipo (tabela, view, view materializada, função, coluna, palavra-chave), o schema à direita (a tabela, no caso de coluna), os argumentos da função ao lado do nome e o trecho digitado destacado em amarelo; tabelas vêm antes das funções e das palavras-chave *(`src/sqlCompletion.ts`)*
- [x] **Destacar aba**: dois cliques numa aba de tabela, view ou consulta abre numa janela própria (mesma sessão do banco); o botão **Anexar de volta** devolve a aba à janela principal, na mesma posição e já ativa. Fechar a janela esquece a aba (pergunta antes se há texto ou alterações). Não destaca com edições não gravadas ou transação aberta. Abas destacadas voltam como abas ao reabrir o app. **Arrastar a janela pela barra de título até a barra de abas da janela principal e soltar também a devolve** (a barra ganha um contorno azul enquanto o cursor está sobre ela) *(`src/components/TabWindow.tsx`, `src/tabWindow.ts`, `src/docBackends.ts`)*
- [x] **A tela é reconstruída igual ao destacar, anexar ou fechar e abrir o banco**: antes de mudar, o estado da tela vai para um arquivo temporário (`tab-state/`), é lido e apagado na hora de montar a aba (sobras com mais de um dia somem sozinhas). Consulta: resultados, mensagens, Summary, Explain, abas fixadas, aba aberta, divisão, parâmetros, cursor e rolagem. Tabela: página, linha selecionada, rolagem, visão grade/formulário e busca; os dados são relidos do banco *(`src/tabState.ts`)*
- [x] **Menu do botão direito nos campos de texto e no editor com o estilo do Atlas** (Recortar, Copiar, Colar, Selecionar tudo; itens que não valem ficam apagados), no lugar do menu do navegador ("Copiar o link para destacar", Imprimir, Inspecionar); vale também nas janelas de ferramentas e de abas destacadas *(`src/components/TextContextMenu.tsx`)*
- [x] **Tabela sem chave primária edita e exclui**: a linha é achada por todos os valores (os de tipos sem igualdade, como `json`, ficam de fora), então linhas iguais são todas afetadas; a tela mostra a observação sobre isso e não grava sozinha ao sair da linha (só pelo Aplicar). Só fica somente leitura se nenhuma coluna pode ser comparada *(`src/live.ts`, `crates/atlas-pg/src/apply.rs`)*
- [x] Tabela sem chave com linhas iguais: alterar ou excluir mexe **só em quantas linhas foram marcadas** (um comando por grupo de iguais, que escolhe as linhas pelo identificador interno dentro do próprio comando); se o servidor mexer em outra quantidade, desfaz tudo como conflito
- [x] Correções: a primeira letra digitada numa célula não duplica mais; linhas iguais marcadas juntas em tabela sem chave viram um comando só (sem o erro de zero linhas)
- [x] **Duplicar registro** (menu do botão direito da linha): janela com todos os campos da linha preenchidos; os que o banco gera (identidade, coluna gerada, sequência) ficam bloqueados e vazios, e os demais podem ser alterados (com caixa NULL); **Inserir** faz o `INSERT` na hora e o erro do servidor (chave repetida, campo obrigatório) aparece na própria janela, que continua aberta. Indisponível em view, com alterações pendentes na grade ou sem conexão real *(`src/components/DuplicateRecordDialog.tsx`)*
- [x] **Restaurar layout padrão** (menu ⋯ do cabeçalho e botão no diálogo Columns): ordem, largura e visibilidade das colunas e a ordenação das linhas voltam ao padrão; o filtro fica
- [x] **Reordenar abas** arrastando na barra (Objects fica sempre primeira); a posição é gravada e a tabela volta no lugar ao reabrir o banco
- [x] Perfis nomeados da visão da tabela (menu `Table Profile ▾`): **decidido não fazer**, porque a visão e o estado das telas já são guardados para continuar depois
- [x] Menu `Table Profile ▾` da referência: **decidido não fazer** (o botão abre a janela de informações direto)
