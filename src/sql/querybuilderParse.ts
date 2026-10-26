/**
 * Lê um SELECT simples (o que o Query Builder consegue montar) e devolve suas partes, para reabrir o builder
 * a partir do texto do editor. Qualquer coisa fora desse formato (expressões, subconsultas, UNION, `::`...)
 * devolve `null`: o builder então começa do zero.
 */
import type { Aggregate, JoinType, Operator, QbDialect } from "./querybuilder";

export interface PRef {
  /** Tabela ou apelido que qualifica o campo (null = sem qualificador). */
  q: string | null;
  col: string;
}

export interface PCond {
  connector: "AND" | "OR";
  ref: PRef | null;
  aggregate: Aggregate;
  op: Operator;
  value: string;
  value2: string;
}

export interface ParsedQuery {
  distinct: boolean;
  fields: { ref: PRef; aggregate: Aggregate; alias: string }[];
  tables: { schema: string | null; name: string; alias: string }[];
  joins: { type: JoinType; left: PRef; right: PRef }[];
  where: PCond[];
  groupBy: PRef[];
  having: PCond[];
  orderBy: { ref: PRef; desc: boolean }[];
  limit: string;
  offset: string;
}

type Tok = { k: "id" | "word" | "str" | "num" | "p"; v: string };

class Unsupported extends Error {}

function tokenize(sql: string): Tok[] {
  const out: Tok[] = [];
  const re = /\s+|--[^\n]*|"((?:[^"]|"")*)"|`((?:[^`]|``)*)`|\[([^\]]+)\]|'((?:[^']|'')*)'|(-?\d+(?:\.\d+)?)|([A-Za-z_][A-Za-z0-9_$]*)|(<>|<=|>=|!=|[(),.*=<>;])/gy;
  let m: RegExpExecArray | null;
  let at = 0;
  while (at < sql.length) {
    re.lastIndex = at;
    m = re.exec(sql);
    if (!m) throw new Unsupported();
    at = re.lastIndex;
    if (m[1] !== undefined) out.push({ k: "id", v: m[1].replace(/""/g, '"') });
    else if (m[2] !== undefined) out.push({ k: "id", v: m[2].replace(/``/g, "`") });
    else if (m[3] !== undefined) out.push({ k: "id", v: m[3] });
    else if (m[4] !== undefined) out.push({ k: "str", v: m[4].replace(/''/g, "'") });
    else if (m[5] !== undefined) out.push({ k: "num", v: m[5] });
    else if (m[6] !== undefined) out.push({ k: "word", v: m[6] });
    else if (m[7] !== undefined) out.push({ k: "p", v: m[7] });
  }
  return out;
}

const STOP = new Set(["JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "OUTER", "ON", "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "AS", "AND", "OR", "NATURAL", "USING"]);
const AGG = new Set(["COUNT", "SUM", "AVG", "MIN", "MAX"]);
const OPS = new Set(["=", "<>", "!=", "<", ">", "<=", ">="]);

export function parseSelect(sql: string, dialect?: QbDialect): ParsedQuery | null {
  try {
    return new Parser(tokenize(sql), !dialect).parse();
  } catch (e) {
    if (e instanceof Unsupported) return null;
    throw e;
  }
}

class Parser {
  private i = 0;
  /** `lower`: o nome sem aspas vai para minúsculas (regra do PostgreSQL; nos outros bancos o nome fica como foi escrito). */
  constructor(private t: Tok[], private lower: boolean) {}

  private peek(n = 0): Tok | undefined {
    return this.t[this.i + n];
  }
  private word(w: string, n = 0): boolean {
    const x = this.peek(n);
    return !!x && x.k === "word" && x.v.toUpperCase() === w;
  }
  private punct(p: string, n = 0): boolean {
    const x = this.peek(n);
    return !!x && x.k === "p" && x.v === p;
  }
  private eatWord(w: string): boolean {
    if (!this.word(w)) return false;
    this.i++;
    return true;
  }
  private eatPunct(p: string): boolean {
    if (!this.punct(p)) return false;
    this.i++;
    return true;
  }
  private need(ok: boolean): void {
    if (!ok) throw new Unsupported();
  }

