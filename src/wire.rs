#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    List(Vec<Json>),
    Map(Vec<(String, Json)>),
}

impl Json {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Map(held) => held
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, one)| one),
            _ => None,
        }
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(held) => Some(held),
            _ => None,
        }
    }

    #[must_use]
    pub fn int(&self) -> Option<i64> {
        match self {
            Self::Int(held) => Some(*held),
            _ => None,
        }
    }

    #[must_use]
    pub fn list(&self) -> Option<&[Json]> {
        match self {
            Self::List(held) => Some(held),
            _ => None,
        }
    }

    #[must_use]
    pub fn counted(&self, key: &str) -> Option<u32> {
        u32::try_from(self.get(key)?.int()?).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Torn {
    Ended { at: usize },
    Stray { at: usize, saw: char },
    TooDeep { at: usize },
    NotANumber { at: usize },
}

impl std::fmt::Display for Torn {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended { at } => write!(out, "{at}"),
            Self::Stray { at, saw } => write!(out, "{at} {saw:?}"),
            Self::TooDeep { at } => write!(out, "{at}"),
            Self::NotANumber { at } => write!(out, "{at}"),
        }
    }
}

impl std::error::Error for Torn {}

const DEEPEST: usize = 64;

struct Reading<'held> {
    held: &'held [u8],
    at: usize,
}

impl Reading<'_> {
    fn peek(&self) -> Option<u8> {
        self.held.get(self.at).copied()
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn take(&mut self, want: u8) -> Result<(), Torn> {
        match self.peek() {
            Some(saw) if saw == want => {
                self.at += 1;
                Ok(())
            }
            Some(saw) => Err(Torn::Stray {
                at: self.at,
                saw: char::from(saw),
            }),
            None => Err(Torn::Ended { at: self.at }),
        }
    }

    fn word(&mut self, want: &str) -> Result<(), Torn> {
        for one in want.bytes() {
            self.take(one)?;
        }
        Ok(())
    }
}

fn stringed(reading: &mut Reading<'_>) -> Result<String, Torn> {
    reading.take(b'"')?;
    let mut held = String::new();
    loop {
        let Some(one) = reading.peek() else {
            return Err(Torn::Ended { at: reading.at });
        };
        reading.at += 1;
        match one {
            b'"' => return Ok(held),
            b'\\' => {
                let Some(next) = reading.peek() else {
                    return Err(Torn::Ended { at: reading.at });
                };
                reading.at += 1;
                match next {
                    b'"' => held.push('"'),
                    b'\\' => held.push('\\'),
                    b'/' => held.push('/'),
                    b'n' => held.push('\n'),
                    b't' => held.push('\t'),
                    b'r' => held.push('\r'),
                    b'b' => held.push('\u{8}'),
                    b'f' => held.push('\u{c}'),
                    b'u' => {
                        let mut code = 0_u32;
                        for _ in 0..4 {
                            let Some(digit) = reading.peek() else {
                                return Err(Torn::Ended { at: reading.at });
                            };
                            let Some(value) = char::from(digit).to_digit(16) else {
                                return Err(Torn::Stray {
                                    at: reading.at,
                                    saw: char::from(digit),
                                });
                            };
                            reading.at += 1;
                            code = code * 16 + value;
                        }
                        held.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                    }
                    saw => {
                        return Err(Torn::Stray {
                            at: reading.at,
                            saw: char::from(saw),
                        });
                    }
                }
            }
            _ => {
                let from = reading.at - 1;
                let mut to = reading.at;
                while reading.peek().is_some_and(|one| one >= 0x80) {
                    reading.at += 1;
                    to += 1;
                }
                match reading.held.get(from..to).map(String::from_utf8_lossy) {
                    Some(part) => held.push_str(&part),
                    None => return Err(Torn::Ended { at: from }),
                }
            }
        }
    }
}

