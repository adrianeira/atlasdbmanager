//! Escritor de PDF mínimo: páginas com texto nas fontes padrão (sem incorporar nada), retângulos, linhas e polígonos, com
//! transparência. O texto sai em WinAnsi (Latin-1 mais as aspas e travessões do Windows).

use crate::fonts::Font;
use crate::page::{Color, Op, Page};
use std::fmt::Write as _;

/// O caractere no código WinAnsi, ou `?` se não existe.
fn win_ansi(c: char) -> u8 {
    match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u32 as u8,
        0x2022 => 0x95,
        0x2013 => 0x96,
        0x2014 => 0x97,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201C => 0x93,
        0x201D => 0x94,
        0x2026 => 0x85,
        0x20AC => 0x80,
        0x09 => b' ',
        _ => b'?',
    }
}

fn pdf_string(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let b = win_ansi(c);
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn rgb(c: Color) -> String {
    format!("{} {} {}", num(c.0 as f32 / 255.0), num(c.1 as f32 / 255.0), num(c.2 as f32 / 255.0))
}

/// Índice da transparência na lista (cada valor de `alpha` vira um `ExtGState`).
fn alpha_index(alphas: &mut Vec<f32>, alpha: f32) -> usize {
    if let Some(i) = alphas.iter().position(|a| (*a - alpha).abs() < 0.001) {
        return i;
    }
    alphas.push(alpha);
    alphas.len() - 1
}

fn content(page: &Page, alphas: &mut Vec<f32>) -> Vec<u8> {
    let h = page.height;
    let mut out: Vec<u8> = Vec::new();
    for op in &page.ops {
        match op {
            Op::Rect { x, y, w, h: rh, fill, stroke, alpha } => {
                let mut s = String::from("q ");
                if *alpha < 0.999 {
                    let _ = write!(s, "/G{} gs ", alpha_index(alphas, *alpha));
                }
                if let Some(c) = fill {
                    let _ = write!(s, "{} rg ", rgb(*c));
                }
                if let Some((c, wd)) = stroke {
                    let _ = write!(s, "{} RG {} w ", rgb(*c), num(*wd));
                }
                let paint = match (fill.is_some(), stroke.is_some()) {
                    (true, true) => "B",
                    (true, false) => "f",
                    (false, true) => "S",
                    (false, false) => "n",
                };
                let _ = writeln!(s, "{} {} {} {} re {paint} Q\n", num(*x), num(h - y - rh), num(*w), num(*rh));
                out.extend(s.bytes());
            }
            Op::Line { x1, y1, x2, y2, color, width } => {
                let s = format!("q {} RG {} w {} {} m {} {} l S Q\n", rgb(*color), num(*width), num(*x1), num(h - y1), num(*x2), num(h - y2));
                out.extend(s.bytes());
            }
            Op::Poly { points, fill, alpha } => {
                if points.len() < 3 {
                    continue;
                }
                let mut s = String::from("q ");
                if *alpha < 0.999 {
                    let _ = write!(s, "/G{} gs ", alpha_index(alphas, *alpha));
                }
                let _ = write!(s, "{} rg ", rgb(*fill));
                for (i, (px, py)) in points.iter().enumerate() {
                    let _ = write!(s, "{} {} {} ", num(*px), num(h - py), if i == 0 { "m" } else { "l" });
                }
                s.push_str("f Q\n");
                out.extend(s.bytes());
            }
            Op::Text { x, y, text, font, size, color } => {
                out.extend(format!("BT {} rg /{} {} Tf {} {} Td ", rgb(*color), font.resource(), num(*size), num(*x), num(h - y)).bytes());
                out.extend(pdf_string(text));
                out.extend(b" Tj ET\n");
            }
        }
    }
    out
}

/// O PDF de todas as páginas (cada uma com o próprio tamanho).
pub fn write_pdf(pages: &[Page], title: &str) -> Vec<u8> {
    // Objetos: 1 catálogo, 2 páginas, 3 info, 4..9 fontes, depois (página, conteúdo) para cada página, depois as transparências.
    let mut alphas: Vec<f32> = Vec::new();
    let streams: Vec<Vec<u8>> = pages.iter().map(|p| content(p, &mut alphas)).collect();
    let first_page = 10;
    let first_alpha = first_page + pages.len() * 2;
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids = (0..pages.len()).map(|i| format!("{} 0 R", first_page + i * 2)).collect::<Vec<_>>().join(" ");
    objects.push(format!("<< /Type /Pages /Count {} /Kids [{kids}] >>", pages.len()).into_bytes());
    let mut info = b"<< /Producer (Atlas DB Manager) /Title ".to_vec();
    info.extend(pdf_string(title));
    info.extend(b" >>");
    objects.push(info);
    for f in Font::ALL {
        objects.push(format!("<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>", f.base_font()).into_bytes());
    }
    let fonts = Font::ALL.iter().enumerate().map(|(i, f)| format!("/{} {} 0 R", f.resource(), 4 + i)).collect::<Vec<_>>().join(" ");
    let gstates = (0..alphas.len()).map(|i| format!("/G{i} {} 0 R", first_alpha + i)).collect::<Vec<_>>().join(" ");
    for (i, page) in pages.iter().enumerate() {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {fonts} >> /ExtGState << {gstates} >> >> /Contents {} 0 R >>",
                num(page.width),
                num(page.height),
                first_page + i * 2 + 1
            )
            .into_bytes(),
        );
        let mut s = format!("<< /Length {} >>\nstream\n", streams[i].len()).into_bytes();
        s.extend(&streams[i]);
        s.extend(b"\nendstream");
        objects.push(s);
    }
    for a in &alphas {
        objects.push(format!("<< /Type /ExtGState /ca {} /CA {} >>", num(*a), num(*a)).into_bytes());
    }

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_well_formed_pdf_with_text_shapes_and_transparency() {
        let mut p = Page::new(595.0, 842.0);
        p.text(50.0, 80.0, "Olá (mundo) \\ — ok", Font::SansBold, 14.0, Color(200, 100, 0));
        p.rect(50.0, 100.0, 200.0, 30.0, Some(Color(250, 230, 200)), Some((Color::BLACK, 0.5)));
        p.line(50.0, 140.0, 250.0, 140.0, Color::BLACK, 1.0);
        p.poly(&[(0.0, 0.0), (100.0, 0.0), (50.0, 80.0)], Color(255, 0, 0), 0.4);
        let pdf = write_pdf(&[p.clone(), p], "Teste");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4") && text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 2") && text.contains("/Title (Teste)"));
        assert!(text.contains("(Ol\u{fffd}") || text.contains("\\(mundo\\) \\\\"), "{text}");
        assert!(text.contains("/ca 0.4"), "a transparência vira um ExtGState");
        // O xref aponta para os objetos de verdade.
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref"));
    }

    #[test]
    fn svg_has_the_same_content() {
        let mut p = Page::new(100.0, 100.0);
        p.text(1.0, 2.0, "a < b & c", Font::Sans, 10.0, Color::BLACK);
        let svg = p.to_svg();
        assert!(svg.contains("a &lt; b &amp; c") && svg.starts_with("<svg"));
    }
}
