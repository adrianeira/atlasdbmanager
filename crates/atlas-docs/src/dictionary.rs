//! Data Dictionary: o modelo dos dados (servidores, bancos, schemas, tabelas, views e funções), os ajustes de layout e o
//! desenho das páginas (capa, sumário, introdução e conteúdo) com os cinco modelos de cor.

use crate::fonts::{Font, text_width, wrap};
use crate::page::{Color, Page};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- dados

pub type Info = Vec<(String, String)>;

#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    pub servers: Vec<Server>,
}

#[derive(Debug, Clone, Default)]
pub struct Server {
    pub name: String,
    pub product: String,
    /// "Server Information", se marcada.
    pub info: Option<Info>,
    pub databases: Vec<Database>,
}

#[derive(Debug, Clone, Default)]
pub struct Database {
    pub name: String,
    pub info: Option<Info>,
    pub schemas: Vec<Schema>,
}

#[derive(Debug, Clone, Default)]
pub struct Schema {
    pub name: String,
    pub info: Option<Info>,
    /// `None` = a categoria não foi marcada.
    pub tables: Option<Vec<Table>>,
    pub views: Option<Vec<View>>,
    pub functions: Option<Vec<Function>>,
}

#[derive(Debug, Clone, Default)]
pub struct Table {
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub foreign_keys: Vec<ForeignKey>,
    pub checks: Vec<(String, String)>,
    pub triggers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct Column {
    pub name: String,
    pub comment: Option<String>,
    pub data_type: String,
    pub not_null: bool,
    pub key: bool,
    pub auto_increment: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Index {
    pub name: String,
    pub comment: Option<String>,
    pub kind: String,
    pub method: String,
    pub fields: String,
}

#[derive(Debug, Clone, Default)]
pub struct ForeignKey {
    pub name: String,
    pub comment: Option<String>,
    pub fields: String,
    pub ref_table: String,
    pub ref_fields: String,
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Debug, Clone, Default)]
pub struct View {
    pub name: String,
    pub comment: Option<String>,
    pub definition: String,
}

#[derive(Debug, Clone, Default)]
pub struct Function {
    pub name: String,
    pub arguments: String,
    pub returns: Option<String>,
    pub language: String,
    pub comment: Option<String>,
    pub body: String,
}

// ---------------------------------------------------------------- ajustes

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Template {
    Orange,
    Red,
    BlackWhite,
    Blue,
    Green,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontSpec {
    /// "Default", "Arial", "Times New Roman" ou "Courier New".
    pub family: String,
    /// Em pixels (como na tela); no papel valem 3/4 de ponto.
    pub size: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub template: Template,
    pub include_cover: bool,
    pub cover_header: String,
    pub cover_title: String,
    pub cover_subtitle: String,
    pub cover_remarks: String,
    /// "default" | "custom" | "none" (a imagem própria ainda não é desenhada).
    pub cover_background: String,
    pub include_toc: bool,
    pub breadcrumbs: bool,
    pub breadcrumb_font: FontSpec,
    pub heading_font: FontSpec,
    pub subheading_font: FontSpec,
    pub left_footer: bool,
    pub left_footer_text: String,
    pub page_number: bool,
    pub footer_font: FontSpec,
    pub include_intro: bool,
    pub intro_text: String,
    pub body_font: String,
    /// "A3" | "A4" | "A5" | "Letter" | "Legal".
    pub paper: String,
    pub landscape: bool,
    /// Margens em centímetros: esquerda, direita, topo e base.
    pub margins: [f32; 4],
    /// Data e hora locais (ano, mês, dia, hora, minuto, segundo) para os `%Y`... da capa.
    pub now: [i32; 6],
}

pub const DEFAULT_INTRO: &str = "A data dictionary is a collection of metadata such as object name, data type, size, classification, and relationships with other data assets. It is used by data administrators, analysts, and engineers to understand and trust data assets. It helps in the creation of authentic, transparent, and consistent data throughout the organization.\n\nIn this data dictionary, following servers are chosen for documentation:\n[SERVER_LIST]\n\nFor each server, definitions of tables (entities/collections), views, functions are listed in order. Detailed properties of data elements (data type, size, nullability, optionality, indexes, foreign keys, constraints) are organized in tabular format.";
pub const DEFAULT_REMARKS: &str = "Server: [SERVER_NAME]\nAuthor: Administrator\nDate: %Y-%m-%d %H:%M:%S\nVersion: 1.0";

impl Settings {
    pub fn defaults(title: &str) -> Self {
        let font = |size: f32| FontSpec { family: "Default".into(), size };
        Settings {
            template: Template::Orange,
            include_cover: true,
            cover_header: String::new(),
            cover_title: title.to_string(),
            cover_subtitle: "Data Dictionary".into(),
            cover_remarks: DEFAULT_REMARKS.into(),
            cover_background: "default".into(),
            include_toc: true,
            breadcrumbs: true,
            breadcrumb_font: font(13.0),
            heading_font: font(18.0),
            subheading_font: font(17.0),
            left_footer: true,
            left_footer_text: title.to_string(),
            page_number: true,
            footer_font: font(13.0),
            include_intro: true,
            intro_text: DEFAULT_INTRO.into(),
            body_font: "Default".into(),
            paper: "A4".into(),
            landscape: false,
            margins: [1.9, 1.9, 1.9, 1.9],
            now: [2026, 1, 1, 0, 0, 0],
        }
    }
}

fn paper_size(name: &str, landscape: bool) -> (f32, f32) {
    // Em pontos (1 pol = 72 pt).
    let (w, h) = match name {
        "A3" => (841.89, 1190.55),
        "A5" => (419.53, 595.28),
        "Letter" => (612.0, 792.0),
        "Legal" => (612.0, 1008.0),
        _ => (595.28, 841.89),
    };
    if landscape { (h, w) } else { (w, h) }
}

// ---------------------------------------------------------------- cores por modelo

struct Palette {
    /// Títulos e fios.
    accent: Color,
    /// Subtítulo da capa.
    deep: Color,
    /// Fundo do cabeçalho das tabelas.
    tint: Color,
    /// Bordas das tabelas.
    border: Color,
}

fn palette(t: Template) -> Palette {
    match t {
        Template::Orange => Palette { accent: Color(204, 102, 10), deep: Color(110, 56, 12), tint: Color(253, 222, 190), border: Color(244, 190, 140) },
        Template::Red => Palette { accent: Color(230, 20, 70), deep: Color(140, 20, 45), tint: Color(255, 226, 232), border: Color(245, 170, 185) },
        Template::BlackWhite => Palette { accent: Color(20, 20, 20), deep: Color(110, 110, 110), tint: Color(232, 232, 232), border: Color(190, 190, 190) },
        Template::Blue => Palette { accent: Color(0, 120, 215), deep: Color(100, 100, 100), tint: Color(222, 238, 252), border: Color(160, 200, 238) },
        Template::Green => Palette { accent: Color(24, 168, 136), deep: Color(110, 110, 110), tint: Color(220, 245, 238), border: Color(150, 215, 195) },
    }
}

const GRAY: Color = Color(110, 110, 110);
const TEXT: Color = Color(40, 40, 40);

// ---------------------------------------------------------------- texto com marcadores

fn fill_placeholders(text: &str, s: &Settings, dict: &Dictionary) -> String {
    let [y, mo, d, h, mi, se] = s.now;
    let server = dict.servers.first().map(|x| x.name.as_str()).unwrap_or("");
    let list = dict.servers.iter().map(|x| format!("{} ({})", x.name, x.product)).collect::<Vec<_>>().join("\n");
    text.replace("[SERVER_NAME]", server)
        .replace("[SERVER_LIST]", &list)
        .replace("%Y", &format!("{y:04}"))
        .replace("%m", &format!("{mo:02}"))
        .replace("%d", &format!("{d:02}"))
        .replace("%H", &format!("{h:02}"))
        .replace("%M", &format!("{mi:02}"))
        .replace("%S", &format!("{se:02}"))
}

// ---------------------------------------------------------------- capa

fn quad(page: &mut Page, pts: [(f32, f32); 4], color: Color, alpha: f32) {
    page.poly(&pts, color, alpha);
}

fn cover(dict: &Dictionary, s: &Settings, pal: &Palette) -> Page {
    let (w, h) = paper_size(&s.paper, s.landscape);
    let mut p = Page::new(w, h);
    let decorated = s.cover_background != "none";
    let centered = s.template == Template::BlackWhite;
    if decorated {
        match s.template {
            Template::Orange => {
                // Placas inclinadas no pé da página, de claras a fortes.
                let base = h * 0.58;
                let rows: [(f32, f32, f32, f32, f32); 12] = [
                    (-0.02, 0.0, 0.16, 0.05, 0.18),
                    (0.12, 0.02, 0.22, 0.05, 0.35),
                    (0.30, 0.0, 0.30, 0.06, 0.2),
                    (0.55, 0.03, 0.2, 0.05, 0.3),
                    (0.78, 0.0, 0.3, 0.06, 0.45),
                    (-0.04, 0.10, 0.34, 0.07, 0.25),
                    (0.36, 0.11, 0.32, 0.07, 0.5),
                    (0.66, 0.13, 0.4, 0.07, 0.3),
                    (0.05, 0.22, 0.45, 0.09, 0.7),
                    (0.50, 0.24, 0.55, 0.09, 0.85),
                    (-0.05, 0.36, 0.5, 0.08, 0.2),
                    (0.30, 0.32, 0.8, 0.1, 0.5),
                ];
                for (x, y, ww, hh, a) in rows {
                    let (x0, y0, wd, ht) = (x * w, base + y * h, ww * w, hh * h);
                    quad(&mut p, [(x0 + 0.06 * w, y0), (x0 + wd, y0), (x0 + wd - 0.06 * w, y0 + ht), (x0, y0 + ht)], pal.accent, a);
                }
            }
            Template::Red => {
                quad(&mut p, [(0.0, 0.0), (w * 0.34, 0.0), (0.0, h * 0.17), (0.0, h * 0.17)], Color(255, 140, 150), 0.45);
                quad(&mut p, [(0.0, 0.0), (w * 0.20, 0.0), (0.0, h * 0.10), (0.0, h * 0.10)], pal.accent, 0.5);
                quad(&mut p, [(w * 0.70, h * 0.70), (w, h * 0.55), (w, h * 0.80), (w * 0.84, h * 0.80)], pal.accent, 0.75);
                quad(&mut p, [(w * 0.55, h * 0.85), (w * 0.85, h * 0.78), (w, h * 0.92), (w * 0.62, h)], Color(255, 150, 160), 0.5);
                quad(&mut p, [(w * 0.80, h * 0.86), (w, h * 0.80), (w, h), (w * 0.70, h)], Color(255, 110, 120), 0.4);
            }
            Template::Blue => {
                p.rect(0.0, h * 0.30, w, h * 0.30, Some(Color(238, 246, 253)), None);
                for (cx, cy, r, a) in [(0.62, 0.12, 0.09, 0.25), (0.78, 0.18, 0.12, 0.3), (0.90, 0.10, 0.07, 0.35), (0.72, 0.30, 0.06, 0.2), (0.86, 0.92, 0.1, 0.4), (0.68, 0.96, 0.07, 0.3)] {
                    let (x, y, rr) = (cx * w, cy * h, r * w);
                    quad(&mut p, [(x, y - rr), (x + rr, y), (x, y + rr), (x - rr, y)], Color(120, 190, 240), a);
                }
            }
            Template::Green => {
                p.rect(w * 0.1, h * 0.255, w * 0.8, 1.2, Some(pal.accent), None);
            }
            Template::BlackWhite => {}
        }
    }
    let margin = w * 0.12;
    if !s.cover_header.is_empty() {
        let x = if centered { (w - text_width(Font::Sans, &s.cover_header, 9.0)) / 2.0 } else { margin };
        p.text(x, h * 0.075, &s.cover_header, Font::Sans, 9.0, TEXT);
    }
    let title_size = 26.0;
    let sub_size = 19.0;
    let title_y = if centered { h * 0.40 } else { h * 0.31 };
    let lines = wrap(Font::SansBold, &s.cover_title, title_size, w - margin * 2.0);
    let mut y = title_y;
    for line in &lines {
        let x = if centered { (w - text_width(Font::SansBold, line, title_size)) / 2.0 } else { margin };
        p.text(x, y, line, Font::SansBold, title_size, pal.accent);
        y += title_size * 1.2;
    }
    let x = if centered { (w - text_width(Font::SansBold, &s.cover_subtitle, sub_size)) / 2.0 } else { margin };
    p.text(x, y + 4.0, &s.cover_subtitle, Font::SansBold, sub_size, pal.deep);
    y += sub_size * 1.2 + 26.0;
    for line in fill_placeholders(&s.cover_remarks, s, dict).lines() {
        let x = if centered { (w - text_width(Font::Sans, line, 6.5)) / 2.0 } else { margin };
        p.text(x, y, line, Font::Sans, 6.5, TEXT);
        y += 9.0;
    }
    let credit = "Generated with Atlas DB Manager";
    p.text(w - margin - text_width(Font::Sans, credit, 6.5), h - h * 0.075, credit, Font::Sans, 6.5, TEXT);
    p
}

// ---------------------------------------------------------------- fluxo de páginas

struct Entry {
    level: usize,
    text: String,
    page: usize,
}

struct Cell {
    lines: Vec<String>,
    /// Texto cinza abaixo (comentário).
    note: Vec<String>,
    /// Etiqueta com borda (ex.: `Auto Increment`).
    tag: Option<String>,
    key: bool,
    check: bool,
}

impl Cell {
    fn text(text: &str) -> Cell {
        Cell { lines: vec![text.to_string()], note: vec![], tag: None, key: false, check: false }
    }
}

struct Flow<'a> {
    s: &'a Settings,
    pal: Palette,
    body: Font,
    w: f32,
    h: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    pages: Vec<Page>,
    /// Quem é dono de cada página (para o cabeçalho): texto do caminho.
    crumbs: Vec<String>,
    crumb: String,
    entries: Vec<Entry>,
    y: f32,
    /// Número da página física da primeira página do fluxo.
    first_page: usize,
}

impl<'a> Flow<'a> {
    fn new(s: &'a Settings, first_page: usize) -> Self {
        let (w, h) = paper_size(&s.paper, s.landscape);
        let cm = 72.0 / 2.54;
        let mut f = Flow {
            s,
            pal: palette(s.template),
            body: Font::family(&s.body_font),
            w,
            h,
            left: s.margins[0] * cm,
            right: s.margins[1] * cm,
            top: s.margins[2] * cm,
            bottom: s.margins[3] * cm,
            pages: Vec::new(),
            crumbs: Vec::new(),
            crumb: String::new(),
            entries: Vec::new(),
            y: 0.0,
            first_page,
        };
        f.new_page();
        f
    }

