# Ajustes do Atlas no oracle-rs 0.1.7

Cópia do `oracle-rs` 0.1.7 (MIT OU Apache-2.0) com dois ajustes em `src/connection.rs`. Sem eles, qualquer erro do servidor (SQL com
erro de sintaxe, tabela inexistente, divisão por zero) derrubava a sessão e a mensagem `ORA-xxxxx` se perdia.

1. **Tamanho do pacote no modo `large_sdu`.** O marcador de reset (`send_marker`, nas duas versões) e o pacote de dados das funções
   simples saíam com 2 bytes de tamanho + 2 zeros; com `large_sdu` o cabeçalho tem 4 bytes de tamanho. O servidor lia um tamanho
   enorme e fechava a conexão. Agora o cabeçalho segue `large_sdu`.
2. **Campos do Oracle 20c na resposta de erro** (`parse_error_response`). Com a versão do protocolo (TTC) 14 ou mais, a resposta traz
   `sql type` e `server checksum` antes do texto do erro (como no `python-oracledb`). Sem lê-los a mensagem vinha vazia ou com lixo.
3. **Mensagem 23 (`ServerSidePiggyback`)**. O servidor a manda quando o estado da sessão muda (`ALTER SESSION`); o driver não a lia e o resto da
   resposta saía desalinhado (`invalid length indicator`). Agora é lida e descartada (`skip_server_side_piggyback`), como no `python-oracledb`,
   nos leitores de comando, consulta, busca, lote e PL/SQL.
4. **Coluna nula pela descrição** (`SELECT NULL, ...`). Com tamanho 0 na descrição o servidor não manda bytes da coluna; o driver tentava
   lê-los e todas as colunas seguintes saíam trocadas. O `buffer_size` passa a ser guardado e a regra do `python-oracledb` é aplicada
   (`null_by_describe`).
5. **Posição do erro no SQL**. A resposta de erro traz o ponto do texto onde o servidor falhou (o `python-oracledb` o expõe como `offset`); o
   driver o lia e descartava. Passa a existir `Error::OracleErrorAt { code, message, position }` (posição a partir de 0), usado quando a
   posição é maior que zero.
6. **Campos do Oracle 20c só quando o servidor os manda**. O leitor do fim da resposta pulava sempre `sql type` e `server checksum`
   ("suportamos 20c ou mais"); em servidores anteriores (18c, TTC 11) isso desalinhava a resposta (`buffer underflow`) em toda consulta. Agora
   depende da versão do protocolo negociada (`ttc_version`, guardada na `Connection`). Testado no 18 XE, no 21c XE e no 26ai Free.
7. **Consultas com mais de 100 linhas**. Três defeitos juntos faziam toda consulta parar na primeira leva (100 linhas) sem aviso:
   a resposta da consulta fixava `has_more_rows = false` (agora é "o fim da resposta não trouxe `ORA-01403`", como no Python); o pacote de
   `fetch_more` declarava 2 bytes a menos que o enviado (as flags de dados) e saía sem número de sequência, sem o `token` do 23ai e com o
   tamanho em 2 bytes no modo `large_sdu`; e o leitor da resposta de busca pulava 10 bytes fixos de `rowid` e adivinhava se havia mais
   linhas (agora usa o mesmo leitor de erro das outras respostas). `fetch_more` também passa a devolver o `cursor_id` para o lote seguinte.
   Verificado com 5.000 linhas no 18 XE, no 21c XE e no 26ai Free.
8. **`Connection::cancel()`** (novo). Interrompe a consulta em andamento, sem esperar a conexão ficar livre e sem privilégios: escreve o
   marcador de interrupção (tipo 3) direto na metade de escrita do socket (que passa a ser separada da de leitura e compartilhada), e a
   consulta termina com `ORA-01013`. Para o servidor aceitar o marcador em banda, o pacote de conexão deixou de anunciar que sabe
   receber "atenção" fora de banda (`CAN_RECV_ATTENTION` / `CHECK_OOB`), como o `python-oracledb` no Windows: o dado urgente do TCP não
   atravessa Docker nem túnel SSH e, quando o servidor o espera, ignora o marcador em banda. Só sem TLS. Verificado no 18 XE, 21c XE e 26ai Free.
9. **`LONG` e `LONG RAW`**. O servidor manda depois do valor o indicador de nulo e o código de retorno (como o `python-oracledb` lê); sem
   lê-los a linha seguinte saía deslocada. Junto com o item 7 (o servidor não manda as linhas desses tipos na primeira resposta, só na busca),
   passam a vir inteiros. `XMLType` não é decodificado pelo driver: o `atlas-oracle` pede o texto (`XMLSERIALIZE`) ao abrir a tabela.
10. **Respostas de consulta e de busca em vários pacotes de rede**. Linhas largas (texto longo, `CLOB`) fazem a resposta passar de um pacote, e o
    driver lia só o primeiro (`buffer underflow`). `read_complete` junta os pacotes seguintes até o leitor chegar à mensagem final (o
    `python-oracledb` lê o fluxo de bytes sem olhar os limites dos pacotes).
11. **Valores repetidos entre lotes de linhas**. O servidor omite as colunas iguais às da linha anterior (bit vector) e a primeira linha de um lote
    de `fetch_more` não conhecia o fim do lote anterior, então esses valores viravam `NULL` (e o erro se espalhava pelo resto do lote). A última
    linha de cada cursor passa a ser guardada (`last_rows`). Verificado com 300 linhas de 1.500 caracteres + `CLOB` nos três servidores.
12. **Cursor em cache sem as colunas**. Ao repetir o mesmo SQL o servidor não manda de novo a descrição das colunas, e o leitor era chamado com a
    lista vazia (linhas erradas ou, com a espera por mais pacotes do item 10, travamento); agora parte das colunas que o cursor já conhece.
    Mesmo assim, a reexecução de uma consulta com LOB pelo cache perde o primeiro lote de linhas: o `atlas-oracle` abre as conexões com o cache
    de instruções desligado (`stmtcachesize(0)`).
