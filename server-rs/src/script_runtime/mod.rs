//! Bounded, expression-oriented scripts for designer script blocks.

use image::{Rgba, RgbaImage};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

pub const MAX_AST_NODES: usize = 400;
pub const MAX_OPERATIONS: usize = 3000;
pub const MAX_LOOP_ITEMS: usize = 100;
pub const MAX_TEXT_LENGTH: usize = 2000;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    #[error("{0}")]
    Error(String),
}

impl From<&str> for ScriptError {
    fn from(s: &str) -> Self {
        ScriptError::Error(s.to_string())
    }
}

impl From<String> for ScriptError {
    fn from(s: String) -> Self {
        ScriptError::Error(s)
    }
}

#[derive(Debug, Clone)]
pub struct ScriptOutput {
    pub image: Option<RgbaImage>,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScriptExample {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub code: &'static str,
}

pub const SCRIPT_EXAMPLES: &[ScriptExample] = &[
    ScriptExample {
        id: "sensor-text",
        name: "HA sensor text",
        description: "Formats a Home Assistant sensor as text.",
        code: "value = ha(\"sensor.office_temperature\")\ntext(f\"Office {value}°C\")",
    },
    ScriptExample {
        id: "progress-bar",
        name: "Sensor progress bar",
        description: "Draws a value from 0–100 as a labeled bar.",
        code: "value = number(ha(\"sensor.battery_level\"), 72)\nclear(\"#18202b\")\nrect(4, 4, width - 8, height - 8, \"#273447\", 6)\nrect(8, 8, (width - 16) * clamp(value, 0, 100) / 100, height - 16, \"#f2c94c\", 4)\nlabel(width / 2, height / 2, f\"{value:.0f}%\", \"#ffffff\", 14, \"mm\")",
    },
    ScriptExample {
        id: "status-badge",
        name: "Status badge",
        description: "Changes color from an HA entity state.",
        code: "state = ha(\"binary_sensor.front_door\")\nif state == \"on\":\n    clear(\"#9b2c3b\")\n    label(width / 2, height / 2, \"DOOR OPEN\", \"#ffffff\", 15, \"mm\")\nelse:\n    clear(\"#1f7a4d\")\n    label(width / 2, height / 2, \"Door closed\", \"#ffffff\", 15, \"mm\")",
    },
    ScriptExample {
        id: "web-json",
        name: "Web JSON value",
        description: "Pulls a field out of an http-text data source's JSON with jq().",
        code: "temp = jq(source(\"weather\"), \".current.temp_c\")\nif temp == None:\n    text(\"Weather unavailable\")\nelif temp < 10:\n    text(f\"Cold: {temp:.1f}°C\")\nelif temp < 25:\n    text(f\"Mild: {temp:.1f}°C\")\nelse:\n    text(f\"Hot: {temp:.1f}°C\")",
    },
    ScriptExample {
        id: "mini-chart",
        name: "Decorative mini chart",
        description: "Shows bounded loops and line drawing without external data.",
        code: "clear(\"#121826\")\nlast_x = 0\nlast_y = height / 2\nfor point in range(1, 13):\n    x = point * width / 12\n    y = height / 2 + sin(point * 0.9) * height * 0.3\n    line(last_x, last_y, x, y, \"#6b9cff\", 2)\n    last_x = x\n    last_y = y\nlabel(5, 5, now(\"%H:%M\"), \"#f2c94c\", 11, \"la\")",
    },
];

// Tokenizer & AST for bounded python subset
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Number(f64),
    String(String),
    FString(String),
    Newline,
    Indent,
    Dedent,
    LParen,
    RParen,
    Comma,
    Colon,
    Dot,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    KwIf,
    KwElif,
    KwElse,
    KwFor,
    KwWhile,
    KwBreak,
    KwContinue,
    KwIn,
    KwAnd,
    KwOr,
    KwNot,
    KwTrue,
    KwFalse,
    KwNone,
    Eof,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, ScriptError> {
    let mut tokens = Vec::new();
    let mut indent_stack = vec![0];
    let lines = input.lines().collect::<Vec<_>>();

    for raw_line in lines {
        let trimmed = raw_line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let line_indent = raw_line.len() - trimmed.len();
        let current_indent = *indent_stack.last().unwrap();

        if line_indent > current_indent {
            indent_stack.push(line_indent);
            tokens.push(Token::Indent);
        } else {
            while line_indent < *indent_stack.last().unwrap() {
                indent_stack.pop();
                tokens.push(Token::Dedent);
            }
            if line_indent != *indent_stack.last().unwrap() {
                return Err(ScriptError::Error("inconsistent indentation".into()));
            }
        }

        let chars: Vec<char> = trimmed.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            if c == '#' {
                break;
            }

            if c == '(' { tokens.push(Token::LParen); i += 1; }
            else if c == ')' { tokens.push(Token::RParen); i += 1; }
            else if c == ',' { tokens.push(Token::Comma); i += 1; }
            else if c == ':' { tokens.push(Token::Colon); i += 1; }
            else if c == '.' { tokens.push(Token::Dot); i += 1; }
            else if c == '=' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::EqEq); i += 2;
                } else {
                    tokens.push(Token::Eq); i += 1;
                }
            }
            else if c == '!' && i + 1 < chars.len() && chars[i + 1] == '=' {
                tokens.push(Token::NotEq); i += 2;
            }
            else if c == '<' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::LtEq); i += 2;
                } else {
                    tokens.push(Token::Lt); i += 1;
                }
            }
            else if c == '>' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::GtEq); i += 2;
                } else {
                    tokens.push(Token::Gt); i += 1;
                }
            }
            else if c == '+' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::PlusEq); i += 2;
                } else {
                    tokens.push(Token::Plus); i += 1;
                }
            }
            else if c == '-' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::MinusEq); i += 2;
                } else {
                    tokens.push(Token::Minus); i += 1;
                }
            }
            else if c == '*' {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    tokens.push(Token::Power); i += 2;
                } else if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::StarEq); i += 2;
                } else {
                    tokens.push(Token::Star); i += 1;
                }
            }
            else if c == '/' {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::SlashEq); i += 2;
                } else {
                    tokens.push(Token::Slash); i += 1;
                }
            }
            else if c == '%' { tokens.push(Token::Percent); i += 1; }
            else if c == '"' || c == '\'' {
                let quote = c;
                i += 1;
                let mut s = String::new();
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            '\\' => s.push('\\'),
                            q if q == quote => s.push(quote),
                            other => { s.push('\\'); s.push(other); }
                        }
                    } else {
                        s.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(ScriptError::Error("unterminated string".into()));
                }
                i += 1;
                tokens.push(Token::String(s));
            }
            else if (c == 'f' || c == 'F') && i + 1 < chars.len() && (chars[i + 1] == '"' || chars[i + 1] == '\'') {
                let quote = chars[i + 1];
                i += 2;
                let mut s = String::new();
                while i < chars.len() && chars[i] != quote {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        match chars[i] {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            q if q == quote => s.push(quote),
                            other => { s.push('\\'); s.push(other); }
                        }
                    } else {
                        s.push(chars[i]);
                    }
                    i += 1;
                }
                if i >= chars.len() {
                    return Err(ScriptError::Error("unterminated f-string".into()));
                }
                i += 1;
                tokens.push(Token::FString(s));
            }
            else if c.is_ascii_digit() {
                let start = i;
                let mut has_dot = false;
                while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == '.' && !has_dot && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())) {
                    if chars[i] == '.' {
                        has_dot = true;
                    }
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                let num = num_str.parse::<f64>().map_err(|e| ScriptError::Error(e.to_string()))?;
                tokens.push(Token::Number(num));
            }
            else if c.is_alphabetic() || c == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                match ident.as_str() {
                    "if" => tokens.push(Token::KwIf),
                    "elif" => tokens.push(Token::KwElif),
                    "else" => tokens.push(Token::KwElse),
                    "for" => tokens.push(Token::KwFor),
                    "while" => tokens.push(Token::KwWhile),
                    "break" => tokens.push(Token::KwBreak),
                    "continue" => tokens.push(Token::KwContinue),
                    "in" => tokens.push(Token::KwIn),
                    "and" => tokens.push(Token::KwAnd),
                    "or" => tokens.push(Token::KwOr),
                    "not" => tokens.push(Token::KwNot),
                    "True" => tokens.push(Token::KwTrue),
                    "False" => tokens.push(Token::KwFalse),
                    "None" => tokens.push(Token::KwNone),
                    "import" => return Err(ScriptError::Error("import is forbidden".into())),
                    _ => tokens.push(Token::Ident(ident)),
                }
            } else {
                return Err(ScriptError::Error(format!("unexpected character: {c}")));
            }
        }
        tokens.push(Token::Newline);
    }

    while indent_stack.len() > 1 {
        indent_stack.pop();
        tokens.push(Token::Dedent);
    }
    tokens.push(Token::Eof);
    Ok(tokens)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Val {
    pub fn is_truthy(&self) -> bool {
        match self {
            Val::Nil => false,
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0,
            Val::Str(s) => !s.is_empty(),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Val::Nil => "None".to_string(),
            Val::Bool(b) => if *b { "True".to_string() } else { "False".to_string() },
            Val::Num(n) => {
                if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Val::Str(s) => s.clone(),
        }
    }

    pub fn to_f64(&self) -> Result<f64, ScriptError> {
        match self {
            Val::Num(n) => Ok(*n),
            Val::Str(s) => s.trim().parse::<f64>().map_err(|_| ScriptError::Error("not a number".into())),
            Val::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Val::Nil => Ok(0.0),
        }
    }
}

pub struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub fn new(tokens: &'a [Token]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let t = self.tokens.get(self.pos).unwrap_or(&Token::Eof);
        self.pos += 1;
        t
    }

    fn check(&self, token: &Token) -> bool {
        self.peek() == token
    }

    fn match_token(&mut self, token: &Token) -> bool {
        if self.check(token) {
            self.advance();
            true
        } else {
            false
        }
    }
}