    fn width(&self) -> f32 {
        self.w - self.left - self.right
    }

    /// Altura útil: deixa espaço para o rodapé.
    fn limit(&self) -> f32 {
        self.h - self.bottom - 18.0
    }

    fn new_page(&mut self) {
        self.pages.push(Page::new(self.w, self.h));
        self.crumbs.push(self.crumb.clone());
        self.y = self.top + if self.s.breadcrumbs { 22.0 } else { 0.0 };
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("page")
    }

    fn ensure(&mut self, height: f32) {
        if self.y + height > self.limit() {
            self.new_page();
        }
    }

    fn font_size(spec: &FontSpec) -> f32 {
        spec.size * 0.75
    }

    /// Título numerado; o nível 1 vem com o fio colorido e a bolinha na ponta.
    fn heading(&mut self, level: usize, number: &str, title: &str, toc: bool) {
        let (size, spec) = if level == 1 { (Self::font_size(&self.s.heading_font), &self.s.heading_font) } else { (Self::font_size(&self.s.subheading_font) - (level.saturating_sub(2) as f32) * 1.5, &self.s.subheading_font) };
        let size = size.max(8.0);
        let font = Font::family(&spec.family).bold();
        self.ensure(size * 2.6 + 30.0);
        if level > 1 {
            self.y += 7.0;
        }
        let text = format!("{number} {title}");
        let (left, y, accent) = (self.left, self.y + size, self.pal.accent);
        self.page().text(left, y, &text, font, size, accent);
        self.y += size + 3.0;
        if level == 1 {
            let (w, y) = (self.width(), self.y);
            self.page().rect(left, y, w, 1.0, Some(accent), None);
            self.page().rect(left + w - 2.0, y - 1.0, 3.0, 3.0, Some(accent), None);
            self.y += 8.0;
        } else {
            self.y += 3.0;
        }
        if toc {
            let page = self.first_page + self.pages.len() - 1;
            self.entries.push(Entry { level, text, page });
        }
    }