  /** Nome de objeto: o que não tem aspas vai para minúsculas (como o PostgreSQL faz). */
  private ident(): string {
    const x = this.peek();
    if (!x || (x.k !== "id" && x.k !== "word")) throw new Unsupported();
    this.i++;
    return x.k === "word" && this.lower ? x.v.toLowerCase() : x.v;
  }

  private ref(): PRef {
    const parts: string[] = [this.ident()];
    while (this.eatPunct(".")) {
      if (this.eatPunct("*")) {
        parts.push("*");
        break;
      }
      parts.push(this.ident());
    }
    if (parts.length > 3) throw new Unsupported();
    const col = parts[parts.length - 1];
    return { q: parts.length > 1 ? parts[parts.length - 2] : null, col };
  }

  private tableRef(): { schema: string | null; name: string; alias: string } {
    let name = this.ident();
    let schema: string | null = null;
    if (this.eatPunct(".")) {
      schema = name;
      name = this.ident();
    }
    let alias = "";
    if (this.eatWord("AS")) alias = this.ident();
    else {
      const x = this.peek();
      if (x && (x.k === "id" || (x.k === "word" && !STOP.has(x.v.toUpperCase())))) alias = this.ident();
    }
    return { schema, name, alias };
  }

  parse(): ParsedQuery {
    const q: ParsedQuery = { distinct: false, fields: [], tables: [], joins: [], where: [], groupBy: [], having: [], orderBy: [], limit: "", offset: "" };
    this.need(this.eatWord("SELECT"));
    q.distinct = this.eatWord("DISTINCT");
    // SQL Server: `SELECT TOP n`.
    if (this.eatWord("TOP")) {
      const paren = this.eatPunct("(");
      q.limit = this.integer();
      if (paren) this.need(this.eatPunct(")"));
    }
    if (this.punct("*") && (this.word("FROM", 1) || !this.peek(1))) this.i++;
    else {
      do {
        let aggregate: Aggregate = "";
        let ref: PRef;
        const x = this.peek();
        if (x && x.k === "word" && AGG.has(x.v.toUpperCase()) && this.punct("(", 1)) {
          aggregate = x.v.toUpperCase() as Aggregate;
          this.i += 2;
          if (this.eatPunct("*")) ref = { q: null, col: "*" };
          else ref = this.ref();
          this.need(this.eatPunct(")"));
        } else {
          this.need(!this.punct("*"));
          ref = this.ref();
        }
        let alias = "";
        if (this.eatWord("AS")) alias = this.ident();
        else {
          const n = this.peek();
          if (n && (n.k === "id" || (n.k === "word" && !STOP.has(n.v.toUpperCase()) && !this.word("FROM")))) alias = this.ident();
        }
        q.fields.push({ ref, aggregate, alias });
      } while (this.eatPunct(","));
    }

    this.need(this.eatWord("FROM"));
    q.tables.push(this.tableRef());
    for (;;) {
      if (this.eatPunct(",")) {
        q.tables.push(this.tableRef());
        continue;
      }
      let type: JoinType | null = null;
      if (this.eatWord("CROSS")) {
        this.need(this.eatWord("JOIN"));
        q.tables.push(this.tableRef());
        continue;
      }
      if (this.eatWord("INNER")) type = "INNER";
      else if (this.eatWord("LEFT")) type = "LEFT";
      else if (this.eatWord("RIGHT")) type = "RIGHT";
      else if (this.eatWord("FULL")) type = "FULL";
      if (type) this.eatWord("OUTER");
      if (!this.eatWord("JOIN")) {
        this.need(type === null);
        break;
      }
      q.tables.push(this.tableRef());
      this.need(this.eatWord("ON"));
      do {
        const left = this.ref();
        this.need(this.eatPunct("="));
        const right = this.ref();
        q.joins.push({ type: type ?? "INNER", left, right });
      } while (this.eatWord("AND"));
    }

    if (this.eatWord("WHERE")) q.where = this.conditions(false);
    if (this.eatWord("GROUP")) {
      this.need(this.eatWord("BY"));
      do q.groupBy.push(this.ref());
      while (this.eatPunct(","));
    }
    if (this.eatWord("HAVING")) q.having = this.conditions(true);
    if (this.eatWord("ORDER")) {
      this.need(this.eatWord("BY"));
      do {
        const ref = this.ref();
        const desc = this.eatWord("DESC");
        if (!desc) this.eatWord("ASC");
        q.orderBy.push({ ref, desc });
      } while (this.eatPunct(","));
    }
    if (this.eatWord("LIMIT")) q.limit = this.integer();
    if (this.eatWord("OFFSET")) {
      q.offset = this.integer();
      // SQL Server: `OFFSET m ROWS [FETCH NEXT n ROWS ONLY]`.
      if (this.eatWord("ROWS") || this.eatWord("ROW")) {
        if (this.eatWord("FETCH")) {
          this.need(this.eatWord("NEXT") || this.eatWord("FIRST"));
          q.limit = this.integer();
          this.need(this.eatWord("ROWS") || this.eatWord("ROW"));
          this.need(this.eatWord("ONLY"));
        }
      }
    }
    this.eatPunct(";");
    this.need(this.i === this.t.length);
    return q;
  }

