use crate::wire::{Json, map, text};

use super::errors::{Code, Refusal, shown, shown_value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Whole,
    Keys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrong {
    Type,
    Missing,
    Unknown,
    Duplicate,
    Range,
    Choice,
    Long,
    Short,
    Count,
}

pub const KEYWORDS: [&str; 12] = [
    "type",
    "description",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
    "enum",
    "default",
];

#[must_use]
pub fn property(kind: &str, what: &str) -> Json {
    map(vec![("type", text(kind)), ("description", text(what))])
}

#[must_use]
pub fn with(held: Json, more: Vec<(&str, Json)>) -> Json {
    match held {
        Json::Map(mut held) => {
            held.extend(more.into_iter().map(|(key, one)| (key.to_owned(), one)));
            Json::Map(held)
        }
        other => other,
    }
}

#[must_use]
pub fn whole(what: &str, least: i64, most: i64) -> Json {
    with(
        property("integer", what),
        vec![("minimum", Json::Int(least)), ("maximum", Json::Int(most))],
    )
}

#[must_use]
pub fn word() -> Json {
    map(vec![
        ("type", text("integer")),
        ("minimum", Json::Int(0)),
        ("maximum", Json::Int(i64::from(u32::MAX))),
    ])
}

#[must_use]
pub fn choice(what: &str, allowed: &[&str]) -> Json {
    with(
        property("string", what),
        vec![(
            "enum",
            Json::List(allowed.iter().map(|one| text(one)).collect()),
        )],
    )
}

#[must_use]
pub fn object(properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
    map(vec![
        ("type", text("object")),
        ("properties", map(properties)),
        (
            "required",
            Json::List(required.iter().map(|one| text(one)).collect()),
        ),
        ("additionalProperties", Json::Bool(false)),
    ])
}

#[must_use]
pub fn closed(schema: Json) -> Json {
    match schema {
        Json::Map(held) => {
            let objected = held
                .iter()
                .any(|(key, one)| key == "type" && one.text() == Some("object"));
            let named = held.iter().any(|(key, _)| key == "properties");
            let open = !held.iter().any(|(key, _)| key == "additionalProperties");
            let mut out: Vec<(String, Json)> = held
                .into_iter()
                .map(|(key, one)| match key.as_str() {
                    "items" => (key, closed(one)),
                    "properties" => match one {
                        Json::Map(inner) => (
                            key,
                            Json::Map(
                                inner
                                    .into_iter()
                                    .map(|(name, one)| (name, closed(one)))
                                    .collect(),
                            ),
                        ),
                        other => (key, other),
                    },
                    _ => (key, one),
                })
                .collect();
            if objected && named && open {
                out.push(("additionalProperties".to_owned(), Json::Bool(false)));
            }
            Json::Map(out)
        }
        other => other,
    }
}

fn plain(key: &str) -> bool {
    let mut letters = key.chars();
    letters
        .next()
        .is_some_and(|one| one.is_ascii_alphabetic() || one == '_')
        && letters.all(|one| one.is_ascii_alphanumeric() || one == '_')
}

#[must_use]
pub fn keyed(path: &str, key: &str) -> String {
    if plain(key) {
        format!("{path}.{key}")
    } else {
        format!("{path}[{}]", shown(&crate::wire::written(&text(key))))
    }
}

fn named(path: &str) -> String {
    match path.strip_prefix("$.") {
        Some(rest) => rest.to_owned(),
        None if path == "$" => "the arguments".to_owned(),
        None => path.trim_start_matches('$').to_owned(),
    }
}

fn number(schema: &Json, key: &str) -> Option<i64> {
    schema.get(key).and_then(Json::int)
}

struct Found {
    wrong: Wrong,
    path: String,
    limit: Option<Json>,
    message: String,
}

fn found(wrong: Wrong, path: &str, limit: Option<Json>, message: String) -> Found {
    Found {
        wrong,
        path: path.to_owned(),
        limit,
        message,
    }
}

fn near(asked: &str, known: &str) -> bool {
    let asked: Vec<char> = asked.to_lowercase().chars().collect();
    let known: Vec<char> = known.to_lowercase().chars().collect();
    if asked.len().abs_diff(known.len()) > 2 || asked.len() > 64 {
        return false;
    }
    let mut row: Vec<usize> = (0..=known.len()).collect();
    for (at, one) in asked.iter().enumerate() {
        let mut last = row.first().copied().unwrap_or(0);
        if let Some(first) = row.first_mut() {
            *first = at + 1;
        }
        for (column, other) in known.iter().enumerate() {
            let above = row.get(column + 1).copied().unwrap_or(0);
            let left = row.get(column).copied().unwrap_or(0);
            let next = (last + usize::from(one != other))
                .min(above + 1)
                .min(left + 1);
            last = above;
            if let Some(slot) = row.get_mut(column + 1) {
                *slot = next;
            }
        }
    }
    row.last().is_some_and(|one| *one <= 2)
}

fn kind_of(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "true or false",
        Json::Int(_) => "a whole number",
        Json::Text(_) => "text",
        Json::List(_) => "a list",
        Json::Map(_) => "an object",
    }
}