    fn paragraph(&mut self, text: &str, size: f32, color: Color) {
        let lines = wrap(self.body, text, size, self.width());
        for line in lines {
            self.ensure(size * 1.45);
            let (left, y, font) = (self.left, self.y + size, self.body);
            self.page().text(left, y, &line, font, size, color);
            self.y += size * 1.45;
        }
        self.y += 3.0;
    }

    fn subtitle(&mut self, text: &str) {
        self.ensure(24.0);
        let (left, y, accent) = (self.left, self.y + 8.0, self.pal.accent);
        let font = self.body.bold();
        self.page().text(left, y, text, font, 8.5, accent);
        self.y += 13.0;
    }

    /// Texto em fonte de código, quebrado por linha (definições de view e função).
    fn code(&mut self, text: &str) {
        let size = 6.5;
        let width = self.width() - 10.0;
        let mut lines: Vec<String> = Vec::new();
        for l in text.replace('\t', "    ").lines() {
            lines.extend(wrap(Font::Mono, l, size, width));
        }
        let mut i = 0;
        while i < lines.len() {
            self.ensure(size * 1.4 + 10.0);
            let room = ((self.limit() - self.y - 6.0) / (size * 1.4)).floor().max(1.0) as usize;
            let take = room.min(lines.len() - i);
            let block_h = take as f32 * size * 1.4 + 6.0;
            let (left, y, w, tint, border) = (self.left, self.y, self.width(), self.pal.tint.tint(0.55), self.pal.border);
            self.page().rect(left, y, w, block_h, Some(tint), Some((border, 0.4)));
            for (k, line) in lines[i..i + take].iter().enumerate() {
                let ty = y + 3.0 + size + k as f32 * size * 1.4 - 1.0;
                self.page().text(left + 5.0, ty, line, Font::Mono, size, TEXT);
            }
            self.y += block_h + 4.0;
            i += take;
        }
    }

