//! JSON: uma lista de objetos (ou um objeto com a lista dentro). Cada chave vira uma coluna, na ordem em que aparece.

use crate::{RowSource, invalid};
use serde_json::Value;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;

/// Acima disso o arquivo não é carregado inteiro na memória.
const MAX_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) struct JsonRows {
    columns: Vec<String>,
    rows: std::vec::IntoIter<Vec<Option<String>>>,
}

fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        // Objeto ou lista dentro do registro: vai como JSON (serve para colunas json/jsonb).
        other => Some(other.to_string()),
    }
}

/// A lista de registros: o próprio valor, ou a única lista dentro de um objeto (ou a chave rows/data/records/items).
fn records(v: Value) -> io::Result<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a),
        Value::Object(map) => {
            for key in ["rows", "data", "records", "items", "results"] {
                if let Some(Value::Array(a)) = map.get(key) {
                    return Ok(a.clone());
                }
            }
            let arrays: Vec<&Value> = map.values().filter(|x| x.is_array()).collect();
            match arrays.as_slice() {
                [Value::Array(a)] => Ok(a.clone()),
                _ => Err(invalid("o JSON precisa ser uma lista de objetos (ou um objeto com a lista dentro)")),
            }
        }
        _ => Err(invalid("o JSON precisa ser uma lista de objetos")),
    }
}

impl JsonRows {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_BYTES {
            return Err(invalid("arquivo JSON grande demais para importar (limite de 512 MB)"));
        }
        let value: Value = serde_json::from_reader(BufReader::new(file)).map_err(|e| invalid(format!("JSON inválido: {e}")))?;
        let items = records(value)?;
        let mut columns: Vec<String> = Vec::new();
        for item in &items {
            let Value::Object(map) = item else { return Err(invalid("cada item do JSON precisa ser um objeto")) };
            for key in map.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
        let rows: Vec<Vec<Option<String>>> = items
            .iter()
            .map(|item| columns.iter().map(|c| item.get(c).and_then(value_text)).collect())
            .collect();
        Ok(Self { columns, rows: rows.into_iter() })
    }
}

impl RowSource for JsonRows {
    fn columns(&self) -> &[String] {
        &self.columns
    }
    fn next_row(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        Ok(self.rows.next())
    }
}