/// Map a JSON value to a script value: scalars convert directly,
/// objects/arrays become their compact JSON text (feed back into jq()).
fn json_to_val(v: &Value) -> Val {
    match v {
        Value::Null => Val::Nil,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => Val::Str(s.clone()),
        _ => Val::Str(v.to_string()),
    }
}

pub struct Runtime<'a> {
    pub width: u32,
    pub operations: usize,
    pub height: u32,
    pub context: &'a Value,
    pub vars: HashMap<String, Val>,
    pub image: RgbaImage,
    pub text_output: Option<String>,
    pub drew: bool,
}

impl<'a> Runtime<'a> {
    pub fn new(width: u32, height: u32, context: &'a Value) -> Self {
        let mut vars = HashMap::new();
        vars.insert("width".to_string(), Val::Num(width as f64));
        vars.insert("height".to_string(), Val::Num(height as f64));
        vars.insert("pi".to_string(), Val::Num(std::f64::consts::PI));

        Self {
            width,
            height,
            context,
            vars,
            image: RgbaImage::new(width, height),
            text_output: None,
            drew: false,
            operations: 0,
        }
    }

    pub fn tick(&mut self, n: usize) -> Result<(), ScriptError> {
        self.operations += n;
        if self.operations > MAX_OPERATIONS {
            return Err(ScriptError::Error("operation limit exceeded".into()));
        }
        Ok(())
    }

