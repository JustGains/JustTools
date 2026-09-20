//! Positioned page text, used to title links and to find URLs typed as text.
//!
//! This follows the text-showing operators closely enough to place each glyph
//! (fonts, text and graphics state, and form XObjects); it does not render.

use super::{dictionary, pdf_number, resolve_object};
use lopdf::{Dictionary, Document, Encoding, Object, ObjectId};
use std::collections::HashMap;
use std::rc::Rc;

/// Largest decoded content stream read for one page or form.
const MAX_CONTENT_BYTES: usize = 64 << 20;
/// Glyph budget per page; guards against pathological content.
const MAX_GLYPHS: usize = 500_000;
const MAX_FORM_DEPTH: usize = 12;
/// Gap, as a fraction of the font size, read as a word break.
const WORD_GAP: f64 = 0.2;

/// A decoded glyph placed in default user space (the space of annotation rectangles).
#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub(crate) text: String,
    origin: (f64, f64),
    end: (f64, f64),
    center: (f64, f64),
    size: f64,
}

/// Every glyph drawn on a page, in content order. Unreadable content yields
/// the glyphs found before the problem.
pub(crate) fn page_glyphs(document: &Document, page_id: ObjectId) -> Vec<Glyph> {
    let mut walker = Walker {
        document,
        fonts: HashMap::new(),
        glyphs: Vec::new(),
    };
    if let Ok(content) = document.get_page_content_with_limit(page_id, MAX_CONTENT_BYTES) {
        walker.run(
            &content,
            page_resources(document, page_id),
            Matrix::IDENTITY,
            0,
        );
    }
    walker.glyphs
}

/// Text drawn inside `rect` (`[x1 y1 x2 y2]`), judged by each glyph's center.
pub(crate) fn text_inside(glyphs: &[Glyph], rect: [f64; 4]) -> String {
    let (left, right) = (rect[0].min(rect[2]) - 1.0, rect[0].max(rect[2]) + 1.0);
    let (bottom, top) = (rect[1].min(rect[3]) - 1.0, rect[1].max(rect[3]) + 1.0);
    let inside: Vec<&Glyph> = glyphs
        .iter()
        .filter(|glyph| {
            let (x, y) = glyph.center;
            (left..=right).contains(&x) && (bottom..=top).contains(&y)
        })
        .collect();
    join(&inside).join(" ")
}

/// Page text as lines, joining glyphs that share a baseline.
pub(crate) fn lines(glyphs: &[Glyph]) -> Vec<String> {
    join(&glyphs.iter().collect::<Vec<_>>())
}

