use std::collections::HashMap;
use std::rc::Rc;

use super::dag::{AT, Dag, FIRST_PARAM, FRAME};
use super::parse::{Path, Program, clipped, described, limited, refused};
use super::trace::Tracer;
use super::{Code, Refusal, limits};
use crate::ops::{Cpu, Ops};
use crate::rule3::{CORNERS, noise_by};
use crate::wire::Json;

#[derive(Clone, Debug)]
enum Value {
    Int(u32),
    Word(u32),
    Flag(u32),
    List(Rc<[Value]>),
}

#[derive(Clone, Copy, Debug)]
enum Operand {
    Int(u32),
    Word(u32),
}

#[derive(Clone, Copy, Debug)]
enum Truth {
    Known(bool),
    Flag(u32),
}

fn kind(value: &Value) -> String {
    match value {
        Value::Int(held) => format!("the int {held}"),
        Value::Word(_) => String::from("a word"),
        Value::Flag(_) => String::from("a flag"),
        Value::List(items) => format!("a vec of {}", items.len()),
    }
}

pub struct Expanded {
    pub dag: Dag,
    pub work: u64,
    pub depth: usize,
}

struct Expander<'c, 'p> {
    program: &'c Program<'p>,
    tracer: Tracer,
    globals: HashMap<&'p str, Value>,
    locals: Vec<(&'p str, Value)>,
    places: HashMap<&'p str, Vec<usize>>,
    base: usize,
    work: u64,
    deepest: usize,
}

fn int(value: &Value, path: &Path<'_>) -> Result<u32, Refusal> {
    match value {
        Value::Int(held) => Ok(*held),
        other => Err(refused(
            Code::Type,
            path,
            "an int, known when the law compiles",
            kind(other),
        )),
    }
}

fn operand(value: &Value, path: &Path<'_>) -> Result<Operand, Refusal> {
    match value {
        Value::Int(held) => Ok(Operand::Int(*held)),
        Value::Word(id) => Ok(Operand::Word(*id)),
        other => Err(refused(Code::Type, path, "an int or a word", kind(other))),
    }
}

fn truth(value: &Value, path: &Path<'_>) -> Result<Truth, Refusal> {
    match value {
        Value::Int(held) => Ok(Truth::Known(*held != 0)),
        Value::Flag(id) => Ok(Truth::Flag(*id)),
        other => Err(refused(Code::Type, path, "a flag or an int", kind(other))),
    }
}

fn list(value: &Value, path: &Path<'_>) -> Result<Rc<[Value]>, Refusal> {
    match value {
        Value::List(items) => Ok(Rc::clone(items)),
        other => Err(refused(Code::Type, path, "a vec", kind(other))),
    }
}

fn arg(args: &[Json], at: usize) -> &Json {
    args.get(at).unwrap_or(&Json::Null)
}

fn binder<'p>(json: &'p Json, path: &Path<'_>) -> Result<&'p str, Refusal> {
    match json {
        Json::Text(name) => Ok(name),
        other => Err(refused(
            Code::Shape,
            path,
            "a name to bind",
            described(other),
        )),
    }
}

fn item(items: &[Value], index: u32, path: &Path<'_>) -> Result<Value, Refusal> {
    match usize::try_from(index).ok().and_then(|at| items.get(at)) {
        Some(value) => Ok(value.clone()),
        None => Err(limited(
            Code::Index,
            path,
            format!("an index below {}", items.len()),
            index.to_string(),
            items.len() as u64,
        )),
    }
}

fn too_deep(path: &Path<'_>) -> Refusal {
    limited(
        Code::Depth,
        path,
        format!(
            "expressions nested at most {} deep, defs included",
            limits::DEPTH
        ),
        "deeper nesting",
        limits::DEPTH as u64,
    )
}

fn too_large(held: i64, path: &Path<'_>) -> Refusal {
    refused(
        Code::Literal,
        path,
        "a whole number from 0 to 4294967295",
        held.to_string(),
    )
}

fn not_an_expression(json: &Json, path: &Path<'_>) -> Refusal {
    refused(
        Code::Shape,
        path,
        "a number, a name or an operation list",
        described(json),
    )
}

fn unnamed(path: &Path<'_>) -> Refusal {
    refused(
        Code::Shape,
        path,
        "an operation list [op, args...]",
        "a list without an operation name",
    )
}

fn not_a_binding(json: &Json, path: &Path<'_>) -> Refusal {
    refused(
        Code::Shape,
        path,
        "a binding [name, value]",
        described(json),
    )
}

fn folded(op: &str, left: u32, right: u32) -> u32 {
    let cpu = &mut Cpu;
    match op {
        "add" => cpu.add(left, right),
        "sub" => cpu.sub(left, right),
        "mul" => cpu.mul(left, right),
        "div" => cpu.div(left, right),
        "and" => cpu.and(left, right),
        "or" => cpu.or(left, right),
        "xor" => cpu.xor(left, right),
        "dot_s" => cpu.dot_signed(left, right),
        "dot_u" => cpu.dot_unsigned(left, right),
        "shl" => cpu.shift_left(left, right),
        "shr" => cpu.shift_right(left, right),
        "eq" => cpu.equal(left, right).word(),
        "ne" => cpu.not_equal(left, right).word(),
        "lt" => cpu.less(left, right).word(),
        "le" => cpu.less_equal(left, right).word(),
        "gt" => cpu.greater(left, right).word(),
        _ => cpu.greater_equal(left, right).word(),
    }
}

fn emitted(tracer: &mut Tracer, op: &str, left: u32, right: u32) -> Value {
    match op {
        "add" => Value::Word(tracer.add(left, right)),
        "sub" => Value::Word(tracer.sub(left, right)),
        "mul" => Value::Word(tracer.mul(left, right)),
        "div" => Value::Word(tracer.div(left, right)),
        "and" => Value::Word(tracer.and(left, right)),
        "or" => Value::Word(tracer.or(left, right)),
        "xor" => Value::Word(tracer.xor(left, right)),
        "dot_s" => Value::Word(tracer.dot_signed(left, right)),
        "dot_u" => Value::Word(tracer.dot_unsigned(left, right)),
        "eq" => Value::Flag(tracer.equal(left, right).word()),
        "ne" => Value::Flag(tracer.not_equal(left, right).word()),
        "lt" => Value::Flag(tracer.less(left, right).word()),
        "le" => Value::Flag(tracer.less_equal(left, right).word()),
        "gt" => Value::Flag(tracer.greater(left, right).word()),
        _ => Value::Flag(tracer.greater_equal(left, right).word()),
    }
}

impl<'p> Expander<'_, 'p> {
    fn spend(&mut self, visits: usize, path: &Path<'_>) -> Result<(), Refusal> {
        self.work = self
            .work
            .saturating_add(u64::try_from(visits).unwrap_or(u64::MAX));
        if self.work > limits::WORK {
            return Err(limited(
                Code::Work,
                path,
                format!("at most {} steps of unrolling", limits::WORK),
                format!("more than {}", limits::WORK),
                limits::WORK,
            ));
        }
        Ok(())
    }

