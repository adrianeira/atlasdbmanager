//! Páginas como lista de desenho (texto, retângulos, linhas e polígonos), com saída em SVG; o PDF usa a mesma lista.
//! As coordenadas são em pontos, com a origem no canto superior esquerdo da página.

use crate::fonts::Font;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const BLACK: Color = Color(0, 0, 0);
    pub const WHITE: Color = Color(255, 255, 255);

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// Mistura com branco: `amount` 0 = a cor, 1 = branco.
    pub fn tint(self, amount: f32) -> Color {
        let m = |c: u8| (c as f32 + (255.0 - c as f32) * amount).round().clamp(0.0, 255.0) as u8;
        Color(m(self.0), m(self.1), m(self.2))
    }
}

#[derive(Debug, Clone)]
pub enum Op {
    Rect { x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<(Color, f32)>, alpha: f32 },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: Color, width: f32 },
    Poly { points: Vec<(f32, f32)>, fill: Color, alpha: f32 },
    /// `y` é a linha de base do texto.
    Text { x: f32, y: f32, text: String, font: Font, size: f32, color: Color },
}

#[derive(Debug, Clone)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

impl Page {
    pub fn new(width: f32, height: f32) -> Self {
        Page { width, height, ops: Vec::new() }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color>, stroke: Option<(Color, f32)>) {
        self.ops.push(Op::Rect { x, y, w, h, fill, stroke, alpha: 1.0 });
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: Color, width: f32) {
        self.ops.push(Op::Line { x1, y1, x2, y2, color, width });
    }

    pub fn poly(&mut self, points: &[(f32, f32)], fill: Color, alpha: f32) {
        self.ops.push(Op::Poly { points: points.to_vec(), fill, alpha });
    }

    pub fn text(&mut self, x: f32, y: f32, text: &str, font: Font, size: f32, color: Color) {
        if !text.is_empty() {
            self.ops.push(Op::Text { x, y, text: text.to_string(), font, size, color });
        }
    }

    /// A página como um SVG (a pré-visualização da tela).
    pub fn to_svg(&self) -> String {
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>",
            w = self.width,
            h = self.height
        );
        for op in &self.ops {
            match op {
                Op::Rect { x, y, w, h, fill, stroke, alpha } => {
                    let fill_attr = fill.map(|c| c.hex()).unwrap_or_else(|| "none".into());
                    let stroke_attr = stroke.map(|(c, wd)| format!(" stroke=\"{}\" stroke-width=\"{wd}\"", c.hex())).unwrap_or_default();
                    out.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"{fill_attr}\" fill-opacity=\"{alpha}\"{stroke_attr}/>"));
                }
                Op::Line { x1, y1, x2, y2, color, width } => {
                    out.push_str(&format!("<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{}\" stroke-width=\"{width}\"/>", color.hex()));
                }
                Op::Poly { points, fill, alpha } => {
                    let pts = points.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(" ");
                    out.push_str(&format!("<polygon points=\"{pts}\" fill=\"{}\" fill-opacity=\"{alpha}\"/>", fill.hex()));
                }
                Op::Text { x, y, text, font, size, color } => {
                    let (family, weight) = match font {
                        Font::Sans => ("Helvetica, Arial, sans-serif", "normal"),
                        Font::SansBold => ("Helvetica, Arial, sans-serif", "bold"),
                        Font::Serif => ("Times New Roman, serif", "normal"),
                        Font::SerifBold => ("Times New Roman, serif", "bold"),
                        Font::Mono => ("Courier New, monospace", "normal"),
                        Font::MonoBold => ("Courier New, monospace", "bold"),
                    };
                    out.push_str(&format!(
                        "<text x=\"{x}\" y=\"{y}\" font-family=\"{family}\" font-weight=\"{weight}\" font-size=\"{size}\" fill=\"{}\" xml:space=\"preserve\">{}</text>",
                        color.hex(),
                        escape(text)
                    ));
                }
            }
        }
        out.push_str("</svg>");
        out
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