/// Joins glyphs into lines in content order, inserting spaces at word gaps.
fn join(glyphs: &[&Glyph]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut previous: Option<&Glyph> = None;
    for glyph in glyphs {
        if let Some(before) = previous {
            let size = glyph.size.max(before.size).max(f64::EPSILON);
            let same_line = (glyph.origin.1 - before.origin.1).abs() <= size * 0.5
                && glyph.origin.0 >= before.origin.0 - size * 0.5;
            if !same_line {
                lines.push(std::mem::take(&mut line));
            } else if glyph.origin.0 - before.end.0 > size * WORD_GAP
                && !line.ends_with(char::is_whitespace)
                && !glyph.text.starts_with(char::is_whitespace)
            {
                line.push(' ');
            }
        }
        line.push_str(&glyph.text);
        previous = Some(glyph);
    }
    lines.push(line);
    lines
        .into_iter()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Matrix([f64; 6]);

impl Matrix {
    const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn translate(x: f64, y: f64) -> Self {
        Self([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// `self` followed by `other`, in PDF's row-vector convention.
    fn then(self, other: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [p, q, r, s, t, u] = other.0;
        Self([
            a * p + b * r,
            a * q + b * s,
            c * p + d * r,
            c * q + d * s,
            e * p + f * r + t,
            e * q + f * s + u,
        ])
    }

    fn apply(self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    fn from_operands(operands: &[Object]) -> Option<Self> {
        let values: Vec<f64> = operands.iter().filter_map(pdf_number).collect();
        <[f64; 6]>::try_from(values).ok().map(Self)
    }

    fn from_numbers(operands: &[Operand]) -> Option<Self> {
        let values: Vec<f64> = operands.iter().filter_map(Operand::number).collect();
        <[f64; 6]>::try_from(values).ok().map(Self)
    }
}

/// Content-stream operands the walker reads; anything else is `Other`.
/// Numbers stay unparsed until an operator needs them: most belong to paths.
#[derive(Debug, PartialEq)]
enum Operand<'c> {
    Number(&'c [u8]),
    Name(&'c [u8]),
    Text(Vec<u8>),
    Array(Vec<Operand<'c>>),
    Other,
}

impl Operand<'_> {
    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(raw) => std::str::from_utf8(raw).ok()?.parse().ok(),
            _ => None,
        }
    }
}

enum Lexeme<'c> {
    Operand(Operand<'c>),
    Operator(&'c [u8]),
    ArrayEnd,
}

/// Array nesting honored before inner arrays are skipped.
const MAX_ARRAY_DEPTH: usize = 16;

/// Calls `handle` with each operator and its operands until it returns
/// false. Vector-heavy pages hold megabytes of path operators, so this lexer
/// leaves numbers unparsed and only allocates for strings and arrays.
fn for_each_operation<'c>(
    content: &'c [u8],
    mut handle: impl FnMut(&[u8], &[Operand<'c>]) -> bool,
) {
    let mut lexer = Lexer {
        data: content,
        at: 0,
        depth: 0,
    };
    let mut operands = Vec::new();
    while let Some(lexeme) = lexer.next() {
        match lexeme {
            Lexeme::Operand(operand) => operands.push(operand),
            Lexeme::Operator(b"BI") => {
                lexer.skip_inline_image();
                operands.clear();
            }
            Lexeme::Operator(operator) => {
                if !handle(operator, &operands) {
                    return;
                }
                operands.clear();
            }
            Lexeme::ArrayEnd => {}
        }
    }
}

/// Byte classes for the lexer: 1 is whitespace, 2 is a delimiter.
const CLASSES: [u8; 256] = {
    let mut classes = [0u8; 256];
    let spaces = b"\0\t\n\x0c\r ";
    let mut index = 0;
    while index < spaces.len() {
        classes[spaces[index] as usize] = 1;
        index += 1;
    }
    let delimiters = b"()<>[]{}/%";
    index = 0;
    while index < delimiters.len() {
        classes[delimiters[index] as usize] = 2;
        index += 1;
    }
    classes
};

fn is_space(byte: u8) -> bool {
    CLASSES[byte as usize] == 1
}

fn is_delimiter(byte: u8) -> bool {
    CLASSES[byte as usize] == 2
}

struct Lexer<'c> {
    data: &'c [u8],
    at: usize,
    depth: usize,
}

impl<'c> Lexer<'c> {
    fn next(&mut self) -> Option<Lexeme<'c>> {
        self.skip_space();
        let byte = *self.data.get(self.at)?;
        Some(match byte {
            b'(' => Lexeme::Operand(Operand::Text(self.literal())),
            b'<' if self.data.get(self.at + 1) == Some(&b'<') => {
                self.skip_dictionary();
                Lexeme::Operand(Operand::Other)
            }
            b'<' => Lexeme::Operand(Operand::Text(self.hex())),
            b'[' => {
                self.at += 1;
                Lexeme::Operand(self.array())
            }
            b']' => {
                self.at += 1;
                Lexeme::ArrayEnd
            }
            b'/' => {
                self.at += 1;
                Lexeme::Operand(Operand::Name(self.word()))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => Lexeme::Operand(Operand::Number(self.word())),
            b')' | b'>' | b'{' | b'}' => {
                self.at += 1;
                Lexeme::Operand(Operand::Other)
            }
            _ => match self.word() {
                b"true" | b"false" | b"null" => Lexeme::Operand(Operand::Other),
                word => Lexeme::Operator(word),
            },
        })
    }

    fn skip_space(&mut self) {
        while let Some(&byte) = self.data.get(self.at) {
            if is_space(byte) {
                self.at += 1;
            } else if byte == b'%' {
                while self
                    .data
                    .get(self.at)
                    .is_some_and(|byte| !matches!(byte, b'\n' | b'\r'))
                {
                    self.at += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &'c [u8] {
        let start = self.at;
        while self
            .data
            .get(self.at)
            .is_some_and(|byte| !is_space(*byte) && !is_delimiter(*byte))
        {
            self.at += 1;
        }
        &self.data[start..self.at]
    }

    fn array(&mut self) -> Operand<'c> {
        if self.depth >= MAX_ARRAY_DEPTH {
            return Operand::Other;
        }
        self.depth += 1;
        let mut items = Vec::new();
        while let Some(lexeme) = self.next() {
            match lexeme {
                Lexeme::ArrayEnd => break,
                Lexeme::Operand(operand) => items.push(operand),
                Lexeme::Operator(_) => {}
            }
        }
        self.depth -= 1;
        Operand::Array(items)
    }

    /// A `( … )` string with escapes and balanced parentheses decoded.
    fn literal(&mut self) -> Vec<u8> {
        self.at += 1;
        let mut bytes = Vec::new();
        let mut depth = 1usize;
        while let Some(&byte) = self.data.get(self.at) {
            self.at += 1;
            match byte {
                b'\\' => {
                    let Some(&escaped) = self.data.get(self.at) else {
                        break;
                    };
                    self.at += 1;
                    match escaped {
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'b' => bytes.push(8),
                        b'f' => bytes.push(12),
                        b'0'..=b'7' => {
                            let mut value = u32::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.data.get(self.at) {
                                    Some(&digit @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(digit - b'0');
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            bytes.push(value as u8);
                        }
                        // A backslash before a line break continues the string.
                        b'\r' => {
                            if self.data.get(self.at) == Some(&b'\n') {
                                self.at += 1;
                            }
                        }
                        b'\n' => {}
                        other => bytes.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    bytes.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    bytes.push(byte);
                }
                _ => bytes.push(byte),
            }
        }
        bytes
    }

    fn hex(&mut self) -> Vec<u8> {
        self.at += 1;
        let mut digits = Vec::new();
        while let Some(&byte) = self.data.get(self.at) {
            self.at += 1;
            if byte == b'>' {
                break;
            }
            if let Some(digit) = (byte as char).to_digit(16) {
                digits.push(digit as u8);
            }
        }
        digits
            .chunks(2)
            .map(|pair| pair[0] << 4 | pair.get(1).copied().unwrap_or(0))
            .collect()
    }

    fn skip_dictionary(&mut self) {
        let mut depth = 0usize;
        while self.at < self.data.len() {
            match &self.data[self.at..] {
                [b'<', b'<', ..] => {
                    depth += 1;
                    self.at += 2;
                }
                [b'>', b'>', ..] => {
                    self.at += 2;
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                [b'(', ..] => {
                    self.literal();
                }
                _ => self.at += 1,
            }
        }
    }

    /// Skips `BI … ID <data> EI`; the data ends at `EI` between whitespace.
    fn skip_inline_image(&mut self) {
        while let Some(lexeme) = self.next() {
            if matches!(lexeme, Lexeme::Operator(b"ID")) {
                break;
            }
        }
        let data = self.data;
        let mut index = self.at + 1;
        while index + 1 < data.len() {
            if &data[index..index + 2] == b"EI"
                && is_space(data[index - 1])
                && data
                    .get(index + 2)
                    .is_none_or(|byte| is_space(*byte) || is_delimiter(*byte))
            {
                self.at = index + 2;
                return;
            }
            index += 1;
        }
        self.at = data.len();
    }
}

enum Widths {
    Simple {
        first: u32,
        widths: Vec<f64>,
        missing: f64,
    },
    Composite {
        /// Inclusive code ranges sorted by start.
        ranges: Vec<(u32, u32, f64)>,
        default: f64,
    },
}

impl Widths {
    fn get(&self, code: u32) -> f64 {
        match self {
            Self::Simple {
                first,
                widths,
                missing,
            } => code
                .checked_sub(*first)
                .and_then(|index| widths.get(index as usize))
                .copied()
                .unwrap_or(*missing),
            Self::Composite { ranges, default } => {
                let index = ranges.partition_point(|(start, _, _)| *start <= code);
                index
                    .checked_sub(1)
                    .map(|index| ranges[index])
                    .filter(|(_, end, _)| code <= *end)
                    .map_or(*default, |(_, _, width)| width)
            }
        }
    }
}

struct Font<'a> {
    code_bytes: usize,
    unicode: Option<HashMap<u32, String>>,
    encoding: Option<Encoding<'a>>,
    widths: Widths,
    /// Text-space units per width unit: 1/1000, or a Type3 font's matrix.
    scale: f64,
}

impl<'a> Font<'a> {
    fn new(document: &'a Document, font: &'a Dictionary) -> Self {
        let subtype = font
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or(b"");
        let composite = subtype == b"Type0";
        let unicode = font
            .get(b"ToUnicode")
            .ok()
            .and_then(|object| resolve_object(document, object).ok())
            .and_then(|object| object.as_stream().ok())
            .and_then(|stream| stream.get_plain_content_with_limit(4 << 20).ok())
            .map(|cmap| parse_to_unicode(&cmap));
        let encoding = if unicode.is_none() && !composite {
            font.get_font_encoding(document).ok()
        } else {
            None
        };
        let number = |dictionary: &Dictionary, key: &[u8]| {
            dictionary
                .get(key)
                .ok()
                .and_then(|object| resolve_object(document, object).ok())
                .and_then(pdf_number)
        };
        let array = |dictionary: &'a Dictionary, key: &[u8]| {
            dictionary
                .get(key)
                .ok()
                .and_then(|object| resolve_object(document, object).ok())
                .and_then(|object| object.as_array().ok())
        };
        let widths = if composite {
            let descendant = array(font, b"DescendantFonts")
                .and_then(|fonts| fonts.first())
                .and_then(|descendant| dictionary(document, descendant));
            Widths::Composite {
                ranges: descendant
                    .and_then(|descendant| array(descendant, b"W"))
                    .map(|widths| composite_widths(document, widths))
                    .unwrap_or_default(),
                default: descendant
                    .and_then(|descendant| number(descendant, b"DW"))
                    .unwrap_or(1000.0),
            }
        } else {
            let widths: Vec<f64> = array(font, b"Widths")
                .map(|widths| {
                    widths
                        .iter()
                        .map(|width| {
                            resolve_object(document, width)
                                .ok()
                                .and_then(pdf_number)
                                .unwrap_or(0.0)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let missing = font
                .get(b"FontDescriptor")
                .ok()
                .and_then(|descriptor| dictionary(document, descriptor))
                .and_then(|descriptor| number(descriptor, b"MissingWidth"))
                // Standard fonts omit widths; an average advance keeps words apart.
                .unwrap_or(if widths.is_empty() { 500.0 } else { 0.0 });
            Widths::Simple {
                first: number(font, b"FirstChar").unwrap_or(0.0).max(0.0) as u32,
                widths,
                missing,
            }
        };
        let scale = if subtype == b"Type3" {
            array(font, b"FontMatrix")
                .and_then(|matrix| matrix.first())
                .and_then(pdf_number)
                .unwrap_or(0.001)
        } else {
            0.001
        };
        Self {
            code_bytes: if composite { 2 } else { 1 },
            unicode,
            encoding,
            widths,
            scale,
        }
    }

    fn decode(&self, code: u32, bytes: &[u8]) -> String {
        if let Some(unicode) = &self.unicode {
            return unicode.get(&code).cloned().unwrap_or_default();
        }
        self.encoding
            .as_ref()
            .and_then(|encoding| encoding.bytes_to_string(bytes).ok())
            .unwrap_or_default()
    }
}

fn composite_widths(document: &Document, widths: &[Object]) -> Vec<(u32, u32, f64)> {
    let value = |object: &Object| resolve_object(document, object).ok().and_then(pdf_number);
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < widths.len() {
        let Some(start) = value(&widths[index]).map(|start| start.max(0.0) as u32) else {
            break;
        };
        match widths
            .get(index + 1)
            .map(|next| resolve_object(document, next))
        {
            Some(Ok(Object::Array(list))) => {
                for (offset, width) in list.iter().enumerate() {
                    let code = start.saturating_add(offset as u32);
                    ranges.push((code, code, value(width).unwrap_or(0.0)));
                }
                index += 2;
            }
            Some(Ok(_)) => {
                let (Some(end), Some(width)) = (
                    widths.get(index + 1).and_then(value),
                    widths.get(index + 2).and_then(value),
                ) else {
                    break;
                };
                ranges.push((start, end.max(0.0) as u32, width));
                index += 3;
            }
            _ => break,
        }
    }
    ranges.sort_by_key(|(start, _, _)| *start);
    ranges
}

/// Code-to-Unicode entries from a ToUnicode CMap's `bfchar` and `bfrange` sections.
fn parse_to_unicode(cmap: &[u8]) -> HashMap<u32, String> {
    let tokens = tokens(cmap);
    let mut map = HashMap::new();
    let code = |bytes: &[u8]| {
        (bytes.len() <= 4).then(|| {
            bytes
                .iter()
                .fold(0u32, |code, byte| code << 8 | u32::from(*byte))
        })
    };
    let unicode = |bytes: &[u8]| {
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            Token::Word(word) if word == "beginbfchar" => {
                index += 1;
                while let (Some(Token::Hex(source)), Some(Token::Hex(target))) =
                    (tokens.get(index), tokens.get(index + 1))
                {
                    if let Some(source) = code(source) {
                        map.insert(source, unicode(target));
                    }
                    index += 2;
                }
            }
            Token::Word(word) if word == "beginbfrange" => {
                index += 1;
                while let (Some(Token::Hex(low)), Some(Token::Hex(high))) =
                    (tokens.get(index), tokens.get(index + 1))
                {
                    let (Some(low), Some(high)) = (code(low), code(high)) else {
                        break;
                    };
                    // Cap absurd ranges from damaged CMaps.
                    let high = high.min(low.saturating_add(0xffff));
                    match tokens.get(index + 2) {
                        Some(Token::Hex(target)) if !target.is_empty() => {
                            let mut target = target.clone();
                            for source in low..=high {
                                map.insert(source, unicode(&target));
                                let last = target.len() - 1;
                                target[last] = target[last].wrapping_add(1);
                            }
                            index += 3;
                        }
                        Some(Token::Open) => {
                            let mut offset = index + 3;
                            let mut source = low;
                            while let Some(Token::Hex(target)) = tokens.get(offset) {
                                if source <= high {
                                    map.insert(source, unicode(target));
                                }
                                source = source.saturating_add(1);
                                offset += 1;
                            }
                            index = offset + 1;
                        }
                        _ => break,
                    }
                }
            }
            _ => index += 1,
        }
    }
    map
}

#[derive(Debug, PartialEq)]
enum Token {
    Hex(Vec<u8>),
    Word(String),
    Open,
    Close,
}

fn tokens(data: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < data.len() {
        match data[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'%' => {
                while index < data.len() && data[index] != b'\n' && data[index] != b'\r' {
                    index += 1;
                }
            }
            b'<' if data.get(index + 1) == Some(&b'<') => index += 2,
            b'>' if data.get(index + 1) == Some(&b'>') => index += 2,
            b'<' => {
                let end = data[index..]
                    .iter()
                    .position(|byte| *byte == b'>')
                    .map_or(data.len(), |offset| index + offset);
                let digits: Vec<u8> = data[index + 1..end]
                    .iter()
                    .copied()
                    .filter(u8::is_ascii_hexdigit)
                    .collect();
                let bytes = digits
                    .chunks(2)
                    .map(|pair| {
                        let text = [pair[0], *pair.get(1).unwrap_or(&b'0')];
                        u8::from_str_radix(std::str::from_utf8(&text).unwrap_or("0"), 16)
                            .unwrap_or(0)
                    })
                    .collect();
                tokens.push(Token::Hex(bytes));
                index = end + 1;
            }
            b'[' => {
                tokens.push(Token::Open);
                index += 1;
            }
            b']' => {
                tokens.push(Token::Close);
                index += 1;
            }
            b'(' => {
                // Literal strings (such as CMap names) are skipped whole.
                let mut depth = 0usize;
                while index < data.len() {
                    match data[index] {
                        b'\\' => index += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                index += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    index += 1;
                }
            }
            _ => {
                let start = index;
                while index < data.len()
                    && !data[index].is_ascii_whitespace()
                    && !b"<>[]()%/".contains(&data[index])
                {
                    index += 1;
                }
                if index == start {
                    index += 1;
                } else {
                    tokens.push(Token::Word(
                        String::from_utf8_lossy(&data[start..index]).into_owned(),
                    ));
                }
            }
        }
    }
    tokens
}

#[derive(Clone)]
struct State<'a> {
    ctm: Matrix,
    char_spacing: f64,
    word_spacing: f64,
    horizontal_scale: f64,
    leading: f64,
    font: Option<Rc<Font<'a>>>,
    size: f64,
    rise: f64,
}

struct Walker<'a> {
    document: &'a Document,
    fonts: HashMap<ObjectId, Rc<Font<'a>>>,
    glyphs: Vec<Glyph>,
}

impl<'a> Walker<'a> {
    fn run(
        &mut self,
        content: &[u8],
        resources: Option<&'a Dictionary>,
        ctm: Matrix,
        depth: usize,
    ) {
        let mut state = State {
            ctm,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            font: None,
            size: 0.0,
            rise: 0.0,
        };
        let mut saved = Vec::new();
        let mut text_matrix = Matrix::IDENTITY;
        let mut line_matrix = Matrix::IDENTITY;
        for_each_operation(content, |operator, operands| {
            let number =
                |index: usize| operands.get(index).and_then(Operand::number).unwrap_or(0.0);
            match operator {
                b"q" => saved.push(state.clone()),
                b"Q" => {
                    if let Some(previous) = saved.pop() {
                        state = previous;
                    }
                }
                b"cm" => {
                    if let Some(matrix) = Matrix::from_numbers(operands) {
                        state.ctm = matrix.then(state.ctm);
                    }
                }
                b"BT" => {
                    text_matrix = Matrix::IDENTITY;
                    line_matrix = Matrix::IDENTITY;
                }
                b"Tf" => {
                    state.font = match operands.first() {
                        Some(Operand::Name(name)) => self.font(resources, name),
                        _ => None,
                    };
                    state.size = number(1);
                }
                b"Tc" => state.char_spacing = number(0),
                b"Tw" => state.word_spacing = number(0),
                b"Tz" => state.horizontal_scale = number(0) / 100.0,
                b"TL" => state.leading = number(0),
                b"Ts" => state.rise = number(0),
                b"Td" | b"TD" => {
                    if operator == b"TD" {
                        state.leading = -number(1);
                    }
                    line_matrix = Matrix::translate(number(0), number(1)).then(line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tm" => {
                    if let Some(matrix) = Matrix::from_numbers(operands) {
                        line_matrix = matrix;
                        text_matrix = matrix;
                    }
                }
                b"T*" => {
                    line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tj" | b"'" | b"\"" => {
                    if operator != b"Tj" {
                        if operator == b"\"" {
                            state.word_spacing = number(0);
                            state.char_spacing = number(1);
                        }
                        line_matrix = Matrix::translate(0.0, -state.leading).then(line_matrix);
                        text_matrix = line_matrix;
                    }
                    if let Some(Operand::Text(bytes)) = operands.last() {
                        self.show(bytes, &state, &mut text_matrix);
                    }
                }
                b"TJ" => {
                    if let Some(Operand::Array(items)) = operands.first() {
                        for item in items {
                            match item {
                                Operand::Text(bytes) => self.show(bytes, &state, &mut text_matrix),
                                Operand::Number(_) => {
                                    let adjustment = item.number().unwrap_or(0.0);
                                    let advance =
                                        -adjustment / 1000.0 * state.size * state.horizontal_scale;
                                    text_matrix = Matrix::translate(advance, 0.0).then(text_matrix);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                b"Do" if depth < MAX_FORM_DEPTH => {
                    if let Some(Operand::Name(name)) = operands.first() {
                        self.form(resources, name, state.ctm, depth);
                    }
                }
                _ => {}
            }
            self.glyphs.len() < MAX_GLYPHS
        });
    }

    fn font(&mut self, resources: Option<&'a Dictionary>, name: &[u8]) -> Option<Rc<Font<'a>>> {
        let document = self.document;
        let entry = resources?
            .get(b"Font")
            .ok()
            .and_then(|fonts| dictionary(document, fonts))?
            .get(name)
            .ok()?;
        if let Ok(id) = entry.as_reference() {
            if let Some(font) = self.fonts.get(&id) {
                return Some(font.clone());
            }
            let font = Rc::new(Font::new(document, document.get_dictionary(id).ok()?));
            self.fonts.insert(id, font.clone());
            return Some(font);
        }
        Some(Rc::new(Font::new(document, entry.as_dict().ok()?)))
    }

    fn form(&mut self, resources: Option<&'a Dictionary>, name: &[u8], ctm: Matrix, depth: usize) {
        let document = self.document;
        let Some(Ok(Object::Stream(form))) = resources
            .and_then(|resources| resources.get(b"XObject").ok())
            .and_then(|objects| dictionary(document, objects))
            .and_then(|objects| objects.get(name).ok())
            .map(|form| resolve_object(document, form))
        else {
            return;
        };
        if form.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") {
            return;
        }
        let Ok(content) = form.decompressed_content_with_limit(MAX_CONTENT_BYTES) else {
            return;
        };
        let matrix = form
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|matrix| matrix.as_array().ok())
            .and_then(|matrix| Matrix::from_operands(matrix))
            .unwrap_or(Matrix::IDENTITY);
        let form_resources = form
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|resources| dictionary(document, resources))
            .or(resources);
        self.run(&content, form_resources, matrix.then(ctm), depth + 1);
    }

    fn show(&mut self, bytes: &[u8], state: &State<'a>, text_matrix: &mut Matrix) {
        let Some(font) = state.font.clone() else {
            return;
        };
        for code_bytes in bytes.chunks(font.code_bytes) {
            let code = code_bytes
                .iter()
                .fold(0u32, |code, byte| code << 8 | u32::from(*byte));
            let width = font.widths.get(code) * font.scale;
            let render = Matrix([
                state.size * state.horizontal_scale,
                0.0,
                0.0,
                state.size,
                0.0,
                state.rise,
            ])
            .then(*text_matrix)
            .then(state.ctm);
            let [_, _, c, d, _, _] = render.0;
            self.glyphs.push(Glyph {
                text: font.decode(code, code_bytes),
                origin: render.apply(0.0, 0.0),
                end: render.apply(width, 0.0),
                center: render.apply(width / 2.0, 0.3),
                size: c.hypot(d),
            });
            let word = if code_bytes == b" " {
                state.word_spacing
            } else {
                0.0
            };
            let advance = (width * state.size + state.char_spacing + word) * state.horizontal_scale;
            *text_matrix = Matrix::translate(advance, 0.0).then(*text_matrix);
        }
    }
}

/// A page's own or inherited resources, borrowed from the document.
fn page_resources(document: &Document, page_id: ObjectId) -> Option<&Dictionary> {
    let mut current = document.get_dictionary(page_id).ok()?;
    for _ in 0..64 {
        if let Ok(resources) = current.get(b"Resources") {
            return dictionary(document, resources);
        }
        current = document
            .get_dictionary(current.get(b"Parent").ok()?.as_reference().ok()?)
            .ok()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unicode_maps_characters_ranges_and_arrays() {
        let cmap = b"1 begincodespacerange <00> <FF> endcodespacerange
            2 beginbfchar <01> <0041> <02> <00660069> endbfchar
            2 beginbfrange <10> <12> <0061> <20> <21> [<0058> <0059>] endbfrange";
        let map = parse_to_unicode(cmap);
        assert_eq!(map[&1], "A");
        assert_eq!(map[&2], "fi");
        assert_eq!((map[&0x10].as_str(), map[&0x12].as_str()), ("a", "c"));
        assert_eq!((map[&0x20].as_str(), map[&0x21].as_str()), ("X", "Y"));
    }

    fn describe(operand: &Operand) -> String {
        match operand {
            Operand::Number(_) => operand.number().unwrap().to_string(),
            Operand::Name(name) => format!("/{}", String::from_utf8_lossy(name)),
            Operand::Text(text) => format!("({})", String::from_utf8_lossy(text)),
            Operand::Array(items) => {
                let items: Vec<String> = items.iter().map(describe).collect();
                format!("[{}]", items.join(" "))
            }
            Operand::Other => "?".into(),
        }
    }

    #[test]
    fn lexer_reads_text_operators_and_skips_everything_else() {
        let content = b"q 1 0 0 1 5 6 cm % note (not a string\n\
            /Span <</MCID 0 /Alt (x (y) z) /K [1 2]>> BDC BT /F1 12 Tf\n\
            [(A\\(b\\)) -250 <4142 4>] TJ (x\\101\\\ny) Tj ET\n\
            BI /W 2 /H 1 /CS /G ID \x00EI\x01 EI 0.5 -.5 re Q";
        let mut operations = Vec::new();
        for_each_operation(content, |operator, operands| {
            let mut operation = String::from_utf8_lossy(operator).into_owned();
            for operand in operands {
                operation.push(' ');
                operation.push_str(&describe(operand));
            }
            operations.push(operation);
            true
        });
        assert_eq!(
            operations,
            [
                "q",
                "cm 1 0 0 1 5 6",
                "BDC /Span ?",
                "BT",
                "Tf /F1 12",
                "TJ [(A(b)) -250 (AB@)]",
                "Tj (xAy)",
                "ET",
                "re 0.5 -0.5",
                "Q",
            ]
        );
    }

    #[test]
    fn matrices_compose_in_pdf_order() {
        let scaled = Matrix([2.0, 0.0, 0.0, 2.0, 0.0, 0.0]).then(Matrix::translate(10.0, 5.0));
        assert_eq!(scaled.apply(1.0, 1.0), (12.0, 7.0));
    }

    #[test]
    fn glyphs_join_into_words_and_lines() {
        let glyph = |text: &str, x: f64, y: f64| Glyph {
            text: text.into(),
            origin: (x, y),
            end: (x + 5.0, y),
            center: (x + 2.5, y + 3.0),
            size: 10.0,
        };
        let glyphs = [
            glyph("L", 0.0, 100.0),
            glyph("a", 5.0, 100.0),
            glyph("t", 10.0, 100.0),
            glyph("P", 20.0, 100.0),
            glyph("x", 0.0, 80.0),
        ];
        assert_eq!(lines(&glyphs), ["Lat P", "x"]);
        assert_eq!(text_inside(&glyphs, [0.0, 95.0, 13.0, 110.0]), "Lat");
    }
}