    fn grown(&self, path: &Path<'_>) -> Result<(), Refusal> {
        if self.tracer.nodes.len() > limits::NODES {
            return Err(limited(
                Code::Nodes,
                path,
                format!("at most {} nodes after unrolling", limits::NODES),
                format!("{} nodes", self.tracer.nodes.len()),
                limits::NODES as u64,
            ));
        }
        Ok(())
    }

    fn id(&mut self, operand: Operand, path: &Path<'_>) -> Result<u32, Refusal> {
        match operand {
            Operand::Word(id) => Ok(id),
            Operand::Int(held) => {
                let id = self.tracer.constant(held);
                self.grown(path)?;
                Ok(id)
            }
        }
    }

    fn truth_id(&mut self, truth: Truth, path: &Path<'_>) -> Result<u32, Refusal> {
        match truth {
            Truth::Flag(id) => Ok(id),
            Truth::Known(holds) => self.id(Operand::Int(u32::from(holds)), path),
        }
    }

    fn bind(&mut self, name: &'p str, value: Value) {
        self.places.entry(name).or_default().push(self.locals.len());
        self.locals.push((name, value));
    }

    fn unbind(&mut self, mark: usize) {
        while self.locals.len() > mark {
            if let Some((name, _)) = self.locals.pop()
                && let Some(places) = self.places.get_mut(name)
            {
                places.pop();
            }
        }
    }

