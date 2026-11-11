//! The JSON a binary glTF file carries and a material map is written in.
//! Unlike [`crate::wire`], numbers keep their fraction: glTF writes its
//! transforms in floats.

/// One JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The value under `key` of a map.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Map(held) => held
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, one)| one),
            _ => None,
        }
    }

    #[must_use]
    pub const fn number(&self) -> Option<f64> {
        match self {
            Self::Number(held) => Some(*held),
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
    pub fn list(&self) -> Option<&[Self]> {
        match self {
            Self::List(held) => Some(held),
            _ => None,
        }
    }

    #[must_use]
    pub fn entries(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Map(held) => Some(held),
            _ => None,
        }
    }

    /// The value as a whole number that fits a `u64`.
    #[must_use]
    pub fn whole(&self) -> Option<u64> {
        let held = self.number()?;
        if held.fract() == 0.0 && (0.0..=WHOLE_MOST).contains(&held) {
            Some(held as u64)
        } else {
            None
        }
    }
}

/// The largest whole number a JSON number is read as exactly.
const WHOLE_MOST: f64 = 9_007_199_254_740_992.0;

/// Where and why JSON could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Torn {
    /// The text ended inside a value.
    Ended { at: usize },
    /// A byte that cannot stand where it does.
    Stray { at: usize, saw: u8 },
    /// Values nested deeper than [`DEEPEST`].
    TooDeep { at: usize },
    /// A number that does not read as a finite float.
    NotANumber { at: usize },
    /// A string that is not UTF-8.
    NotText { at: usize },
    /// More than whitespace after the value.
    Trailing { at: usize },
}

impl std::fmt::Display for Torn {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended { at } => write!(out, "the JSON ends inside a value at byte {at}"),
            Self::Stray { at, saw } => write!(out, "the JSON has a stray byte {saw:#04x} at {at}"),
            Self::TooDeep { at } => write!(out, "the JSON nests too deep at byte {at}"),
            Self::NotANumber { at } => write!(out, "the JSON has a bad number at byte {at}"),
            Self::NotText { at } => write!(out, "the JSON has a string that is not UTF-8 at {at}"),
            Self::Trailing { at } => write!(out, "the JSON goes on past its value at byte {at}"),
        }
    }
}

impl std::error::Error for Torn {}

/// How deep lists and maps may nest.
pub const DEEPEST: usize = 64;

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
            Some(saw) => Err(Torn::Stray { at: self.at, saw }),
            None => Err(Torn::Ended { at: self.at }),
        }
    }

    fn word(&mut self, want: &str) -> Result<(), Torn> {
        want.bytes().try_for_each(|one| self.take(one))
    }

    fn hex(&mut self) -> Result<u32, Torn> {
        let mut code = 0;
        for _ in 0..4 {
            let Some(digit) = self.peek() else {
                return Err(Torn::Ended { at: self.at });
            };
            let Some(value) = char::from(digit).to_digit(16) else {
                return Err(Torn::Stray {
                    at: self.at,
                    saw: digit,
                });
            };
            self.at += 1;
            code = code * 16 + value;
        }
        Ok(code)
    }
}

fn escaped(reading: &mut Reading<'_>, held: &mut String) -> Result<(), Torn> {
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
            let first = reading.hex()?;
            let code = if (0xD800..0xDC00).contains(&first)
                && reading.held.get(reading.at..reading.at + 2) == Some(b"\\u")
            {
                reading.at += 2;
                let second = reading.hex()?;
                if (0xDC00..0xE000).contains(&second) {
                    0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                } else {
                    0xFFFD
                }
            } else {
                first
            };
            held.push(char::from_u32(code).unwrap_or('\u{fffd}'));
        }
        saw => {
            return Err(Torn::Stray {
                at: reading.at - 1,
                saw,
            });
        }
    }
    Ok(())
}

