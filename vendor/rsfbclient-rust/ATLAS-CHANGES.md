# Alterações do Atlas / Atlas changes

Cópia do `rsfbclient-rust` 0.29.0 (cliente Firebird em Rust puro, licença MIT, © os autores do `rsfbclient`), usada pelo
`crates/atlas-firebird` por `[patch.crates-io]` no `Cargo.toml` da raiz. Mudanças em relação ao original:

- `xsqlda.rs`: guarda o tipo e a escala que o servidor descreveu (`orig_type`, `orig_scale`) antes de `coerce`; `NUMERIC`/`DECIMAL`
  continuam inteiros com escala (o original os convertia em `double`, perdendo casas); o BLR de `INT64` usa a escala descrita.
- `wire.rs`: `Column::raw_type` leva o tipo descrito empacotado (`atlas_raw_type`: código do tipo, escala e subtipo do blob), o que
  permite distinguir `DATE`, `TIME` e `TIMESTAMP`, e formatar `NUMERIC` com as casas certas.
- `client.rs`: `last_describe()` devolve as colunas descritas pelo último comando preparado na thread (nome e tipo), para mostrar
  o cabeçalho mesmo quando o resultado não tem linhas.

Copy of `rsfbclient-rust` 0.29.0 (pure-Rust Firebird client, MIT license, © the `rsfbclient` authors), used by `crates/atlas-firebird`
through `[patch.crates-io]`. The changes above keep the described column type and scale instead of coercing them.