    fn lookup(&self, name: &str, path: &Path<'_>) -> Result<Value, Refusal> {
        let local = self
            .places
            .get(name)
            .and_then(|places| places.last())
            .filter(|&&place| place >= self.base)
            .and_then(|&place| self.locals.get(place));
        match local
            .map(|(_, value)| value)
            .or_else(|| self.globals.get(name))
        {
            Some(value) => Ok(value.clone()),
            None => Err(refused(
                Code::UnknownName,
                path,
                "a const, an input or a bound name",
                format!("the name {}", clipped(name)),
            )),
        }
    }

    fn eval(&mut self, json: &'p Json, path: &Path<'_>, depth: usize) -> Result<Value, Refusal> {
        self.deepest = self.deepest.max(depth);
        if depth > limits::DEPTH {
            return Err(too_deep(path));
        }
        self.spend(1, path)?;
        match json {
            Json::Int(held) => u32::try_from(*held)
                .map(Value::Int)
                .map_err(|_| too_large(*held, path)),
            Json::Text(name) => self.lookup(name, path),
            Json::List(items) => self.operation(items, path, depth + 1),
            other => Err(not_an_expression(other, path)),
        }
    }

    fn bound(
        &mut self,
        names: &[(&'p str, Value)],
        body: &'p Json,
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let mark = self.locals.len();
        for (name, value) in names {
            self.bind(name, value.clone());
        }
        let result = self.eval(body, path, depth);
        self.unbind(mark);
        result
    }

    fn operation(
        &mut self,
        items: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let Some((Json::Text(op), args)) = items.split_first() else {
            return Err(unnamed(path));
        };
        match op.as_str() {
            "let" => self.bindings(args, path, depth),
            "if" => self.chosen(args, path, depth),
            "vec" => self.listed(args, path, depth),
            "range" => self.ranged(args, path, depth),
            "at" => self.indexed(args, path, depth),
            "put" => self.replaced(args, path, depth),
            "map" | "filter" => self.mapped(op == "map", args, path, depth),
            "fold" => self.reduced(args, path, depth),
            "rev" => self.reversed(args, path, depth),
            "concat" => self.joined(args, path, depth),
            "param" => self.param(args, path, depth),
            "noise" => self.noise(path),
            _ => match self.program.named.get(op.as_str()) {
                Some(&index) => self.call(index, args, path, depth),
                None => self.builtin(op, args, path, depth),
            },
        }
    }

    fn bindings(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let bindings_path = path.index(1);
        let Some(Json::List(bindings)) = args.first() else {
            return Err(refused(
                Code::Shape,
                &bindings_path,
                "a list of [name, value] bindings",
                described(arg(args, 0)),
            ));
        };
        let mark = self.locals.len();
        let mut result = Ok(());
        for (index, binding) in bindings.iter().enumerate() {
            let here = bindings_path.index(index);
            let Json::List(pair) = binding else {
                result = Err(not_a_binding(binding, &here));
                break;
            };
            let [Json::Text(name), value] = pair.as_slice() else {
                result = Err(not_a_binding(binding, &here));
                break;
            };
            match self.eval(value, &here.index(1), depth) {
                Ok(made) => self.bind(name.as_str(), made),
                Err(why) => {
                    result = Err(why);
                    break;
                }
            }
        }
        let value = result.and_then(|()| self.eval(arg(args, 1), &path.index(2), depth));
        self.unbind(mark);
        value
    }

    fn chosen(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let condition = self.int_at(args, 0, path, depth)?;
        let chosen = if condition != 0 { 1 } else { 2 };
        self.eval(arg(args, chosen), &path.index(chosen + 1), depth)
    }

    fn listed(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let mut made = Vec::with_capacity(args.len());
        for (index, one) in args.iter().enumerate() {
            made.push(self.eval(one, &path.index(index + 1), depth)?);
        }
        self.spend(made.len(), path)?;
        Ok(Value::List(made.into()))
    }

    fn ranged(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let many = self.int_at(args, 0, path, depth)?;
        self.spend(usize::try_from(many).unwrap_or(usize::MAX), path)?;
        Ok(Value::List((0..many).map(Value::Int).collect()))
    }

    fn indexed(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let items = self.list_at(args, 0, path, depth)?;
        let index = self.int_at(args, 1, path, depth)?;
        item(&items, index, &path.index(2))
    }

    fn replaced(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let items = self.list_at(args, 0, path, depth)?;
        let index = self.int_at(args, 1, path, depth)?;
        let value = self.eval(arg(args, 2), &path.index(3), depth)?;
        item(&items, index, &path.index(2))?;
        self.spend(items.len(), path)?;
        let mut made = items.to_vec();
        if let Some(slot) = usize::try_from(index).ok().and_then(|at| made.get_mut(at)) {
            *slot = value;
        }
        Ok(Value::List(made.into()))
    }

    fn mapped(
        &mut self,
        mapping: bool,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let name = binder(arg(args, 0), &path.index(1))?;
        let items = self.list_at(args, 1, path, depth)?;
        self.spend(items.len(), path)?;
        let mut made = Vec::with_capacity(items.len());
        for one in items.iter() {
            let value = self.bound(&[(name, one.clone())], arg(args, 2), &path.index(3), depth)?;
            if mapping {
                made.push(value);
            } else if int(&value, &path.index(3))? != 0 {
                made.push(one.clone());
            }
        }
        Ok(Value::List(made.into()))
    }

    fn reduced(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let acc = binder(arg(args, 0), &path.index(1))?;
        let name = binder(arg(args, 2), &path.index(3))?;
        let mut value = self.eval(arg(args, 1), &path.index(2), depth)?;
        let items = self.list_at(args, 3, path, depth)?;
        for one in items.iter() {
            value = self.bound(
                &[(acc, value), (name, one.clone())],
                arg(args, 4),
                &path.index(5),
                depth,
            )?;
        }
        Ok(value)
    }

    fn reversed(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let items = self.list_at(args, 0, path, depth)?;
        self.spend(items.len(), path)?;
        Ok(Value::List(items.iter().rev().cloned().collect()))
    }

    fn joined(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let mut parts = Vec::with_capacity(args.len());
        for index in 0..args.len() {
            parts.push(self.list_at(args, index, path, depth)?);
        }
        self.spend(parts.iter().map(|part| part.len()).sum(), path)?;
        Ok(Value::List(
            parts.iter().flat_map(|part| part.iter().cloned()).collect(),
        ))
    }

    fn param(&mut self, args: &'p [Json], path: &Path<'_>, depth: usize) -> Result<Value, Refusal> {
        let which = self.int_at(args, 0, path, depth)?;
        match usize::try_from(which) {
            Ok(index) if index < self.program.params => {
                Ok(Value::Word(FIRST_PARAM.saturating_add(which)))
            }
            _ => Err(limited(
                Code::Index,
                &path.index(1),
                format!("a param index below {}", self.program.params),
                which.to_string(),
                self.program.params as u64,
            )),
        }
    }

    fn noise(&mut self, path: &Path<'_>) -> Result<Value, Refusal> {
        let made = noise_by(&mut self.tracer, [AT, AT + 1, AT + 2], FRAME);
        self.grown(path)?;
        Ok(Value::Word(made))
    }

    fn int_at(
        &mut self,
        args: &'p [Json],
        at: usize,
        path: &Path<'_>,
        depth: usize,
    ) -> Result<u32, Refusal> {
        let here = path.index(at + 1);
        let value = self.eval(arg(args, at), &here, depth)?;
        int(&value, &here)
    }

    fn list_at(
        &mut self,
        args: &'p [Json],
        at: usize,
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Rc<[Value]>, Refusal> {
        let here = path.index(at + 1);
        let value = self.eval(arg(args, at), &here, depth)?;
        list(&value, &here)
    }

    fn call(
        &mut self,
        index: usize,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let program = self.program;
        let Some(def) = program.defs.get(index) else {
            return Err(refused(Code::UnknownOp, path, "a def", "no such def"));
        };
        if def.args.len() != args.len() {
            return Err(refused(
                Code::Arity,
                path,
                format!("{} with {} arguments", def.name, def.args.len()),
                format!("{} arguments", args.len()),
            ));
        }
        let mut values = Vec::with_capacity(args.len());
        for (at, (one, name)) in args.iter().zip(&def.args).enumerate() {
            values.push((*name, self.eval(one, &path.index(at + 1), depth)?));
        }
        let saved = self.base;
        self.base = self.locals.len();
        let root = Path::root();
        let defs = root.key("defs");
        let named = defs.key(def.name);
        let body = named.index(1);
        let result = self.bound(&values, def.body, &body, depth);
        self.base = saved;
        result
    }

    fn builtin(
        &mut self,
        op: &str,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let mut values = Vec::with_capacity(args.len());
        for (at, one) in args.iter().enumerate() {
            values.push(self.eval(one, &path.index(at + 1), depth)?);
        }
        self.applied(op, &values, path)
    }

    fn applied(&mut self, op: &str, values: &[Value], path: &Path<'_>) -> Result<Value, Refusal> {
        let at = |at: usize| path.index(at + 1);
        let value = |at: usize| values.get(at).cloned().unwrap_or(Value::Int(0));
        match op {
            "add" | "sub" | "mul" | "div" | "and" | "or" | "xor" | "dot_s" | "dot_u" | "eq"
            | "ne" | "lt" | "le" | "gt" | "ge" => {
                let left = operand(&value(0), &at(0))?;
                let right = operand(&value(1), &at(1))?;
                if let (Operand::Int(left), Operand::Int(right)) = (left, right) {
                    return Ok(Value::Int(folded(op, left, right)));
                }
                let left = self.id(left, path)?;
                let right = self.id(right, path)?;
                let made = emitted(&mut self.tracer, op, left, right);
                self.grown(path)?;
                Ok(made)
            }
            "shl" | "shr" => {
                let shifted = operand(&value(0), &at(0))?;
                let by = int(&value(1), &at(1))?;
                match shifted {
                    Operand::Int(held) => Ok(Value::Int(folded(op, held, by))),
                    Operand::Word(id) => {
                        let made = if op == "shl" {
                            self.tracer.shift_left(id, by)
                        } else {
                            self.tracer.shift_right(id, by)
                        };
                        self.grown(path)?;
                        Ok(Value::Word(made))
                    }
                }
            }
            "not" => match operand(&value(0), &at(0))? {
                Operand::Int(held) => Ok(Value::Int(Cpu.not(held))),
                Operand::Word(id) => {
                    let made = self.tracer.not(id);
                    self.grown(path)?;
                    Ok(Value::Word(made))
                }
            },
            "both" | "either" => {
                let left = truth(&value(0), &at(0))?;
                let right = truth(&value(1), &at(1))?;
                if let (Truth::Known(left), Truth::Known(right)) = (left, right) {
                    let holds = if op == "both" {
                        left && right
                    } else {
                        left || right
                    };
                    return Ok(Value::Int(u32::from(holds)));
                }
                let left = self.truth_id(left, path)?;
                let right = self.truth_id(right, path)?;
                let made = if op == "both" {
                    self.tracer.and(left, right)
                } else {
                    self.tracer.or(left, right)
                };
                self.grown(path)?;
                Ok(Value::Flag(made))
            }
            "denied" => match truth(&value(0), &at(0))? {
                Truth::Known(holds) => Ok(Value::Int(u32::from(!holds))),
                Truth::Flag(id) => {
                    let one = self.tracer.constant(1);
                    let made = self.tracer.xor(id, one);
                    self.grown(path)?;
                    Ok(Value::Flag(made))
                }
            },
            "word" => match truth(&value(0), &at(0))? {
                Truth::Known(holds) => Ok(Value::Int(u32::from(holds))),
                Truth::Flag(id) => Ok(Value::Word(id)),
            },
            "count" => {
                let items = list(&value(0), &at(0))?;
                let mut truths = Vec::with_capacity(items.len());
                for item in items.iter() {
                    truths.push(truth(item, &at(0))?);
                }
                if truths.iter().all(|one| matches!(one, Truth::Known(_))) {
                    let total = truths
                        .iter()
                        .filter(|one| matches!(one, Truth::Known(true)))
                        .count();
                    return Ok(Value::Int(u32::try_from(total).unwrap_or(u32::MAX)));
                }
                let mut total = self.tracer.constant(0);
                for one in truths {
                    let id = self.truth_id(one, path)?;
                    total = self.tracer.add(total, id);
                    self.grown(path)?;
                }
                Ok(Value::Word(total))
            }
            "sel" => match truth(&value(0), &at(0))? {
                Truth::Known(holds) => Ok(value(if holds { 1 } else { 2 })),
                Truth::Flag(flag) => {
                    let then = operand(&value(1), &at(1))?;
                    let otherwise = operand(&value(2), &at(2))?;
                    let then = self.id(then, path)?;
                    let otherwise = self.id(otherwise, path)?;
                    let made = self
                        .tracer
                        .select(crate::ops::Flag::raised(flag), then, otherwise);
                    self.grown(path)?;
                    Ok(Value::Word(made))
                }
            },
            _ => Err(refused(
                Code::UnknownOp,
                &path.index(0),
                "an operation of the language or a def",
                format!("the name {}", clipped(op)),
            )),
        }
    }
}

fn shaped(path: &Path<'_>, found: String) -> Refusal {
    refused(
        Code::Result,
        path,
        "[\"vec\", out, counts]: 8 out words, and as many count words as tally.counts",
        found,
    )
}

fn shaped_planes(path: &Path<'_>, planes: u32, found: String) -> Refusal {
    refused(
        Code::Result,
        path,
        format!(
            "[\"vec\", out, counts]: out 8 vecs of {planes} words, and as many count words as tally.counts"
        ),
        found,
    )
}

fn first_plane(program: &Program<'_>) -> u32 {
    FIRST_PARAM.saturating_add(u32::try_from(program.params).unwrap_or(u32::MAX))
}

fn cells_of(program: &Program<'_>) -> Value {
    if program.channels <= 1 {
        return Value::List((0..8).map(Value::Word).collect());
    }
    let first = first_plane(program);
    let corners = u32::try_from(CORNERS).unwrap_or(8);
    Value::List(
        (0..corners)
            .map(|corner| {
                let words: Vec<Value> = (0..program.channels)
                    .map(|plane| match plane {
                        0 => Value::Word(corner),
                        _ => Value::Word(first + (plane - 1) * corners + corner),
                    })
                    .collect();
                Value::List(words.into())
            })
            .collect(),
    )
}

pub fn expand(program: &Program<'_>) -> Result<Expanded, Refusal> {
    let planes = program.channels.max(1);
    let extra = usize::try_from(planes - 1).unwrap_or(0) * CORNERS;
    let tracer = Tracer::with_inputs(program.params + extra);
    let mut globals: HashMap<&str, Value> = HashMap::new();
    globals.insert("X", Value::Int(1));
    globals.insert("Z", Value::Int(2));
    globals.insert("Y", Value::Int(4));
    globals.insert("cells", cells_of(program));
    globals.insert("frame", Value::Word(FRAME));
    globals.insert(
        "at",
        Value::List([AT, AT + 1, AT + 2].into_iter().map(Value::Word).collect()),
    );
    for &(name, value) in &program.consts {
        globals.insert(name, Value::Int(value));
    }
    let mut expander = Expander {
        program,
        tracer,
        globals,
        locals: Vec::new(),
        places: HashMap::new(),
        base: 0,
        work: 0,
        deepest: 0,
    };
    let root = Path::root();
    let step = root.key("step");
    let result = expander.eval(program.step, &step, 0)?;
    let Value::List(halves) = &result else {
        return Err(shaped(&step, kind(&result)));
    };
    let [out, counts] = &**halves else {
        return Err(shaped(&step, kind(&result)));
    };
    let (Value::List(out), Value::List(counts)) = (out, counts) else {
        return Err(shaped(&step, format!("[{}, {}]", kind(out), kind(counts))));
    };
    if out.len() != CORNERS || counts.len() != program.tally.counts {
        let found = format!("{} out values and {} counts", out.len(), counts.len());
        return Err(if planes > 1 {
            shaped_planes(&step, planes, found)
        } else {
            shaped(&step, found)
        });
    }
    let mut ids = [0_u32; CORNERS];
    let mut upper = vec![[0_u32; CORNERS]; usize::try_from(planes - 1).unwrap_or(0)];
    if planes > 1 {
        for (index, one) in out.iter().enumerate() {
            let Value::List(words) = one else {
                return Err(shaped_planes(
                    &step,
                    planes,
                    format!("{} at out[{index}]", kind(one)),
                ));
            };
            if words.len() != usize::try_from(planes).unwrap_or(0) {
                return Err(shaped_planes(
                    &step,
                    planes,
                    format!("{} at out[{index}]", kind(one)),
                ));
            }
            for (plane, word) in words.iter().enumerate() {
                let made = operand(word, &step).map_err(|_| {
                    shaped_planes(
                        &step,
                        planes,
                        format!("{} at out[{index}][{plane}]", kind(word)),
                    )
                })?;
                let id = expander.id(made, &step)?;
                let slot = match plane {
                    0 => ids.get_mut(index),
                    _ => upper
                        .get_mut(plane - 1)
                        .and_then(|words| words.get_mut(index)),
                };
                if let Some(slot) = slot {
                    *slot = id;
                }
            }
        }
    } else {
        for (slot, (index, one)) in ids.iter_mut().zip(out.iter().enumerate()) {
            let made = operand(one, &step)
                .map_err(|_| shaped(&step, format!("{} at out[{index}]", kind(one))))?;
            *slot = expander.id(made, &step)?;
        }
    }
    let mut tallied = Vec::with_capacity(counts.len());
    for (index, one) in counts.iter().enumerate() {
        let made = operand(one, &step)
            .map_err(|_| shaped(&step, format!("{} at counts[{index}]", kind(one))))?;
        tallied.push(expander.id(made, &step)?);
    }
    Ok(Expanded {
        dag: Dag::new(
            expander.tracer.nodes,
            ids,
            tallied,
            program.params,
            program.tally,
        )
        .with_planes(upper),
        work: expander.work,
        depth: expander.deepest,
    })
}