    fn kv_table(&mut self, rows: &[(String, String)]) {
        if rows.is_empty() {
            return;
        }
        let widths = [self.width() * 0.28, self.width() * 0.72];
        let body: Vec<Vec<Cell>> = rows.iter().map(|(k, v)| vec![Cell::text(k), Cell::text(v)]).collect();
        self.table(&[], &widths, &body);
    }

    fn cell_height(&self, c: &Cell) -> f32 {
        let line = 7.5 * 1.35;
        let mut h = c.lines.len() as f32 * line;
        if !c.note.is_empty() {
            h += c.note.len() as f32 * 6.5 * 1.3;
        }
        if c.tag.is_some() {
            h += 10.0;
        }
        h
    }

    /// Prepara a célula para a largura da coluna: quebra o texto e o comentário.
    fn fit(&self, mut c: Cell, width: f32) -> Cell {
        let pad = 8.0 + if c.key { 12.0 } else { 0.0 };
        let avail = (width - pad).max(10.0);
        let text = c.lines.join("\n");
        c.lines = wrap(self.body, &text, 7.5, avail);
        let note = c.note.join("\n");
        if !note.is_empty() {
            c.note = wrap(self.body, &note, 6.5, avail);
        }
        c
    }

    fn table(&mut self, headers: &[&str], widths: &[f32], rows: &[Vec<Cell>]) {
        let (tint, border, accent) = (self.pal.tint, self.pal.border, self.pal.accent);
        let head_h = 15.0;
        let draw_head = |f: &mut Flow| {
            if headers.is_empty() {
                return;
            }
            let (left, y, bold) = (f.left, f.y, f.body.bold());
            let total: f32 = widths.iter().sum();
            f.page().rect(left, y, total, head_h, Some(tint), Some((border, 0.5)));
            let mut x = left;
            for (i, hd) in headers.iter().enumerate() {
                f.page().text(x + 4.0, y + 10.5, hd, bold, 7.5, TEXT);
                x += widths[i];
                if i + 1 < headers.len() {
                    f.page().line(x, y, x, y + head_h, border, 0.5);
                }
            }
            f.y += head_h;
        };
        self.ensure(head_h + 24.0);
        draw_head(self);
        for row in rows {
            let cells: Vec<Cell> = row.iter().enumerate().map(|(i, c)| self.fit(Cell { lines: c.lines.clone(), note: c.note.clone(), tag: c.tag.clone(), key: c.key, check: c.check }, widths[i])).collect();
            let row_h = cells.iter().map(|c| self.cell_height(c)).fold(0.0, f32::max) + 8.0;
            if self.y + row_h > self.limit() {
                self.new_page();
                draw_head(self);
            }
            let (left, y) = (self.left, self.y);
            let total: f32 = widths.iter().sum();
            self.page().rect(left, y, total, row_h, None, Some((border, 0.5)));
            let mut x = left;
            for (i, c) in cells.iter().enumerate() {
                let mut ty = y + 4.0;
                let font = self.body;
                for (k, line) in c.lines.iter().enumerate() {
                    ty += 7.5 * 1.35;
                    self.page().text(x + 4.0, ty - 2.2, line, font, 7.5, TEXT);
                    if c.key && k == 0 {
                        let kx = x + 5.0 + text_width(font, line, 7.5);
                        // Chave: aro e haste.
                        self.page().rect(kx, ty - 8.2, 4.0, 4.0, None, Some((accent, 0.8)));
                        self.page().line(kx + 4.0, ty - 6.2, kx + 9.0, ty - 6.2, accent, 0.8);
                        self.page().line(kx + 7.5, ty - 6.2, kx + 7.5, ty - 4.2, accent, 0.8);
                    }
                }
                for line in &c.note {
                    ty += 6.5 * 1.3;
                    self.page().text(x + 4.0, ty - 1.6, line, font, 6.5, GRAY);
                }
                if let Some(tag) = &c.tag {
                    let tw = text_width(font, tag, 6.0) + 6.0;
                    self.page().rect(x + 4.0, ty + 2.0, tw, 8.0, None, Some((accent, 0.6)));
                    self.page().text(x + 7.0, ty + 8.0, tag, font, 6.0, accent);
                }
                if c.check {
                    let cx = x + widths[i] / 2.0;
                    let cy = y + row_h / 2.0;
                    self.page().line(cx - 3.0, cy, cx - 1.0, cy + 2.5, TEXT, 1.0);
                    self.page().line(cx - 1.0, cy + 2.5, cx + 3.5, cy - 3.0, TEXT, 1.0);
                }
                x += widths[i];
                if i + 1 < cells.len() {
                    self.page().line(x, y, x, y + row_h, border, 0.5);
                }
            }
            self.y += row_h;
        }
        self.y += 8.0;
    }