fn stringed(reading: &mut Reading<'_>) -> Result<String, Torn> {
    reading.take(b'"')?;
    let mut held = String::new();
    loop {
        let from = reading.at;
        while reading
            .peek()
            .is_some_and(|one| one != b'"' && one != b'\\' && one >= 0x20)
        {
            reading.at += 1;
        }
        let run = reading
            .held
            .get(from..reading.at)
            .ok_or(Torn::Ended { at: from })?;
        held.push_str(std::str::from_utf8(run).map_err(|_| Torn::NotText { at: from })?);
        match reading.peek() {
            None => return Err(Torn::Ended { at: reading.at }),
            Some(b'"') => {
                reading.at += 1;
                return Ok(held);
            }
            Some(b'\\') => {
                reading.at += 1;
                escaped(reading, &mut held)?;
            }
            Some(saw) => {
                return Err(Torn::Stray {
                    at: reading.at,
                    saw,
                });
            }
        }
    }
}

fn numbered(reading: &mut Reading<'_>) -> Result<Value, Torn> {
    let from = reading.at;
    while reading
        .peek()
        .is_some_and(|one| one.is_ascii_digit() || matches!(one, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        reading.at += 1;
    }
    let held = reading
        .held
        .get(from..reading.at)
        .and_then(|run| std::str::from_utf8(run).ok())
        .ok_or(Torn::NotANumber { at: from })?;
    let starts = held.bytes().next();
    if !matches!(starts, Some(b'-' | b'0'..=b'9')) {
        return match reading.peek() {
            Some(saw) => Err(Torn::Stray { at: from, saw }),
            None => Err(Torn::Ended { at: from }),
        };
    }
    held.parse::<f64>()
        .ok()
        .filter(|one| one.is_finite())
        .map(Value::Number)
        .ok_or(Torn::NotANumber { at: from })
}

fn listed(reading: &mut Reading<'_>, deep: usize) -> Result<Value, Torn> {
    reading.take(b'[')?;
    let mut held = Vec::new();
    reading.skip();
    if reading.peek() == Some(b']') {
        reading.at += 1;
        return Ok(Value::List(held));
    }
    loop {
        held.push(valued(reading, deep + 1)?);
        reading.skip();
        match reading.peek() {
            Some(b',') => reading.at += 1,
            Some(b']') => {
                reading.at += 1;
                return Ok(Value::List(held));
            }
            Some(saw) => {
                return Err(Torn::Stray {
                    at: reading.at,
                    saw,
                });
            }
            None => return Err(Torn::Ended { at: reading.at }),
        }
    }
}

fn mapped(reading: &mut Reading<'_>, deep: usize) -> Result<Value, Torn> {
    reading.take(b'{')?;
    let mut held = Vec::new();
    reading.skip();
    if reading.peek() == Some(b'}') {
        reading.at += 1;
        return Ok(Value::Map(held));
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
                return Ok(Value::Map(held));
            }
            Some(saw) => {
                return Err(Torn::Stray {
                    at: reading.at,
                    saw,
                });
            }
            None => return Err(Torn::Ended { at: reading.at }),
        }
    }
}

fn valued(reading: &mut Reading<'_>, deep: usize) -> Result<Value, Torn> {
    if deep > DEEPEST {
        return Err(Torn::TooDeep { at: reading.at });
    }
    reading.skip();
    match reading.peek() {
        None => Err(Torn::Ended { at: reading.at }),
        Some(b'n') => reading.word("null").map(|()| Value::Null),
        Some(b't') => reading.word("true").map(|()| Value::Bool(true)),
        Some(b'f') => reading.word("false").map(|()| Value::Bool(false)),
        Some(b'"') => stringed(reading).map(Value::Text),
        Some(b'[') => listed(reading, deep),
        Some(b'{') => mapped(reading, deep),
        Some(_) => numbered(reading),
    }
}

/// Reads one JSON value from `held`; only whitespace, or the NUL bytes
/// some writers pad a glTF chunk with, may follow it.
pub fn read(held: &[u8]) -> Result<Value, Torn> {
    let mut reading = Reading { held, at: 0 };
    let one = valued(&mut reading, 0)?;
    reading.skip();
    while reading.peek() == Some(0) {
        reading.at += 1;
    }
    if reading.at < held.len() {
        return Err(Torn::Trailing { at: reading.at });
    }
    Ok(one)
}