  private integer(): string {
    const x = this.peek();
    this.need(!!x && x.k === "num" && /^\d+$/.test(x.v));
    this.i++;
    return x!.v;
  }

  private value(): string {
    const x = this.peek();
    if (!x) throw new Unsupported();
    this.i++;
    if (x.k === "num") return x.v;
    if (x.k === "str") return /^(-?\d+(\.\d+)?|true|false|null)$/i.test(x.v) ? `'${x.v.replace(/'/g, "''")}'` : x.v;
    if (x.k === "word" && /^(true|false|null)$/i.test(x.v)) return x.v.toLowerCase();
    throw new Unsupported();
  }

  private conditions(having: boolean): PCond[] {
    const out: PCond[] = [];
    let connector: "AND" | "OR" = "AND";
    for (;;) {
      let aggregate: Aggregate = "";
      let ref: PRef | null;
      const x = this.peek();
      if (having && x && x.k === "word" && AGG.has(x.v.toUpperCase()) && this.punct("(", 1)) {
        aggregate = x.v.toUpperCase() as Aggregate;
        this.i += 2;
        ref = this.eatPunct("*") ? null : this.ref();
        this.need(this.eatPunct(")"));
        if (ref === null) this.need(aggregate === "COUNT");
      } else {
        ref = this.ref();
      }
      let op: Operator;
      let value = "";
      let value2 = "";
      const n = this.peek();
      if (n && n.k === "p" && OPS.has(n.v)) {
        this.i++;
        op = (n.v === "!=" ? "<>" : n.v) as Operator;
        value = this.value();
      } else if (this.word("IS")) {
        this.i++;
        op = this.eatWord("NOT") ? "IS NOT NULL" : "IS NULL";
        this.need(this.eatWord("NULL"));
      } else {
        const not = this.eatWord("NOT");
        if (this.eatWord("LIKE")) op = not ? "NOT LIKE" : "LIKE";
        else if (!not && this.eatWord("ILIKE")) op = "ILIKE";
        else if (this.eatWord("IN")) {
          op = not ? "NOT IN" : "IN";
          this.need(this.eatPunct("("));
          const items: string[] = [];
          do items.push(this.value());
          while (this.eatPunct(","));
          this.need(this.eatPunct(")"));
          value = items.join(", ");
        } else if (!not && this.eatWord("BETWEEN")) {
          op = "BETWEEN";
          value = this.value();
          this.need(this.eatWord("AND"));
          value2 = this.value();
        } else throw new Unsupported();
        if (op === "LIKE" || op === "NOT LIKE" || op === "ILIKE") value = this.value();
      }
      out.push({ connector, ref, aggregate, op, value, value2 });
      if (this.eatWord("AND")) connector = "AND";
      else if (this.eatWord("OR")) connector = "OR";
      else break;
    }
    return out;
  }
}