    fn finish(mut self) -> (Vec<Page>, Vec<Entry>) {
        let total = self.pages.len();
        let s = self.s;
        for (i, page) in self.pages.iter_mut().enumerate() {
            if s.breadcrumbs && !self.crumbs[i].is_empty() {
                let font = Font::family(&s.breadcrumb_font.family);
                page.text(self.left, self.top + 6.0, &self.crumbs[i], font, (s.breadcrumb_font.size * 0.75).min(9.0), GRAY);
            }
            let size = (s.footer_font.size * 0.75).min(9.0);
            let font = Font::family(&s.footer_font.family);
            let y = self.h - self.bottom + 4.0;
            if s.left_footer {
                page.text(self.left, y, &s.left_footer_text, font, size, GRAY);
            }
            if s.page_number {
                let n = (self.first_page + i).to_string();
                page.text(self.w - self.right - text_width(font, &n, size), y, &n, font, size, GRAY);
            }
        }
        let _ = total;
        (self.pages, self.entries)
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------- tabelas do dicionário

fn table_section(f: &mut Flow, t: &Table) {
    if let Some(c) = t.comment.as_ref().filter(|c| !c.is_empty()) {
        f.paragraph(c, 7.5, GRAY);
    }
    let w = f.width();
    f.subtitle("Fields");
    let widths = [w * 0.07, w * 0.30, w * 0.25, w * 0.11, w * 0.27];
    let rows: Vec<Vec<Cell>> = t
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            vec![
                Cell::text(&(i + 1).to_string()),
                Cell { lines: vec![c.name.clone()], note: c.comment.iter().filter(|x| !x.is_empty()).cloned().collect(), tag: None, key: c.key, check: false },
                Cell { lines: vec![c.data_type.clone()], note: vec![], tag: c.auto_increment.then(|| "Auto Increment".to_string()), key: false, check: false },
                Cell { lines: vec![String::new()], note: vec![], tag: None, key: false, check: c.not_null },
                Cell::text(&c.default.as_ref().map(|d| format!("Default: {d}")).unwrap_or_default()),
            ]
        })
        .collect();
    f.table(&["Pos", "Name", "Type", "Not Null", "Others"], &widths, &rows);