fn numbered(reading: &mut Reading<'_>) -> Result<Json, Torn> {
    let from = reading.at;
    if reading.peek() == Some(b'-') {
        reading.at += 1;
    }
    while reading
        .peek()
        .is_some_and(|one| one.is_ascii_digit() || matches!(one, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        reading.at += 1;
    }
    let held = reading
        .held
        .get(from..reading.at)
        .map(String::from_utf8_lossy)
        .ok_or(Torn::NotANumber { at: from })?;
    held.parse::<i64>()
        .map(Json::Int)
        .map_err(|_| Torn::NotANumber { at: from })
}

fn valued(reading: &mut Reading<'_>, deep: usize) -> Result<Json, Torn> {
    if deep > DEEPEST {
        return Err(Torn::TooDeep { at: reading.at });
    }
    reading.skip();
    match reading.peek() {
        None => Err(Torn::Ended { at: reading.at }),
        Some(b'n') => reading.word("null").map(|()| Json::Null),
        Some(b't') => reading.word("true").map(|()| Json::Bool(true)),
        Some(b'f') => reading.word("false").map(|()| Json::Bool(false)),
        Some(b'"') => stringed(reading).map(Json::Text),
        Some(b'[') => {
            reading.at += 1;
            let mut held = Vec::new();
            reading.skip();
            if reading.peek() == Some(b']') {
                reading.at += 1;
                return Ok(Json::List(held));
            }
            loop {
                held.push(valued(reading, deep + 1)?);
                reading.skip();
                match reading.peek() {
                    Some(b',') => reading.at += 1,
                    Some(b']') => {
                        reading.at += 1;
                        return Ok(Json::List(held));
                    }
                    Some(saw) => {
                        return Err(Torn::Stray {
                            at: reading.at,
                            saw: char::from(saw),
                        });
                    }
                    None => return Err(Torn::Ended { at: reading.at }),
                }
            }
        }
        Some(b'{') => {
            reading.at += 1;
            let mut held = Vec::new();
            reading.skip();
            if reading.peek() == Some(b'}') {
                reading.at += 1;
                return Ok(Json::Map(held));
            }
            loop {
                reading.skip();
                let name = stringed(reading)?;
                reading.skip();
                reading.take(b':')?;
                held.push((name, valued(reading, deep + 1)?));
                reading.skip();
                match reading.peek() {
                    Some(b',') => reading.at += 1,
                    Some(b'}') => {
                        reading.at += 1;
                        return Ok(Json::Map(held));
                    }
                    Some(saw) => {
                        return Err(Torn::Stray {
                            at: reading.at,
                            saw: char::from(saw),
                        });
                    }
                    None => return Err(Torn::Ended { at: reading.at }),
                }
            }
        }
        Some(_) => numbered(reading),
    }
}

pub fn read(held: &str) -> Result<Json, Torn> {
    let mut reading = Reading {
        held: held.as_bytes(),
        at: 0,
    };
    let one = valued(&mut reading, 0)?;
    reading.skip();
    Ok(one)
}

fn quoted(held: &str, out: &mut String) {
    out.push('"');
    for one in held.chars() {
        match one {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            one if (one as u32) < 0x20 => {
                out.push_str("\\u");
                for shift in [12, 8, 4, 0] {
                    let digit = (one as u32 >> shift) & 0xf;
                    out.push(char::from_digit(digit, 16).unwrap_or('0'));
                }
            }
            one => out.push(one),
        }
    }
    out.push('"');
}

pub fn write(held: &Json, out: &mut String) {
    match held {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Int(one) => out.push_str(&one.to_string()),
        Json::Text(one) => quoted(one, out),
        Json::List(many) => {
            out.push('[');
            for (at, one) in many.iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                write(one, out);
            }
            out.push(']');
        }
        Json::Map(many) => {
            out.push('{');
            for (at, (name, one)) in many.iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                quoted(name, out);
                out.push(':');
                write(one, out);
            }
            out.push('}');
        }
    }
}

#[must_use]
pub fn written(held: &Json) -> String {
    let mut out = String::new();
    write(held, &mut out);
    out
}

#[must_use]
pub fn text(held: &str) -> Json {
    Json::Text(held.to_string())
}

#[must_use]
pub fn counted(held: u32) -> Json {
    Json::Int(i64::from(held))
}

#[must_use]
pub fn map(held: Vec<(&str, Json)>) -> Json {
    Json::Map(
        held.into_iter()
            .map(|(name, one)| (name.to_string(), one))
            .collect(),
    )
}