fn walk(schema: &Json, value: &Json, path: &str, check: Check) -> Result<Json, Found> {
    let kinds = schema.get("type").and_then(Json::list);
    if matches!(value, Json::Null)
        && kinds.is_some_and(|held| held.iter().any(|one| one.text() == Some("null")))
    {
        return Ok(value.clone());
    }
    let kind = schema
        .get("type")
        .and_then(Json::text)
        .or_else(|| {
            kinds.and_then(|held| {
                held.iter()
                    .filter_map(Json::text)
                    .find(|one| *one != "null")
            })
        })
        .unwrap_or("");
    let whole = check == Check::Whole;
    let typed = |wanted: &str| {
        found(
            Wrong::Type,
            path,
            None,
            format!(
                "{} is {}, which is {} and not {wanted}",
                named(path),
                shown_value(value),
                kind_of(value)
            ),
        )
    };
    match kind {
        "object" => {
            let Json::Map(held) = value else {
                if whole || path == "$" {
                    return Err(typed("an object"));
                }
                return Ok(value.clone());
            };
            let properties = match schema.get("properties") {
                Some(Json::Map(properties)) => properties.as_slice(),
                _ => &[],
            };
            let shut = schema.get("additionalProperties") == Some(&Json::Bool(false));
            if whole {
                let mut seen = std::collections::HashSet::with_capacity(held.len());
                for (key, _) in held {
                    if !seen.insert(key.as_str()) {
                        let here = keyed(path, key);
                        return Err(found(
                            Wrong::Duplicate,
                            &here,
                            None,
                            format!("{} is given more than once", named(&here)),
                        ));
                    }
                }
            }
            if shut {
                for (key, _) in held {
                    if properties.iter().any(|(name, _)| name == key) {
                        continue;
                    }
                    let here = keyed(path, key);
                    let known: Vec<&str> =
                        properties.iter().map(|(name, _)| name.as_str()).collect();
                    let guess = known.iter().find(|one| near(key, one));
                    let takes = if known.is_empty() {
                        "it takes no arguments".to_owned()
                    } else {
                        format!("it takes {}", known.join(", "))
                    };
                    return Err(found(
                        Wrong::Unknown,
                        &here,
                        None,
                        match guess {
                            Some(guess) => format!(
                                "{} is not an argument here; did you mean {guess}? {takes}",
                                named(&here)
                            ),
                            None => format!("{} is not an argument here; {takes}", named(&here)),
                        },
                    ));
                }
            }
            if whole {
                for required in schema.get("required").and_then(Json::list).unwrap_or(&[]) {
                    let Some(required) = required.text() else {
                        continue;
                    };
                    if !held.iter().any(|(key, _)| key == required) {
                        let here = keyed(path, required);
                        return Err(found(
                            Wrong::Missing,
                            &here,
                            None,
                            format!("{} is missing", named(&here)),
                        ));
                    }
                }
            }
            let mut out = Vec::new();
            for (name, inner) in properties {
                match held.iter().find(|(key, _)| key == name) {
                    Some((_, one)) => {
                        out.push((name.clone(), walk(inner, one, &keyed(path, name), check)?));
                    }
                    None => {
                        if whole && let Some(default) = inner.get("default") {
                            out.push((name.clone(), default.clone()));
                        }
                    }
                }
            }
            if !shut {
                for (key, one) in held {
                    if !properties.iter().any(|(name, _)| name == key) {
                        out.push((key.clone(), one.clone()));
                    }
                }
            }
            Ok(Json::Map(out))
        }
        "array" => {
            let Json::List(held) = value else {
                if whole {
                    return Err(typed("a list"));
                }
                return Ok(value.clone());
            };
            if whole {
                let many = i64::try_from(held.len()).unwrap_or(i64::MAX);
                let least = number(schema, "minItems").unwrap_or(0);
                let most = number(schema, "maxItems").unwrap_or(i64::MAX);
                let name = named(path);
                if least == most && many != least {
                    return Err(found(
                        Wrong::Count,
                        path,
                        Some(Json::Int(least)),
                        format!("{name} holds {many} items, where it takes exactly {least}"),
                    ));
                }
                if many > most {
                    return Err(found(
                        Wrong::Long,
                        path,
                        Some(Json::Int(most)),
                        format!("{name} holds {many} items, more than the {most} one call carries"),
                    ));
                }
                if many < least {
                    return Err(found(
                        Wrong::Short,
                        path,
                        Some(Json::Int(least)),
                        format!("{name} holds {many} items, fewer than the {least} it needs"),
                    ));
                }
            }
            match schema.get("items") {
                Some(items) => Ok(Json::List(
                    held.iter()
                        .enumerate()
                        .map(|(at, one)| walk(items, one, &format!("{path}[{at}]"), check))
                        .collect::<Result<_, _>>()?,
                )),
                None => Ok(value.clone()),
            }
        }
        _ if !whole => Ok(value.clone()),
        "integer" => {
            let least = number(schema, "minimum").unwrap_or(i64::MIN);
            let most = number(schema, "maximum").unwrap_or(i64::MAX);
            let wrong = |wrong: Wrong, limit: Option<Json>| {
                found(
                    wrong,
                    path,
                    limit,
                    format!(
                        "{} is {}, which is not a whole number from {least} to {most}",
                        named(path),
                        shown_value(value)
                    ),
                )
            };
            match value.int() {
                None => Err(wrong(Wrong::Type, None)),
                Some(one) if one < least => Err(wrong(Wrong::Range, Some(Json::Int(least)))),
                Some(one) if one > most => Err(wrong(Wrong::Range, Some(Json::Int(most)))),
                Some(_) => Ok(value.clone()),
            }
        }
        "string" => {
            let allowed = schema.get("enum").and_then(Json::list);
            let choices = || {
                allowed
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Json::text)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            match value.text() {
                None if allowed.is_some() => Err(found(
                    Wrong::Type,
                    path,
                    allowed.map(|one| Json::List(one.to_vec())),
                    format!(
                        "{} is {}, which is not one of {}",
                        named(path),
                        shown_value(value),
                        choices()
                    ),
                )),
                None => Err(typed("text")),
                Some(one) => match allowed {
                    Some(allowed) if !allowed.iter().any(|choice| choice.text() == Some(one)) => {
                        Err(found(
                            Wrong::Choice,
                            path,
                            Some(Json::List(allowed.to_vec())),
                            format!(
                                "{} is {}, which is not one of {}",
                                named(path),
                                shown_value(value),
                                choices()
                            ),
                        ))
                    }
                    _ => Ok(value.clone()),
                },
            }
        }
        "boolean" => match value {
            Json::Bool(_) => Ok(value.clone()),
            _ => Err(typed("true or false")),
        },
        _ => Ok(value.clone()),
    }
}

#[must_use]
pub fn code_of(tool: &str, path: &str, wrong: Wrong) -> Code {
    match (tool, path, wrong) {
        ("grit_world_create", "$.rule", Wrong::Missing | Wrong::Type | Wrong::Choice) => {
            Code::UnknownRule
        }
        ("grit_world_step", "$.frames", Wrong::Range) => Code::Frames,
        ("grit_world_slice", "$.axis", Wrong::Type | Wrong::Choice) => Code::Axis,
        (_, _, Wrong::Unknown) => Code::UnknownKey,
        (_, _, Wrong::Duplicate) => Code::DuplicateKey,
        (_, _, Wrong::Long) => Code::TooMany,
        _ => Code::Malformed,
    }
}

pub fn checked(tool: &str, schema: &Json, arguments: &Json, check: Check) -> Result<Json, Refusal> {
    walk(schema, arguments, "$", check).map_err(|held| {
        let refusal = Refusal::new(
            code_of(tool, &held.path, held.wrong),
            &held.path,
            held.message,
        );
        match held.limit {
            Some(limit) => refusal.limited(limit),
            None => refusal,
        }
    })
}