    if !t.indexes.is_empty() {
        f.subtitle("Indexes");
        let widths = [w * 0.30, w * 0.16, w * 0.14, w * 0.40];
        let rows: Vec<Vec<Cell>> = t
            .indexes
            .iter()
            .map(|i| {
                vec![
                    Cell { lines: vec![i.name.clone()], note: i.comment.iter().filter(|x| !x.is_empty()).cloned().collect(), tag: None, key: false, check: false },
                    Cell::text(&i.kind),
                    Cell::text(&i.method),
                    Cell::text(&i.fields),
                ]
            })
            .collect();
        f.table(&["Name", "Type", "Method", "Fields"], &widths, &rows);
    }
    if !t.foreign_keys.is_empty() {
        f.subtitle("Foreign Keys");
        let widths = [w * 0.20, w * 0.14, w * 0.20, w * 0.18, w * 0.14, w * 0.14];
        let rows: Vec<Vec<Cell>> = t
            .foreign_keys
            .iter()
            .map(|k| {
                vec![
                    Cell { lines: vec![k.name.clone()], note: k.comment.iter().filter(|x| !x.is_empty()).cloned().collect(), tag: None, key: false, check: false },
                    Cell::text(&k.fields),
                    Cell::text(&k.ref_table),
                    Cell::text(&k.ref_fields),
                    Cell::text(&k.on_delete),
                    Cell::text(&k.on_update),
                ]
            })
            .collect();
        f.table(&["Name", "Fields", "Referenced Table", "Referenced Fields", "On Delete", "On Update"], &widths, &rows);
    }
    if !t.checks.is_empty() {
        f.subtitle("Checks");
        let rows: Vec<Vec<Cell>> = t.checks.iter().map(|(n, d)| vec![Cell::text(n), Cell::text(d)]).collect();
        f.table(&["Name", "Definition"], &[w * 0.30, w * 0.70], &rows);
    }
    if !t.triggers.is_empty() {
        f.subtitle("Triggers");
        let rows: Vec<Vec<Cell>> = t.triggers.iter().map(|(n, d)| vec![Cell::text(n), Cell::text(d)]).collect();
        f.table(&["Name", "Definition"], &[w * 0.30, w * 0.70], &rows);
    }
}

/// Todo o conteúdo depois da introdução: servidores, bancos, schemas e seus objetos.
fn content(f: &mut Flow, dict: &Dictionary, first_number: usize) {
    for (si, server) in dict.servers.iter().enumerate() {
        let n1 = first_number + si;
        f.crumb = format!("Server: {} ({})", server.name, server.product);
        if !f.pages.last().is_some_and(|p| p.ops.is_empty()) || si > 0 {
            f.new_page();
        }
        f.heading(1, &format!("{n1}."), &format!("Server: {} ({})", server.name, server.product), true);
        f.paragraph(&format!("{} {} listed below.", plural(server.databases.len(), "database", "databases"), if server.databases.len() == 1 { "is" } else { "are" }), 7.5, GRAY);
        if let Some(info) = &server.info {
            f.subtitle("Server Information");
            f.kv_table(info);
        }
        for (di, db) in server.databases.iter().enumerate() {
            let n2 = format!("{n1}.{}", di + 1);
            f.crumb = format!("Server: {} ({}) > Database: {}", server.name, server.product, db.name);
            f.heading(2, &format!("{n2}."), &format!("Database: {}", db.name), true);
            let tables: usize = db.schemas.iter().map(|s| s.tables.as_ref().map_or(0, Vec::len)).sum();
            let views: usize = db.schemas.iter().map(|s| s.views.as_ref().map_or(0, Vec::len)).sum();
            let funcs: usize = db.schemas.iter().map(|s| s.functions.as_ref().map_or(0, Vec::len)).sum();
            f.paragraph(&format!("Database objects to be listed: {}, {}, {}", plural(tables, "table", "tables"), plural(views, "view", "views"), plural(funcs, "function", "functions")), 7.5, GRAY);
            if let Some(info) = &db.info {
                f.subtitle("Database Information");
                f.kv_table(info);
            }
            for (ci, schema) in db.schemas.iter().enumerate() {
                let n3 = format!("{n2}.{}", ci + 1);
                f.heading(3, &format!("{n3}."), &format!("Schema: {}", schema.name), true);
                if let Some(info) = &schema.info {
                    f.subtitle("Schema Information");
                    f.kv_table(info);
                }
                let mut cat = 0;
                if let Some(tables) = &schema.tables {
                    cat += 1;
                    let n4 = format!("{n3}.{cat}");
                    f.heading(4, &format!("{n4}."), "Tables", true);
                    for (ti, t) in tables.iter().enumerate() {
                        f.heading(5, &format!("{n4}.{}.", ti + 1), &format!("Table: {}", t.name), true);
                        table_section(f, t);
                    }
                }
                if let Some(views) = &schema.views {
                    cat += 1;
                    let n4 = format!("{n3}.{cat}");
                    f.heading(4, &format!("{n4}."), "Views", true);
                    for (vi, v) in views.iter().enumerate() {
                        f.heading(5, &format!("{n4}.{}.", vi + 1), &format!("View: {}", v.name), true);
                        if let Some(c) = v.comment.as_ref().filter(|c| !c.is_empty()) {
                            f.paragraph(c, 7.5, GRAY);
                        }
                        f.subtitle("Definition");
                        f.code(&v.definition);
                    }
                }
                if let Some(functions) = &schema.functions {
                    cat += 1;
                    let n4 = format!("{n3}.{cat}");
                    f.heading(4, &format!("{n4}."), "Functions", true);
                    for (fi, func) in functions.iter().enumerate() {
                        f.heading(5, &format!("{n4}.{}.", fi + 1), &format!("Function: {}", func.name), true);
                        if let Some(c) = func.comment.as_ref().filter(|c| !c.is_empty()) {
                            f.paragraph(c, 7.5, GRAY);
                        }
                        let mut rows = vec![("Arguments".to_string(), func.arguments.clone())];
                        if let Some(r) = &func.returns {
                            rows.push(("Returns".to_string(), r.clone()));
                        }
                        rows.push(("Language".to_string(), func.language.clone()));
                        f.kv_table(&rows);
                        f.subtitle("Definition");
                        f.code(&func.body);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- sumário

fn toc_pages(entries: &[Entry], s: &Settings, pal: &Palette, first_page: usize) -> Vec<Page> {
    let (w, h) = paper_size(&s.paper, s.landscape);
    let cm = 72.0 / 2.54;
    let (left, right, top, bottom) = (s.margins[0] * cm, s.margins[1] * cm, s.margins[2] * cm, s.margins[3] * cm);
    let font = Font::Sans;
    let line_h = 11.0;
    let mut pages: Vec<Page> = Vec::new();
    let mut i = 0;
    let mut first = true;
    while i < entries.len() || first {
        let mut p = Page::new(w, h);
        let mut y = top + 22.0;
        if first {
            p.text(left, y + 6.0, "Table of Contents", Font::SansBold, 11.0, pal.accent);
            y += 12.0;
            p.rect(left, y, w - left - right, 1.0, Some(pal.accent), None);
            p.rect(left + w - left - right - 2.0, y - 1.0, 3.0, 3.0, Some(pal.accent), None);
            y += 12.0;
        }
        while i < entries.len() && y + line_h < h - bottom - 18.0 {
            let e = &entries[i];
            let indent = 6.0 * (e.level.saturating_sub(1)) as f32;
            let size = 6.5;
            let page_text = e.page.to_string();
            let pw = text_width(font, &page_text, size);
            let tw = text_width(font, &e.text, size);
            let x0 = left + indent;
            p.text(x0, y, &e.text, font, size, TEXT);
            // Pontilhado até o número da página.
            let dots_from = x0 + tw + 3.0;
            let dots_to = w - right - pw - 3.0;
            let dot_w = text_width(font, ".", size) + 1.2;
            let mut x = dots_from;
            let mut dots = String::new();
            while x + dot_w < dots_to {
                dots.push('.');
                x += dot_w;
            }
            p.text(dots_from, y, &dots.chars().flat_map(|c| [c, ' ']).take(dots.len()).collect::<String>(), font, size, Color(150, 150, 150));
            p.text(w - right - pw, y, &page_text, font, size, TEXT);
            y += line_h;
            i += 1;
        }
        first = false;
        pages.push(p);
    }
    let _ = first_page;
    pages
}

// ---------------------------------------------------------------- documento completo

/// As páginas do documento e onde está cada parte.
pub struct Rendered {
    pub pages: Vec<Page>,
    pub cover: Option<usize>,
    pub toc: Vec<usize>,
    pub intro: Option<usize>,
    pub content: usize,
}

pub fn render(dict: &Dictionary, s: &Settings) -> Rendered {
    let pal = palette(s.template);
    let has_cover = s.include_cover;
    let mut toc_count = if s.include_toc { 1 } else { 0 };
    // O sumário precisa saber em que página cada título cai, e ocupa páginas que empurram o resto: itera até estabilizar.
    for _ in 0..4 {
        let first = 1 + has_cover as usize + toc_count;
        let (pages, entries) = build_body(dict, s, first);
        let needed = if s.include_toc { toc_pages(&entries, s, &pal, first).len() } else { 0 };
        if needed == toc_count {
            let mut all: Vec<Page> = Vec::new();
            let (mut cover_i, mut toc_i, mut intro_i) = (None, Vec::new(), None);
            if has_cover {
                cover_i = Some(all.len());
                all.push(cover(dict, s, &pal));
            }
            if s.include_toc {
                let toc = decorate(toc_pages(&entries, s, &pal, first), s, 1 + has_cover as usize);
                for p in toc {
                    toc_i.push(all.len());
                    all.push(p);
                }
            }
            let intro_offset = all.len();
            if s.include_intro {
                intro_i = Some(intro_offset);
            }
            let content_start = intro_offset + usize::from(s.include_intro);
            all.extend(pages);
            return Rendered { pages: all, cover: cover_i, toc: toc_i, intro: intro_i, content: content_start };
        }
        toc_count = needed;
    }
    unreachable!("the table of contents settles in a few passes")
}

/// Rodapé nas páginas do sumário (que não passam pelo fluxo).
fn decorate(mut pages: Vec<Page>, s: &Settings, first_number: usize) -> Vec<Page> {
    let cm = 72.0 / 2.54;
    for (i, p) in pages.iter_mut().enumerate() {
        let (left, right, bottom) = (s.margins[0] * cm, s.margins[1] * cm, s.margins[3] * cm);
        let size = (s.footer_font.size * 0.75).min(9.0);
        let font = Font::family(&s.footer_font.family);
        let y = p.height - bottom + 4.0;
        if s.left_footer {
            p.text(left, y, &s.left_footer_text, font, size, GRAY);
        }
        if s.page_number {
            let n = (first_number + i + 1).to_string();
            p.text(p.width - right - text_width(font, &n, size), y, &n, font, size, GRAY);
        }
    }
    pages
}

/// Introdução e conteúdo, numerados a partir de `first_page`.
fn build_body(dict: &Dictionary, s: &Settings, first_page: usize) -> (Vec<Page>, Vec<Entry>) {
    let mut f = Flow::new(s, first_page);
    let mut first_number = 1;
    if s.include_intro {
        f.heading(1, "1.", "Introduction", true);
        let text = fill_placeholders(&s.intro_text, s, dict);
        for para in text.split('\n') {
            if para.is_empty() {
                f.y += 4.0;
            } else {
                f.paragraph(para, 7.0, TEXT);
            }
        }
        first_number = 2;
    }
    content(&mut f, dict, first_number);
    f.finish()
}

/// Um dicionário de exemplo para a pré-visualização das abas de layout.
pub fn sample_dictionary() -> Dictionary {
    let col = |name: &str, ty: &str, key: bool, auto: bool, not_null: bool, default: Option<&str>| Column {
        name: name.into(),
        comment: Some(format!("comment_{}", name.trim_start_matches("field_"))),
        data_type: ty.into(),
        not_null,
        key,
        auto_increment: auto,
        default: default.map(String::from),
    };
    Dictionary {
        servers: vec![Server {
            name: "Server_1 (Demo)".into(),
            product: "PostgreSQL".into(),
            info: None,
            databases: vec![Database {
                name: "Database_1".into(),
                info: None,
                schemas: vec![Schema {
                    name: "public".into(),
                    info: None,
                    tables: Some(vec![Table {
                        name: "Sample Table".into(),
                        comment: None,
                        columns: vec![
                            col("field_1", "int4(32)", true, true, true, Some("default_1")),
                            col("field_2", "varchar(50)", false, true, true, Some("default_2")),
                            col("field_3", "varchar(50)", false, false, true, None),
                            col("field_4", "varchar(255)", false, false, true, Some("default_4")),
                            col("field_5", "timestamp(6)", false, false, true, None),
                            col("field_6", "timestamp(6)", false, false, false, None),
                        ],
                        indexes: vec![Index { name: "index_1".into(), comment: Some("comment_1".into()), kind: "NORMAL".into(), method: "HASH".into(), fields: "field_1".into() }],
                        foreign_keys: vec![
                            ForeignKey { name: "foreignKey_1".into(), comment: Some("comment_1".into()), fields: "field_1".into(), ref_table: "table_1".into(), ref_fields: "field_1".into(), on_delete: "RESTRICT".into(), on_update: "CASCADE".into() },
                            ForeignKey { name: "foreignKey_2".into(), comment: Some("comment_2".into()), fields: "field_2".into(), ref_table: "table_1".into(), ref_fields: "field_2".into(), on_delete: "RESTRICT".into(), on_update: "CASCADE".into() },
                        ],
                        checks: vec![],
                        triggers: vec![],
                    }]),
                    views: None,
                    functions: None,
                }],
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Op;

    fn text_of(page: &Page) -> String {
        page.ops.iter().filter_map(|o| if let Op::Text { text, .. } = o { Some(text.as_str()) } else { None }).collect::<Vec<_>>().join(" ")
    }

    fn big_dictionary(tables: usize) -> Dictionary {
        let mut d = sample_dictionary();
        let t = d.servers[0].databases[0].schemas[0].tables.as_ref().unwrap()[0].clone();
        d.servers[0].databases[0].schemas[0].tables = Some((0..tables).map(|i| Table { name: format!("table_{i}"), ..t.clone() }).collect());
        d
    }

    #[test]
    fn renders_cover_toc_intro_and_content_in_order() {
        let mut s = Settings::defaults("atlas_demo");
        s.now = [2026, 10, 23, 16, 40, 55];
        let r = render(&sample_dictionary(), &s);
        assert_eq!(r.cover, Some(0));
        assert_eq!(r.toc, vec![1]);
        assert_eq!(r.intro, Some(2));
        assert_eq!(r.content, 3);
        let cover = text_of(&r.pages[0]);
        assert!(cover.contains("atlas_demo") && cover.contains("Data Dictionary") && cover.contains("Date: 2026-10-23 16:40:55") && cover.contains("Server: Server_1 (Demo)"), "{cover}");
        let toc = text_of(&r.pages[1]);
        assert!(toc.contains("Table of Contents") && toc.contains("1. Introduction") && toc.contains("Table: Sample Table"), "{toc}");
        assert!(text_of(&r.pages[2]).contains("Server_1 (Demo) (PostgreSQL)"), "o marcador da lista de servidores vira a lista");
        let body = text_of(&r.pages[3]);
        assert!(body.contains("2. Server: Server_1 (Demo) (PostgreSQL)") && body.contains("Fields") && body.contains("Auto Increment") && body.contains("Foreign Keys") && body.contains("Default: default_1"), "{body}");
    }

    #[test]
    fn page_numbers_in_the_toc_match_where_headings_land() {
        let s = Settings::defaults("x");
        let r = render(&big_dictionary(60), &s);
        assert!(r.pages.len() > 6, "{} páginas", r.pages.len());
        // O título da última tabela aparece na página que o sumário diz.
        let toc: String = r.toc.iter().map(|i| text_of(&r.pages[*i])).collect::<Vec<_>>().join(" ");
        assert!(toc.contains("Table: table_59"));
        let page_of = r.pages.iter().skip(r.content).position(|p| text_of(p).contains("Table: table_59")).unwrap() + r.content;
        // A página física é a posição mais um; o número impresso no fim da linha do sumário é o mesmo.
        let line = r.toc.iter().flat_map(|i| r.pages[*i].ops.iter()).collect::<Vec<_>>();
        let idx = line.iter().position(|o| matches!(o, Op::Text { text, .. } if text.contains("Table: table_59"))).unwrap();
        let number = line[idx..].iter().find_map(|o| if let Op::Text { text, .. } = o { text.parse::<usize>().ok() } else { None }).unwrap();
        assert_eq!(number, page_of + 1, "sumário e página precisam concordar");
    }

    #[test]
    fn switches_remove_the_cover_toc_and_intro() {
        let mut s = Settings::defaults("x");
        s.include_cover = false;
        s.include_toc = false;
        s.include_intro = false;
        let r = render(&sample_dictionary(), &s);
        assert_eq!((r.cover, r.toc.len(), r.intro, r.content), (None, 0, None, 0));
        assert!(text_of(&r.pages[0]).contains("1. Server: Server_1 (Demo) (PostgreSQL)"));
    }

    #[test]
    fn landscape_paper_and_footer_options_apply() {
        let mut s = Settings::defaults("x");
        s.landscape = true;
        s.paper = "Letter".into();
        s.left_footer_text = "MeuRodape".into();
        s.page_number = false;
        let r = render(&sample_dictionary(), &s);
        assert_eq!((r.pages[0].width, r.pages[0].height), (792.0, 612.0));
        let body = text_of(&r.pages[r.content]);
        assert!(body.ends_with("MeuRodape"), "sem número de página: {body}");
    }

    #[test]
    fn every_template_draws_a_cover() {
        for t in [Template::Orange, Template::Red, Template::BlackWhite, Template::Blue, Template::Green] {
            let mut s = Settings::defaults("db");
            s.template = t;
            let r = render(&sample_dictionary(), &s);
            let svg = r.pages[0].to_svg();
            assert!(svg.contains(&palette(t).accent.hex()), "{t:?}");
        }
    }
}
