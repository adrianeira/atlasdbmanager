//! Fontes padrão do PDF (Helvetica, Times e Courier) e a largura dos textos.
//!
//! Só as larguras de Helvetica e Helvetica-Bold (tabelas AFM, em milésimos de em) são exatas; Times usa as de Helvetica
//! reduzidas e Courier tem largura fixa. Letras acentuadas valem o mesmo que a letra-base.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Sans,
    SansBold,
    Serif,
    SerifBold,
    Mono,
    MonoBold,
}

impl Font {
    /// Nome PDF da fonte padrão.
    pub fn base_font(self) -> &'static str {
        match self {
            Font::Sans => "Helvetica",
            Font::SansBold => "Helvetica-Bold",
            Font::Serif => "Times-Roman",
            Font::SerifBold => "Times-Bold",
            Font::Mono => "Courier",
            Font::MonoBold => "Courier-Bold",
        }
    }

    /// Nome do recurso na página (`/F1`...).
    pub fn resource(self) -> &'static str {
        match self {
            Font::Sans => "F1",
            Font::SansBold => "F2",
            Font::Serif => "F3",
            Font::SerifBold => "F4",
            Font::Mono => "F5",
            Font::MonoBold => "F6",
        }
    }

    pub const ALL: [Font; 6] = [Font::Sans, Font::SansBold, Font::Serif, Font::SerifBold, Font::Mono, Font::MonoBold];

    pub fn bold(self) -> Font {
        match self {
            Font::Sans | Font::SansBold => Font::SansBold,
            Font::Serif | Font::SerifBold => Font::SerifBold,
            Font::Mono | Font::MonoBold => Font::MonoBold,
        }
    }

    pub fn regular(self) -> Font {
        match self {
            Font::Sans | Font::SansBold => Font::Sans,
            Font::Serif | Font::SerifBold => Font::Serif,
            Font::Mono | Font::MonoBold => Font::Mono,
        }
    }

    /// Família escolhida na tela: "Default"/"Arial" (sem serifa), "Times New Roman" ou "Courier New".
    pub fn family(name: &str) -> Font {
        match name {
            "Times New Roman" | "Times" => Font::Serif,
            "Courier New" | "Courier" => Font::Mono,
            _ => Font::Sans,
        }
    }
}

// Larguras de U+0020 a U+007E.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // espaço ! " # $ % & ' ( ) * + , - . /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // : ; < = > ? @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // [ \ ] ^ _ `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // a-z
    334, 260, 334, 584, // { | } ~
];

const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, //
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, //
    333, 333, 584, 584, 584, 611, 975, //
    722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, //
    333, 278, 333, 584, 556, 333, //
    556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, //
    389, 280, 389, 584, //
];

/// A letra sem acento (só para medir).
fn base_letter(c: char) -> char {
    match c {
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' | 'Ø' => 'O',
        'Ù'..='Ü' => 'U',
        'Ý' => 'Y',
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' | 'ø' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// Largura de um caractere em milésimos de em.
fn glyph(font: Font, c: char) -> f32 {
    let c = base_letter(c);
    let table = match font {
        Font::Mono | Font::MonoBold => return 600.0,
        Font::Sans | Font::Serif => &HELVETICA,
        Font::SansBold | Font::SerifBold => &HELVETICA_BOLD,
    };
    let w = match c as u32 {
        0x20..=0x7E => table[c as usize - 0x20] as f32,
        0xA0 => 278.0,
        // Travessão, aspas e reticências.
        0x2013 => 556.0,
        0x2014 | 0x2026 => 1000.0,
        0x2018 | 0x2019 => 222.0,
        0x201C | 0x201D => 333.0,
        0x2022 => 350.0,
        0x2713 => 556.0,
        _ => 556.0,
    };
    if matches!(font, Font::Serif | Font::SerifBold) { w * 0.92 } else { w }
}

/// Largura do texto em pontos.
pub fn text_width(font: Font, text: &str, size: f32) -> f32 {
    text.chars().map(|c| glyph(font, c)).sum::<f32>() * size / 1000.0
}

/// Quebra o texto em linhas de até `width` pontos (nas palavras; uma palavra maior que a linha é cortada por caractere).
pub fn wrap(font: Font, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_width(font, &candidate, size) <= width || line.is_empty() && word.is_empty() {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Palavra sozinha maior que a linha: corta.
            let mut rest = word.to_string();
            while text_width(font, &rest, size) > width && rest.chars().count() > 1 {
                let mut cut = String::new();
                for c in rest.chars() {
                    if text_width(font, &format!("{cut}{c}"), size) > width && !cut.is_empty() {
                        break;
                    }
                    cut.push(c);
                }
                rest = rest.chars().skip(cut.chars().count()).collect();
                lines.push(cut);
            }
            line = rest;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_afm_tables() {
        assert!((text_width(Font::Sans, "Hello", 10.0) - 22.78).abs() < 0.01, "{}", text_width(Font::Sans, "Hello", 10.0));
        assert_eq!(text_width(Font::Mono, "abcd", 10.0), 24.0);
        assert!(text_width(Font::SansBold, "Hello", 10.0) > text_width(Font::Sans, "Hello", 10.0));
        assert_eq!(text_width(Font::Sans, "é", 10.0), text_width(Font::Sans, "e", 10.0));
    }

    #[test]
    fn wrapping_breaks_on_words_and_cuts_long_ones() {
        let lines = wrap(Font::Sans, "um dois tres quatro cinco", 10.0, 40.0);
        assert!(lines.len() > 1 && lines.iter().all(|l| text_width(Font::Sans, l, 10.0) <= 40.0), "{lines:?}");
        let cut = wrap(Font::Sans, "supercalifragilisticexpialidocious", 10.0, 40.0);
        assert!(cut.len() > 1 && cut.concat() == "supercalifragilisticexpialidocious", "{cut:?}");
        assert_eq!(wrap(Font::Sans, "a\nb", 10.0, 100.0), vec!["a", "b"]);
    }
}