    pub fn parse_color(&self, val: &Val) -> Result<Rgba<u8>, ScriptError> {
        let s = val.to_string();
        let s = s.trim_start_matches('#');
        if s.len() == 6 {
            let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ScriptError::Error("invalid color".into()))?;
            let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ScriptError::Error("invalid color".into()))?;
            let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ScriptError::Error("invalid color".into()))?;
            Ok(Rgba([r, g, b, 255]))
        } else {
            Err(ScriptError::Error("colors must use #rrggbb".into()))
        }
    }

    pub fn call_func(&mut self, name: &str, args: Vec<Val>) -> Result<Val, ScriptError> {
        self.tick(1)?;
        match name {
            "ha" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(ScriptError::Error("ha() expects an entity ID and optional attribute".into()));
                }
                let entity_id = args[0].to_string();
                let entity = self.context.get("homeAssistant").and_then(|h| h.get(&entity_id));
                if let Some(ent) = entity {
                    if ent.get("error").and_then(|v| v.as_bool()) == Some(true) {
                        return Ok(Val::Str("unavailable".into()));
                    }
                    if args.len() == 2 {
                        let attr_name = args[1].to_string();
                        if let Some(attr_val) = ent.get("attributes").and_then(|a| a.get(&attr_name)) {
                            return Ok(match attr_val {
                                Value::String(s) => Val::Str(s.clone()),
                                Value::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
                                Value::Bool(b) => Val::Bool(*b),
                                _ => Val::Str(attr_val.to_string()),
                            });
                        }
                        return Ok(Val::Str("unknown".into()));
                    }
                    if let Some(state) = ent.get("state") {
                        return Ok(match state {
                            Value::String(s) => Val::Str(s.clone()),
                            Value::Number(n) => Val::Num(n.as_f64().unwrap_or(0.0)),
                            Value::Bool(b) => Val::Bool(*b),
                            _ => Val::Str(state.to_string()),
                        });
                    }
                    Ok(Val::Str("unknown".into()))
                } else {
                    Ok(Val::Str("unavailable".into()))
                }
            }
            "now" => {
                let fmt = if !args.is_empty() { args[0].to_string() } else { "%H:%M".to_string() };
                let now = chrono::Utc::now().with_timezone(&crate::renderer::studio_timezone());
                Ok(Val::Str(now.format(&fmt).to_string()))
            }
            "source" => {
                // The current text of an http-text data source, by id —
                // the same values external-text elements display. Missing
                // or not-yet-fetched sources read as "".
                if args.len() != 1 {
                    return Err(ScriptError::Error("source() expects a data source id".into()));
                }
                let id = args[0].to_string();
                let text = self
                    .context
                    .get("external")
                    .and_then(|e| e.get(&id))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(Val::Str(text))
            }
            "jq" => {
                // jq(text, program): parse text as JSON and run a jq
                // program over it (the same jaq engine as the jq_design
                // tool). Unparsable JSON reads as None so scripts can
                // guard on it; a bad program is a hard error. One scalar
                // output comes back as that value; objects/arrays and
                // multi-output programs come back as JSON text.
                if args.len() != 2 {
                    return Err(ScriptError::Error("jq() expects JSON text and a jq program".into()));
                }
                let text = args[0].to_string();
                let program = args[1].to_string();
                let parsed: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => return Ok(Val::Nil),
                };
                match crate::server::jq::run_jq(&program, &parsed) {
                    Ok(outputs) => Ok(match outputs.len() {
                        0 => Val::Nil,
                        1 => json_to_val(&outputs[0]),
                        _ => Val::Str(Value::Array(outputs).to_string()),
                    }),
                    Err(e) => Err(ScriptError::Error(e)),
                }
            }
            "number" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(ScriptError::Error("number() expects a value and optional default".into()));
                }
                let default = if args.len() == 2 { args[1].to_f64().unwrap_or(0.0) } else { 0.0 };
                Ok(Val::Num(args[0].to_f64().unwrap_or(default)))
            }
            "clamp" => {
                if args.len() != 3 {
                    return Err(ScriptError::Error("clamp() expects value, min, max".into()));
                }
                let v = args[0].to_f64()?;
                let min = args[1].to_f64()?;
                let max = args[2].to_f64()?;
                Ok(Val::Num(v.clamp(min, max)))
            }
            "sin" => {
                if args.len() != 1 { return Err(ScriptError::Error("sin() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.sin()))
            }
            "cos" => {
                if args.len() != 1 { return Err(ScriptError::Error("cos() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.cos()))
            }
            "tan" => {
                if args.len() != 1 { return Err(ScriptError::Error("tan() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.tan()))
            }
            "sqrt" => {
                if args.len() != 1 { return Err(ScriptError::Error("sqrt() expects 1 arg".into())); }
                let n = args[0].to_f64()?;
                if n < 0.0 { return Err(ScriptError::Error("sqrt of negative".into())); }
                Ok(Val::Num(n.sqrt()))
            }
            "abs" => {
                if args.len() != 1 { return Err(ScriptError::Error("abs() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.abs()))
            }
            "min" => {
                if args.is_empty() { return Err(ScriptError::Error("min() expects at least 1 arg".into())); }
                let mut m = args[0].to_f64()?;
                for a in &args[1..] {
                    m = m.min(a.to_f64()?);
                }
                Ok(Val::Num(m))
            }
            "max" => {
                if args.is_empty() { return Err(ScriptError::Error("max() expects at least 1 arg".into())); }
                let mut m = args[0].to_f64()?;
                for a in &args[1..] {
                    m = m.max(a.to_f64()?);
                }
                Ok(Val::Num(m))
            }
            "round" => {
                if args.len() == 1 {
                    Ok(Val::Num(args[0].to_f64()?.round()))
                } else if args.len() == 2 {
                    let v = args[0].to_f64()?;
                    let d = args[1].to_f64()? as i32;
                    let factor = 10f64.powi(d);
                    Ok(Val::Num((v * factor).round() / factor))
                } else {
                    Err(ScriptError::Error("round() expects 1 or 2 args".into()))
                }
            }
            "len" => {
                if args.len() != 1 { return Err(ScriptError::Error("len() expects 1 arg".into())); }
                match &args[0] {
                    Val::Str(s) => Ok(Val::Num(s.chars().count() as f64)),
                    _ => Err(ScriptError::Error("len() expects a string".into())),
                }
            }
            "str" => {
                if args.len() != 1 { return Err(ScriptError::Error("str() expects 1 arg".into())); }
                Ok(Val::Str(args[0].to_string()))
            }
            "int" => {
                if args.len() != 1 { return Err(ScriptError::Error("int() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.trunc()))
            }
            "float" => {
                if args.len() != 1 { return Err(ScriptError::Error("float() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?))
            }
            "bool" => {
                if args.len() != 1 { return Err(ScriptError::Error("bool() expects 1 arg".into())); }
                Ok(Val::Bool(args[0].is_truthy()))
            }
            "floor" => {
                if args.len() != 1 { return Err(ScriptError::Error("floor() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.floor()))
            }
            "ceil" => {
                if args.len() != 1 { return Err(ScriptError::Error("ceil() expects 1 arg".into())); }
                Ok(Val::Num(args[0].to_f64()?.ceil()))
            }
            "pow" => {
                if args.len() != 2 { return Err(ScriptError::Error("pow() expects 2 args".into())); }
                Ok(Val::Num(args[0].to_f64()?.powf(args[1].to_f64()?)))
            }
            "lerp" => {
                if args.len() != 3 { return Err(ScriptError::Error("lerp() expects a, b, t".into())); }
                let a = args[0].to_f64()?;
                let b = args[1].to_f64()?;
                let t = args[2].to_f64()?;
                Ok(Val::Num(a + (b - a) * t))
            }
            "map_range" => {
                if args.len() != 5 { return Err(ScriptError::Error("map_range() expects value, in_min, in_max, out_min, out_max".into())); }
                let x = args[0].to_f64()?;
                let in_min = args[1].to_f64()?;
                let in_max = args[2].to_f64()?;
                let out_min = args[3].to_f64()?;
                let out_max = args[4].to_f64()?;
                if in_max == in_min { return Err(ScriptError::Error("map_range() input range is empty".into())); }
                Ok(Val::Num(out_min + (x - in_min) * (out_max - out_min) / (in_max - in_min)))
            }
            "upper" => {
                if args.len() != 1 { return Err(ScriptError::Error("upper() expects 1 arg".into())); }
                Ok(Val::Str(args[0].to_string().to_uppercase()))
            }
            "lower" => {
                if args.len() != 1 { return Err(ScriptError::Error("lower() expects 1 arg".into())); }
                Ok(Val::Str(args[0].to_string().to_lowercase()))
            }
            "rgb" => {
                if args.len() != 3 { return Err(ScriptError::Error("rgb() expects r, g, b".into())); }
                let ch = |v: &Val| -> Result<u8, ScriptError> {
                    Ok(v.to_f64()?.round().clamp(0.0, 255.0) as u8)
                };
                Ok(Val::Str(format!("#{:02x}{:02x}{:02x}", ch(&args[0])?, ch(&args[1])?, ch(&args[2])?)))
            }
            "text" => {
                if args.len() != 1 { return Err(ScriptError::Error("text() expects 1 arg".into())); }
                self.text_output = Some(args[0].to_string());
                Ok(Val::Nil)
            }
            "clear" => {
                if args.len() != 1 { return Err(ScriptError::Error("clear() expects 1 color arg".into())); }
                let col = self.parse_color(&args[0])?;
                for p in self.image.pixels_mut() {
                    *p = col;
                }
                self.drew = true;
                Ok(Val::Nil)
            }
            "rect" => {
                if args.len() < 5 || args.len() > 6 {
                    return Err(ScriptError::Error("rect() expects x, y, w, h, color, [radius]".into()));
                }
                let x = args[0].to_f64()?.round() as i32;
                let y = args[1].to_f64()?.round() as i32;
                let w = args[2].to_f64()?.round() as i32;
                let h = args[3].to_f64()?.round() as i32;
                let col = self.parse_color(&args[4])?;
                let radius = if args.len() == 6 { args[5].to_f64()?.round() as i32 } else { 0 };

                self.draw_rect(x, y, w, h, col, radius);
                self.drew = true;
                Ok(Val::Nil)
            }
            "line" => {
                if args.len() < 5 || args.len() > 6 {
                    return Err(ScriptError::Error("line() expects x1, y1, x2, y2, color, [width]".into()));
                }
                let x1 = args[0].to_f64()?.round() as i32;
                let y1 = args[1].to_f64()?.round() as i32;
                let x2 = args[2].to_f64()?.round() as i32;
                let y2 = args[3].to_f64()?.round() as i32;
                let col = self.parse_color(&args[4])?;
                let width = if args.len() == 6 { args[5].to_f64()?.round() as i32 } else { 1 };

                self.draw_line(x1, y1, x2, y2, col, width);
                self.drew = true;
                Ok(Val::Nil)
            }
            "circle" => {
                if args.len() != 4 {
                    return Err(ScriptError::Error("circle() expects x, y, radius, color".into()));
                }
                let cx = args[0].to_f64()?.round() as i32;
                let cy = args[1].to_f64()?.round() as i32;
                let r = args[2].to_f64()?.round() as i32;
                let col = self.parse_color(&args[3])?;

                self.draw_circle(cx, cy, r, col);
                self.drew = true;
                Ok(Val::Nil)
            }
            "label" => {
                if args.len() < 3 {
                    return Err(ScriptError::Error("label() expects at least x, y, text".into()));
                }
                let x = args[0].to_f64()?.round() as i32;
                let y = args[1].to_f64()?.round() as i32;
                let text = args[2].to_string();
                let col = if args.len() >= 4 { self.parse_color(&args[3])? } else { Rgba([255, 255, 255, 255]) };
                let font_size = if args.len() >= 5 { args[4].to_f64()?.round() as u32 } else { 16 };
                let align = if args.len() >= 6 { args[5].to_string() } else { "la".to_string() };

                self.draw_label(x, y, &text, col, font_size, &align);
                self.drew = true;
                Ok(Val::Nil)
            }
            _ => Err(ScriptError::Error(format!("unknown function: {name}"))),
        }
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, col: Rgba<u8>, radius: i32) {
        if w <= 0 || h <= 0 { return; }
        let r = radius.max(0).min(w / 2).min(h / 2);
        for dy in 0..h {
            let py = y + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            for dx in 0..w {
                let px = x + dx;
                if px < 0 || px >= self.width as i32 { continue; }

                if r > 0 {
                    let in_corner_tl = dx < r && dy < r && (dx - r).pow(2) + (dy - r).pow(2) > r.pow(2);
                    let in_corner_tr = dx >= w - r && dy < r && (dx - (w - r - 1)).pow(2) + (dy - r).pow(2) > r.pow(2);
                    let in_corner_bl = dx < r && dy >= h - r && (dx - r).pow(2) + (dy - (h - r - 1)).pow(2) > r.pow(2);
                    let in_corner_br = dx >= w - r && dy >= h - r && (dx - (w - r - 1)).pow(2) + (dy - (h - r - 1)).pow(2) > r.pow(2);
                    if in_corner_tl || in_corner_tr || in_corner_bl || in_corner_br {
                        continue;
                    }
                }
                self.image.put_pixel(px as u32, py as u32, col);
            }
        }
    }

    fn draw_circle(&mut self, cx: i32, cy: i32, r: i32, col: Rgba<u8>) {
        if r <= 0 { return; }
        for dy in -r..=r {
            let py = cy + dy;
            if py < 0 || py >= self.height as i32 { continue; }
            for dx in -r..=r {
                let px = cx + dx;
                if px < 0 || px >= self.width as i32 { continue; }
                if dx * dx + dy * dy <= r * r {
                    self.image.put_pixel(px as u32, py as u32, col);
                }
            }
        }
    }

    fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, col: Rgba<u8>, width: i32) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        let half_w = (width - 1).max(0) / 2;

        loop {
            for ox in -half_w..=half_w {
                for oy in -half_w..=half_w {
                    let px = x0 + ox;
                    let py = y0 + oy;
                    if px >= 0 && px < self.width as i32 && py >= 0 && py < self.height as i32 {
                        self.image.put_pixel(px as u32, py as u32, col);
                    }
                }
            }
            if x0 == x1 && y0 == y1 { break; }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn draw_label(&mut self, x: i32, y: i32, text: &str, col: Rgba<u8>, font_size: u32, align: &str) {
        let (scale, text_w) = label_metrics(text, font_size);
        let char_h = 8 * scale;

        let start_x = match align.chars().next().unwrap_or('l') {
            'c' | 'm' => x - (text_w as i32) / 2,
            'r' => x - text_w as i32,
            _ => x,
        };

        let start_y = match align.chars().nth(1).unwrap_or('a') {
            'm' => y - (char_h as i32) / 2,
            'b' => y - char_h as i32,
            _ => y,
        };

        crate::renderer::draw_bitmap_text(&mut self.image, start_x, start_y, text, col, scale);
    }
}

/// Bitmap-font metrics for label(): the glyph cell is 8x8 and
/// draw_bitmap_text advances 8 * scale px per character, so the measured
/// text width must use the same 8px advance. (This used to measure 6px per
/// char, which pushed centered and right-anchored labels 25% too far right
/// and cut their text off at the block edge.) Width counts characters, not
/// UTF-8 bytes.
pub fn label_metrics(text: &str, font_size: u32) -> (u32, u32) {
    let scale = (font_size as f32 / 8.0).max(1.0).round() as u32;
    let text_w = text.chars().count() as u32 * 8 * scale;
    (scale, text_w)
}

pub fn eval_fstring(raw: &str, runtime: &mut Runtime) -> Result<String, ScriptError> {
    let mut out = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            i += 1;
            let mut expr_str = String::new();
            while i < chars.len() && chars[i] != '}' {
                expr_str.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1; // skip '}'
            }
            let (expr_part, fmt_part) = if let Some((e, f)) = expr_str.split_once(':') {
                (e.trim(), Some(f.trim()))
            } else {
                (expr_str.trim(), None)
            };

            let val = eval_expr_str(expr_part, runtime)?;
            if let Some(fmt) = fmt_part {
                if fmt.ends_with('d') {
                    // Integer with optional width / zero padding: {n:02d}, {n:3d}, {n:d}
                    let spec = fmt.trim_end_matches('d');
                    let zero = spec.starts_with('0') && spec.len() > 1;
                    let width = spec.trim_start_matches('0').parse::<usize>().unwrap_or(0);
                    let n = val.to_f64()?.trunc() as i64;
                    if zero {
                        out.push_str(&format!("{:0width$}", n, width = width));
                    } else if width > 0 {
                        out.push_str(&format!("{:width$}", n, width = width));
                    } else {
                        out.push_str(&format!("{n}"));
                    }
                } else if fmt.ends_with('f') {
                    let num = val.to_f64()?;
                    let decimals = fmt.trim_end_matches('f').trim_start_matches('.').parse::<usize>().unwrap_or(1);
                    out.push_str(&format!("{:.prec$}", num, prec = decimals));
                } else if fmt.ends_with('%') {
                    let num = val.to_f64()?;
                    let decimals = fmt.trim_end_matches('%').trim_start_matches('.').parse::<usize>().unwrap_or(0);
                    out.push_str(&format!("{:.prec$}%", num, prec = decimals));
                } else {
                    out.push_str(&val.to_string());
                }
            } else {
                out.push_str(&val.to_string());
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Ok(out)
}

pub fn eval_expr_str(expr_str: &str, runtime: &mut Runtime) -> Result<Val, ScriptError> {
    let tokens = tokenize(expr_str)?;
    let mut parser = Parser::new(&tokens);
    parse_expr(&mut parser, runtime)
}

fn parse_expr(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let value = parse_or(p, rt)?;
    // Ternary: `value if cond else other`. The first operand is evaluated
    // before the condition is known (this interpreter evaluates as it
    // parses), so keep it side-effect free; the else branch stays lazy —
    // it is skipped token-wise when the condition holds.
    if p.match_token(&Token::KwIf) {
        let cond = parse_or(p, rt)?;
        if !p.match_token(&Token::KwElse) {
            return Err(ScriptError::Error("expected 'else' in conditional expression".into()));
        }
        if cond.is_truthy() {
            skip_expr(p)?;
            Ok(value)
        } else {
            parse_expr(p, rt)
        }
    } else {
        Ok(value)
    }
}

/// Advance past one expression's tokens without evaluating them (the
/// untaken ternary branch). Tracks parenthesis depth; stops at the token
/// that ends the expression in the enclosing context.
fn skip_expr(p: &mut Parser) -> Result<(), ScriptError> {
    let mut depth = 0i32;
    loop {
        match p.peek() {
            Token::Eof => break,
            Token::LParen => {
                depth += 1;
                p.advance();
            }
            Token::RParen => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                p.advance();
            }
            Token::Newline | Token::Comma | Token::Dedent | Token::Colon if depth == 0 => break,
            _ => {
                p.advance();
            }
        }
    }
    Ok(())
}

fn parse_or(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let mut left = parse_and(p, rt)?;
    while p.match_token(&Token::KwOr) {
        let right = parse_and(p, rt)?;
        left = Val::Bool(left.is_truthy() || right.is_truthy());
    }
    Ok(left)
}

fn parse_and(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let mut left = parse_not(p, rt)?;
    while p.match_token(&Token::KwAnd) {
        let right = parse_not(p, rt)?;
        left = Val::Bool(left.is_truthy() && right.is_truthy());
    }
    Ok(left)
}

fn parse_not(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    if p.match_token(&Token::KwNot) {
        let val = parse_not(p, rt)?;
        Ok(Val::Bool(!val.is_truthy()))
    } else {
        parse_comparison(p, rt)
    }
}

fn parse_comparison(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let left = parse_add_sub(p, rt)?;
    if p.check(&Token::KwIn) {
        p.advance();
        let right = parse_add_sub(p, rt)?;
        return match (&left, &right) {
            (Val::Str(needle), Val::Str(haystack)) => Ok(Val::Bool(haystack.contains(needle.as_str()))),
            _ => Err(ScriptError::Error("'in' expects two strings (substring test)".into())),
        };
    }
    let op = p.peek().clone();
    match op {
        Token::EqEq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => {
            p.advance();
            let right = parse_add_sub(p, rt)?;
            let cmp = match (&left, &right) {
                (Val::Num(a), Val::Num(b)) => a.partial_cmp(b),
                (Val::Str(a), Val::Str(b)) => a.partial_cmp(b),
                (Val::Bool(a), Val::Bool(b)) => a.partial_cmp(b),
                _ => {
                    let as_str = left.to_string();
                    let bs_str = right.to_string();
                    as_str.partial_cmp(&bs_str)
                }
            };
            let res = match op {
                Token::EqEq => left == right,
                Token::NotEq => left != right,
                Token::Lt => cmp == Some(std::cmp::Ordering::Less),
                Token::LtEq => cmp == Some(std::cmp::Ordering::Less) || cmp == Some(std::cmp::Ordering::Equal),
                Token::Gt => cmp == Some(std::cmp::Ordering::Greater),
                Token::GtEq => cmp == Some(std::cmp::Ordering::Greater) || cmp == Some(std::cmp::Ordering::Equal),
                _ => false,
            };
            Ok(Val::Bool(res))
        }
        _ => Ok(left),
    }
}

fn parse_add_sub(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let mut left = parse_mul_div(p, rt)?;
    while p.check(&Token::Plus) || p.check(&Token::Minus) {
        let op = p.advance().clone();
        let right = parse_mul_div(p, rt)?;
        match op {
            Token::Plus => {
                if let (Val::Str(a), Val::Str(b)) = (&left, &right) {
                    left = Val::Str(format!("{a}{b}"));
                } else {
                    left = Val::Num(left.to_f64()? + right.to_f64()?);
                }
            }
            Token::Minus => {
                left = Val::Num(left.to_f64()? - right.to_f64()?);
            }
            _ => unreachable!(),
        }
    }
    Ok(left)
}

fn parse_mul_div(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    let mut left = parse_unary(p, rt)?;
    while p.check(&Token::Star) || p.check(&Token::Slash) || p.check(&Token::Percent) || p.check(&Token::Power) {
        let op = p.advance().clone();
        let right = parse_unary(p, rt)?;
        match op {
            Token::Star => left = Val::Num(left.to_f64()? * right.to_f64()?),
            Token::Slash => {
                let r = right.to_f64()?;
                if r == 0.0 { return Err(ScriptError::Error("division by zero".into())); }
                left = Val::Num(left.to_f64()? / r);
            }
            Token::Percent => {
                let r = right.to_f64()?;
                if r == 0.0 { return Err(ScriptError::Error("modulo by zero".into())); }
                left = Val::Num(left.to_f64()? % r);
            }
            Token::Power => left = Val::Num(left.to_f64()?.powf(right.to_f64()?)),
            _ => unreachable!(),
        }
    }
    Ok(left)
}

fn parse_unary(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    if p.match_token(&Token::Minus) {
        let v = parse_unary(p, rt)?;
        Ok(Val::Num(-v.to_f64()?))
    } else if p.match_token(&Token::Plus) {
        parse_unary(p, rt)
    } else {
        parse_primary(p, rt)
    }
}

fn parse_primary(p: &mut Parser, rt: &mut Runtime) -> Result<Val, ScriptError> {
    if p.match_token(&Token::LParen) {
        let val = parse_expr(p, rt)?;
        if !p.match_token(&Token::RParen) {
            return Err(ScriptError::Error("expected ')'".into()));
        }
        if p.match_token(&Token::Dot) {
            return Err(ScriptError::Error("attribute access is not allowed".into()));
        }
        return Ok(val);
    }

    let token = p.advance().clone();
    match token {
        Token::Number(n) => Ok(Val::Num(n)),
        Token::String(s) => Ok(Val::Str(s)),
        Token::FString(s) => Ok(Val::Str(eval_fstring(&s, rt)?)),
        Token::KwTrue => Ok(Val::Bool(true)),
        Token::KwFalse => Ok(Val::Bool(false)),
        Token::KwNone => Ok(Val::Nil),
        Token::Ident(name) => {
            if p.match_token(&Token::LParen) {
                let mut args = Vec::new();
                if !p.check(&Token::RParen) {
                    loop {
                        args.push(parse_expr(p, rt)?);
                        if !p.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                if !p.match_token(&Token::RParen) {
                    return Err(ScriptError::Error("expected ')' after function arguments".into()));
                }
                rt.call_func(&name, args)
            } else if p.match_token(&Token::Dot) {
                Err(ScriptError::Error("attribute access is not allowed".into()))
            } else if let Some(v) = rt.vars.get(&name) {
                Ok(v.clone())
            } else {
                Err(ScriptError::Error(format!("undefined variable: {name}")))
            }
        }
        other => Err(ScriptError::Error(format!("unexpected token: {:?}", other))),
    }
}

/// Loop control signal bubbling out of a block: a block that receives
/// Break/Continue first consumes the rest of its own construct's tokens,
/// then propagates the signal to the enclosing loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Normal,
    Break,
    Continue,
}

fn execute_statements(p: &mut Parser, rt: &mut Runtime) -> Result<Flow, ScriptError> {
    while !p.check(&Token::Eof) && !p.check(&Token::Dedent) {
        while p.match_token(&Token::Newline) {}
        if p.check(&Token::Eof) || p.check(&Token::Dedent) {
            break;
        }

        rt.tick(1)?;

        if p.match_token(&Token::KwBreak) {
            p.match_token(&Token::Newline);
            return Ok(Flow::Break);
        }
        if p.match_token(&Token::KwContinue) {
            p.match_token(&Token::Newline);
            return Ok(Flow::Continue);
        }

        if p.match_token(&Token::KwIf) {
            // if / elif* / else? chain: branches run in order until one is
            // taken; the rest are skipped without executing.
            let mut flow = Flow::Normal;
            let mut decided = false;
            loop {
                let cond = parse_expr(p, rt)?;
                if !p.match_token(&Token::Colon) {
                    return Err(ScriptError::Error("expected ':' after if/elif condition".into()));
                }
                if !p.match_token(&Token::Newline) {
                    return Err(ScriptError::Error("expected newline after ':'".into()));
                }
                if !p.match_token(&Token::Indent) {
                    return Err(ScriptError::Error("expected indent in if block".into()));
                }
                if !decided && cond.is_truthy() {
                    flow = execute_statements(p, rt)?;
                    if p.match_token(&Token::Dedent) {}
                    decided = true;
                } else {
                    skip_block(p)?;
                }
                if p.match_token(&Token::KwElif) {
                    continue;
                }
                if p.match_token(&Token::KwElse) {
                    if !p.match_token(&Token::Colon) || !p.match_token(&Token::Newline) || !p.match_token(&Token::Indent) {
                        return Err(ScriptError::Error("invalid else block".into()));
                    }
                    if !decided {
                        flow = execute_statements(p, rt)?;
                        if p.match_token(&Token::Dedent) {}
                    } else {
                        skip_block(p)?;
                    }
                }
                break;
            }
            if flow != Flow::Normal {
                return Ok(flow);
            }
        } else if p.match_token(&Token::KwWhile) {
            let cond_start = p.pos;
            let mut iterations = 0usize;
            loop {
                iterations += 1;
                if iterations > MAX_LOOP_ITEMS {
                    return Err(ScriptError::Error(format!("while loops are limited to {MAX_LOOP_ITEMS} iterations")));
                }
                p.pos = cond_start;
                let cond = parse_expr(p, rt)?;
                if !p.match_token(&Token::Colon) || !p.match_token(&Token::Newline) || !p.match_token(&Token::Indent) {
                    return Err(ScriptError::Error("expected ':' and indent in while loop".into()));
                }
                if !cond.is_truthy() {
                    skip_block(p)?;
                    break;
                }
                let flow = execute_statements(p, rt)?;
                match flow {
                    Flow::Normal => {
                        if p.match_token(&Token::Dedent) {}
                    }
                    Flow::Continue => {
                        skip_block(p)?;
                    }
                    Flow::Break => {
                        skip_block(p)?;
                        break;
                    }
                }
            }
        } else if p.match_token(&Token::KwFor) {
            let var_name = match p.advance().clone() {
                Token::Ident(name) => name,
                _ => return Err(ScriptError::Error("expected identifier in for loop".into())),
            };
            if !p.match_token(&Token::KwIn) {
                return Err(ScriptError::Error("expected 'in' in for loop".into()));
            }
            let _func_name = match p.advance().clone() {
                Token::Ident(name) if name == "range" => name,
                _ => return Err(ScriptError::Error("for loops must use range()".into())),
            };
            if !p.match_token(&Token::LParen) {
                return Err(ScriptError::Error("expected '(' after range".into()));
            }
            let mut range_args = Vec::new();
            if !p.check(&Token::RParen) {
                loop {
                    range_args.push(parse_expr(p, rt)?);
                    if !p.match_token(&Token::Comma) { break; }
                }
            }
            if !p.match_token(&Token::RParen) {
                return Err(ScriptError::Error("expected ')' after range arguments".into()));
            }
            if !p.match_token(&Token::Colon) || !p.match_token(&Token::Newline) || !p.match_token(&Token::Indent) {
                return Err(ScriptError::Error("expected ':' and indent in for loop".into()));
            }

            let (start, end, step) = match range_args.len() {
                1 => (0i64, range_args[0].to_f64()? as i64, 1i64),
                2 => (range_args[0].to_f64()? as i64, range_args[1].to_f64()? as i64, 1i64),
                3 => (range_args[0].to_f64()? as i64, range_args[1].to_f64()? as i64, range_args[2].to_f64()? as i64),
                _ => return Err(ScriptError::Error("range() expects 1 to 3 args".into())),
            };

            let count = if step > 0 && end > start {
                ((end - start + step - 1) / step) as usize
            } else if step < 0 && start > end {
                ((start - end - step - 1) / (-step)) as usize
            } else {
                0
            };

            if count > MAX_LOOP_ITEMS {
                return Err(ScriptError::Error(format!("loops are limited to {MAX_LOOP_ITEMS} items")));
            }

            let loop_body_start = p.pos;
            let mut current = start;
            let mut broke = false;
            for _ in 0..count {
                rt.vars.insert(var_name.clone(), Val::Num(current as f64));
                p.pos = loop_body_start;
                let flow = execute_statements(p, rt)?;
                current += step;
                match flow {
                    Flow::Normal => {}
                    Flow::Continue => {
                        skip_block(p)?;
                    }
                    Flow::Break => {
                        skip_block(p)?;
                        broke = true;
                        break;
                    }
                }
            }
            if !broke {
                if p.match_token(&Token::Dedent) {}
            }
        } else if let Token::Ident(name) = p.peek().clone() {
            let assign_op = p.tokens.get(p.pos + 1).cloned();
            let is_assign = matches!(
                assign_op,
                Some(Token::Eq) | Some(Token::PlusEq) | Some(Token::MinusEq) | Some(Token::StarEq) | Some(Token::SlashEq)
            );
            if is_assign {
                p.advance(); // skip name
                p.advance(); // skip operator
                if name.starts_with('_') || ["width", "height", "pi"].contains(&name.as_str()) {
                    return Err(ScriptError::Error(format!("cannot assign to {name}")));
                }
                let rhs = parse_expr(p, rt)?;
                let val = match assign_op {
                    Some(Token::Eq) => rhs,
                    Some(op) => {
                        let cur = rt
                            .vars
                            .get(&name)
                            .cloned()
                            .ok_or_else(|| ScriptError::Error(format!("undefined variable: {name}")))?;
                        match op {
                            Token::PlusEq => {
                                if let (Val::Str(a), Val::Str(b)) = (&cur, &rhs) {
                                    Val::Str(format!("{a}{b}"))
                                } else {
                                    Val::Num(cur.to_f64()? + rhs.to_f64()?)
                                }
                            }
                            Token::MinusEq => Val::Num(cur.to_f64()? - rhs.to_f64()?),
                            Token::StarEq => Val::Num(cur.to_f64()? * rhs.to_f64()?),
                            Token::SlashEq => {
                                let r = rhs.to_f64()?;
                                if r == 0.0 {
                                    return Err(ScriptError::Error("division by zero".into()));
                                }
                                Val::Num(cur.to_f64()? / r)
                            }
                            _ => unreachable!(),
                        }
                    }
                    _ => unreachable!(),
                };
                rt.vars.insert(name, val);
                p.match_token(&Token::Newline);
            } else {
                parse_expr(p, rt)?;
                p.match_token(&Token::Newline);
            }
        } else {
            parse_expr(p, rt)?;
            p.match_token(&Token::Newline);
        }
    }
    Ok(Flow::Normal)
}

fn skip_block(p: &mut Parser) -> Result<(), ScriptError> {
    let mut depth = 1;
    while depth > 0 && !p.check(&Token::Eof) {
        if p.match_token(&Token::Indent) {
            depth += 1;
        } else if p.match_token(&Token::Dedent) {
            depth -= 1;
        } else {
            p.advance();
        }
    }
    Ok(())
}

pub fn execute_script(
    code: &str,
    width: u32,
    height: u32,
    context: &Value,
) -> Result<ScriptOutput, ScriptError> {
    if code.len() > 8000 {
        return Err(ScriptError::Error("script is too long".into()));
    }
    let tokens = tokenize(code)?;
    if tokens.len() > MAX_AST_NODES {
        return Err(ScriptError::Error("script is too complex".into()));
    }

    let mut runtime = Runtime::new(width.max(1), height.max(1), context);
    let mut parser = Parser::new(&tokens);
    match execute_statements(&mut parser, &mut runtime)? {
        Flow::Normal => {}
        Flow::Break => return Err(ScriptError::Error("break outside a loop".into())),
        Flow::Continue => return Err(ScriptError::Error("continue outside a loop".into())),
    }

    if runtime.drew {
        Ok(ScriptOutput {
            image: Some(runtime.image),
            text: None,
        })
    } else {
        Ok(ScriptOutput {
            image: None,
            text: Some(runtime.text_output.unwrap_or_default()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_examples_execute_with_context() {
        let context = json!({
            "homeAssistant": {
                "sensor.office_temperature": {"state": "21.5", "attributes": {}},
                "sensor.battery_level": {"state": "72", "attributes": {}},
                "binary_sensor.front_door": {"state": "off", "attributes": {}}
            }
        });

        let out0 = execute_script(SCRIPT_EXAMPLES[0].code, 200, 60, &context).unwrap();
        assert_eq!(out0.text.as_deref(), Some("Office 21.5°C"));

        for ex in SCRIPT_EXAMPLES {
            let out = execute_script(ex.code, 200, 60, &context).unwrap();
            assert!(out.text.is_some() || out.image.is_some());
        }
    }

    #[test]
    #[test]
    fn test_label_metrics_match_glyph_advance() {
        // 4 chars at size 8 (scale 1) advance 8px each = 32px wide.
        assert_eq!(label_metrics("MMMM", 8), (1, 32));
        // Size 14 snaps to scale 2; width counts chars, not UTF-8 bytes
        // ("°C" is 2 chars but 3 bytes).
        assert_eq!(label_metrics("21°C", 14), (2, 64));
    }

    #[test]
    fn test_centered_label_stays_inside_block() {
        // A centered label as wide as the block must start at x=0 and keep
        // its ink inside the canvas (regression: the 6px/char measurement
        // shifted it right and clipped the end).
        let ctx = json!({});
        let out = execute_script("label(16, 0, \"MMMM\", \"#ffffff\", 8, \"ma\")", 32, 8, &ctx).unwrap();
        let image = out.image.expect("label draws an image");
        let mut max_x = 0;
        for (x, _y, pixel) in image.enumerate_pixels() {
            if pixel[3] > 0 {
                max_x = max_x.max(x);
            }
        }
        assert!(max_x < 32, "ink must stay inside the 32px block, got max_x={max_x}");
        assert!(max_x >= 24, "the full 4-char label should render, got max_x={max_x}");
    }

    #[test]
    fn test_elif_chain() {
        let ctx = json!({});
        let pick = |v: i32| {
            let code = format!(
                "v = {v}\nif v == 1:\n    text(\"one\")\nelif v == 2:\n    text(\"two\")\nelif v == 3:\n    text(\"three\")\nelse:\n    text(\"many\")"
            );
            execute_script(&code, 100, 50, &ctx).unwrap().text.unwrap()
        };
        assert_eq!(pick(1), "one");
        assert_eq!(pick(2), "two");
        assert_eq!(pick(3), "three");
        assert_eq!(pick(9), "many");
        // No else: a non-matching chain is a no-op.
        let out = execute_script("v = 5\nif v == 1:\n    text(\"one\")\nelif v == 2:\n    text(\"two\")", 100, 50, &ctx).unwrap();
        assert_eq!(out.text.as_deref(), Some(""));
    }

    #[test]
    fn test_while_break_continue_and_augmented_assign() {
        let ctx = json!({});
        // Sum the odd numbers 1..=9: while + continue + break + +=.
        let code = "total = 0\nn = 0\nwhile n < 20:\n    n += 1\n    if n > 9:\n        break\n    if n % 2 == 0:\n        continue\n    total += n\ntext(str(total))";
        let out = execute_script(code, 100, 50, &ctx).unwrap();
        assert_eq!(out.text.as_deref(), Some("25"));
        // while True with a break terminates.
        let code2 = "n = 0\nwhile True:\n    n += 1\n    if n == 3:\n        break\ntext(str(n))";
        let out2 = execute_script(code2, 100, 50, &ctx).unwrap();
        assert_eq!(out2.text.as_deref(), Some("3"));
        // An unbounded while hits the iteration cap, not the op budget first.
        let err = execute_script("while True:\n    x = 1", 100, 50, &ctx).unwrap_err();
        assert!(format!("{err}").contains("while"), "cap error: {err}");
        // break outside a loop is an error.
        assert!(execute_script("break", 100, 50, &ctx).is_err());
    }

    #[test]
    fn test_for_break_and_continue() {
        let ctx = json!({});
        let code = "total = 0\nfor i in range(10):\n    if i == 3:\n        break\n    total += i\ntext(str(total))";
        assert_eq!(execute_script(code, 100, 50, &ctx).unwrap().text.as_deref(), Some("3"));
        let code2 = "total = 0\nfor i in range(6):\n    if i % 2 == 0:\n        continue\n    total += i\ntext(str(total))";
        assert_eq!(execute_script(code2, 100, 50, &ctx).unwrap().text.as_deref(), Some("9"));
    }

    #[test]
    fn test_ternary_and_in() {
        let ctx = json!({});
        let code = "x = 7\nlabel_text = \"big\" if x > 5 else \"small\"\ntext(label_text)";
        assert_eq!(execute_script(code, 100, 50, &ctx).unwrap().text.as_deref(), Some("big"));
        let code2 = "x = 2\ntext(\"big\" if x > 5 else \"small\")";
        assert_eq!(execute_script(code2, 100, 50, &ctx).unwrap().text.as_deref(), Some("small"));
        // Nested ternary in the else branch.
        let code3 = "x = 2\ntext(\"a\" if x == 1 else \"b\" if x == 2 else \"c\")";
        assert_eq!(execute_script(code3, 100, 50, &ctx).unwrap().text.as_deref(), Some("b"));
        // Ternary inside call arguments, before a comma.
        let code4 = "text(str(max(1 if False else 4, 3)))";
        assert_eq!(execute_script(code4, 100, 50, &ctx).unwrap().text.as_deref(), Some("4"));
        let code5 = "state = \"heating\"\nif \"heat\" in state:\n    text(\"warm\")\nelse:\n    text(\"cold\")";
        assert_eq!(execute_script(code5, 100, 50, &ctx).unwrap().text.as_deref(), Some("warm"));
    }

    #[test]
    fn test_new_builtins() {
        let ctx = json!({});
        let cases: &[(&str, &str)] = &[
            ("text(str(len(\"hello\")))", "5"),
            ("text(str(int(3.9)))", "3"),
            ("text(str(float(\"2.5\") + 1))", "3.5"),
            ("text(str(floor(2.7)) + \"/\" + str(ceil(2.1)))", "2/3"),
            ("text(str(pow(2, 10)))", "1024"),
            ("text(str(lerp(0, 100, 0.25)))", "25"),
            ("text(str(map_range(15, 10, 20, 0, 100)))", "50"),
            ("text(upper(\"on\") + \"/\" + lower(\"OFF\"))", "ON/off"),
            ("text(rgb(255, 128, 0))", "#ff8000"),
            ("text(str(bool(\"x\")) + \"/\" + str(bool(\"\")))", "True/False"),
            ("s = \"a\"\ns += \"b\"\ns += str(1)\ntext(s)", "ab1"),
            ("text(f\"{7:02d}:{3:02d}\")", "07:03"),
        ];
        for (code, want) in cases {
            let out = execute_script(code, 100, 50, &ctx).unwrap();
            assert_eq!(out.text.as_deref(), Some(*want), "script: {code}");
        }
    }

    #[test]
    fn test_source_and_jq() {
        let ctx = json!({
            "external": {
                "weather": "{\"current\": {\"temp_c\": 21.5, \"label\": \"Sunny\"}, \"tags\": [\"a\", \"b\"]}"
            }
        });
        let num = execute_script("t = jq(source(\"weather\"), \".current.temp_c\")\ntext(str(t))", 100, 50, &ctx).unwrap();
        assert_eq!(num.text.as_deref(), Some("21.5"));
        let string = execute_script("text(jq(source(\"weather\"), \".current.label\"))", 100, 50, &ctx).unwrap();
        assert_eq!(string.text.as_deref(), Some("Sunny"));
        // Objects come back as JSON text; missing fields and unparsable
        // sources read as None.
        let obj = execute_script("text(jq(source(\"weather\"), \".current\"))", 100, 50, &ctx).unwrap();
        assert_eq!(obj.text.as_deref(), Some("{\"label\":\"Sunny\",\"temp_c\":21.5}"));
        let missing = execute_script("if jq(source(\"weather\"), \".nope\") == None:\n    text(\"none\")", 100, 50, &ctx).unwrap();
        assert_eq!(missing.text.as_deref(), Some("none"));
        let bad_source = execute_script("if jq(source(\"absent\"), \".x\") == None:\n    text(\"none\")", 100, 50, &ctx).unwrap();
        assert_eq!(bad_source.text.as_deref(), Some("none"));
        // A bad jq program is a hard error, unlike bad data.
        assert!(execute_script("text(jq(source(\"weather\"), \".[\"))", 100, 50, &ctx).is_err());
    }

    #[test]
    fn test_security_rejections() {
        let ctx = json!({});
        assert!(execute_script("import os", 100, 50, &ctx).is_err());
        assert!(execute_script("text((1).__class__)", 100, 50, &ctx).is_err());
        assert!(execute_script("for x in range(101):\n    text(x)", 100, 50, &ctx).is_err());
    }
}
