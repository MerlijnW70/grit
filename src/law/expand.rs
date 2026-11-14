use std::collections::HashMap;

use super::dag::{AT, Dag, FIRST_PARAM, FRAME};
use super::parse::{Path, Program, Step, clipped, described, limited, refused, shown};
use super::trace::Tracer;
use super::value::{Items, Value};
use super::{Code, Refusal, limits};
use crate::halo::{self, BOX, Halo};
use crate::ops::{Cpu, Ops};
use crate::rule3::{CORNERS, noise_by};
use crate::wire::Json;

const KNOWN_INT: &str = "an int, known when the law compiles";

const KNOWN_CHOICE: &str = "an int or a flag, known when the law compiles";

const LIVE: &str = "known only when the law runs";

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
        Value::Truth(_) | Value::Flag(_) => String::from("a flag"),
        Value::List(items) => format!("a vec of {}", items.len()),
    }
}

fn live(value: &Value) -> String {
    format!("{}, {LIVE}", kind(value))
}

pub struct Expanded {
    pub dag: Dag,
    pub work: u64,
    pub depth: usize,
    pub sites: Vec<usize>,
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
    site: usize,
    sites: Vec<usize>,
    halo: Halo,
    halo_inputs: HashMap<u32, u32>,
    looks: Vec<(u32, String)>,
}

fn address(json: &Json) -> usize {
    std::ptr::from_ref(json).addr()
}

fn int(value: &Value, path: &Path<'_>) -> Result<u32, Refusal> {
    match value {
        Value::Int(held) => Ok(*held),
        Value::Word(_) => Err(refused(Code::Type, path, KNOWN_INT, live(value))),
        other => Err(refused(Code::Type, path, KNOWN_INT, kind(other))),
    }
}

fn chooser(value: &Value, path: &Path<'_>) -> Result<bool, Refusal> {
    match value {
        Value::Int(held) => Ok(*held != 0),
        Value::Truth(holds) => Ok(*holds),
        Value::Word(_) | Value::Flag(_) => {
            Err(refused(Code::Type, path, KNOWN_CHOICE, live(value)))
        }
        Value::List(_) => Err(refused(Code::Type, path, KNOWN_CHOICE, kind(value))),
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
        Value::Truth(holds) => Ok(Truth::Known(*holds)),
        Value::Flag(id) => Ok(Truth::Flag(*id)),
        other => Err(refused(
            Code::Type,
            path,
            "a flag, made by a compare, both, either or denied",
            kind(other),
        )),
    }
}

const fn spoken(operand: Operand) -> Value {
    match operand {
        Operand::Int(held) => Value::Int(held),
        Operand::Word(id) => Value::Word(id),
    }
}

fn list(value: &Value, path: &Path<'_>) -> Result<Items, Refusal> {
    match value {
        Value::List(items) => Ok(items.clone()),
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

fn item(items: &Items, index: u32, path: &Path<'_>) -> Result<Value, Refusal> {
    match usize::try_from(index)
        .ok()
        .and_then(|at| items.items().get(at))
    {
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

fn built(items: Vec<Value>, path: &Path<'_>) -> Result<Value, Refusal> {
    let made = Items::new(items);
    if made.depth() > limits::VALUES {
        return Err(limited(
            Code::ValueDepth,
            path,
            format!("a value of vecs nested at most {} deep", limits::VALUES),
            format!("vecs nested {} deep", made.depth()),
            limits::VALUES as u64,
        ));
    }
    Ok(Value::List(made))
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

    fn grown(&mut self, path: &Path<'_>) -> Result<(), Refusal> {
        let made = self.tracer.nodes.len();
        if self.sites.len() < made {
            self.sites.resize(made, self.site);
        }
        if made > limits::NODES {
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
            Json::List(items) => {
                let outer = std::mem::replace(&mut self.site, address(json));
                let made = self.operation(items, path, depth + 1);
                self.site = outer;
                made
            }
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
            "look" => self.looked(args, path, depth),
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
        let here = path.index(1);
        let condition = self.eval(arg(args, 0), &here, depth)?;
        let chosen = if chooser(&condition, &here)? { 1 } else { 2 };
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
        built(made, path)
    }

    fn ranged(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let many = self.int_at(args, 0, path, depth)?;
        self.spend(usize::try_from(many).unwrap_or(usize::MAX), path)?;
        built((0..many).map(Value::Int).collect(), path)
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
        let mut made = items.items().to_vec();
        if let Some(slot) = usize::try_from(index).ok().and_then(|at| made.get_mut(at)) {
            *slot = value;
        }
        built(made, path)
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
        for one in items.items() {
            let value = self.bound(&[(name, one.clone())], arg(args, 2), &path.index(3), depth)?;
            if mapping {
                made.push(value);
            } else if chooser(&value, &path.index(3))? {
                made.push(one.clone());
            }
        }
        built(made, path)
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
        for one in items.items() {
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
        built(items.items().iter().rev().cloned().collect(), path)
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
        built(
            parts
                .iter()
                .flat_map(|part| part.items().iter().cloned())
                .collect(),
            path,
        )
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

    fn known(&mut self, json: &'p Json, path: &Path<'_>, depth: usize) -> Result<u32, Refusal> {
        let value = self.eval(json, path, depth)?;
        match value {
            Value::Int(held) => Ok(held),
            Value::Word(_) | Value::Flag(_) => {
                Err(refused(Code::HaloConstant, path, KNOWN_INT, live(&value)))
            }
            other => Err(refused(Code::Type, path, KNOWN_INT, kind(&other))),
        }
    }

    fn step_of(&mut self, json: &'p Json, path: &Path<'_>, depth: usize) -> Result<i32, Refusal> {
        let step = match json {
            Json::Int(held) => *held,
            other => i64::from(self.known(other, path, depth)?.cast_signed()),
        };
        match i32::try_from(step) {
            Ok(step) if step.abs() <= halo::RADIUS => Ok(step),
            _ => Err(limited(
                Code::HaloRadius,
                path,
                "an offset of -1, 0 or 1: a look reaches only the cells next to the corner",
                step.to_string(),
                u64::from(halo::RADIUS.unsigned_abs()),
            )),
        }
    }

    fn looked(
        &mut self,
        args: &'p [Json],
        path: &Path<'_>,
        depth: usize,
    ) -> Result<Value, Refusal> {
        let channels = self.program.channels.max(1);
        let plane_path = path.index(1);
        let plane = self.known(arg(args, 0), &plane_path, depth)?;
        if plane >= channels {
            return Err(limited(
                Code::HaloPlane,
                &plane_path,
                format!("a plane below the law's {channels} channels"),
                plane.to_string(),
                u64::from(channels),
            ));
        }
        let mut offset = [0_i32; 3];
        for (axis, slot) in offset.iter_mut().enumerate() {
            *slot = self.step_of(arg(args, axis + 1), &path.index(axis + 2), depth)?;
        }
        let corner_path = path.index(5);
        let corner = self.known(arg(args, 4), &corner_path, depth)?;
        let corners = u32::try_from(CORNERS).unwrap_or(8);
        let Some(cell) = usize::try_from(corner)
            .ok()
            .filter(|&corner| corner < CORNERS)
            .and_then(|corner| halo::looked(corner, offset))
        else {
            return Err(limited(
                Code::Index,
                &corner_path,
                format!("a corner below {corners}"),
                corner.to_string(),
                u64::from(corners),
            ));
        };
        if let Some(own) = halo::own(cell) {
            let own = u32::try_from(own).unwrap_or(0);
            return Ok(Value::Word(match plane {
                0 => own,
                _ => first_plane(self.program) + (plane - 1) * corners + own,
            }));
        }
        if !self.looks.iter().any(|(seen, _)| *seen == plane) {
            self.looks.push((plane, path.shown()));
        }
        let first = first_plane(self.program) + (channels - 1) * corners;
        let box_cells = u32::try_from(BOX).unwrap_or(u32::MAX);
        let which = first + plane * box_cells + u32::try_from(cell).unwrap_or(0);
        if let Some(&id) = self.halo_inputs.get(&which) {
            return Ok(Value::Word(id));
        }
        let id = self.tracer.input(which);
        self.grown(path)?;
        self.halo_inputs.insert(which, id);
        self.halo = self
            .halo
            .with(usize::try_from(plane).unwrap_or(usize::MAX), cell);
        Ok(Value::Word(id))
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
    ) -> Result<Items, Refusal> {
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
                let compare = matches!(op, "eq" | "ne" | "lt" | "le" | "gt" | "ge");
                if let (Operand::Int(left), Operand::Int(right)) = (left, right) {
                    let held = folded(op, left, right);
                    return Ok(if compare {
                        Value::Truth(held != 0)
                    } else {
                        Value::Int(held)
                    });
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
                    return Ok(Value::Truth(holds));
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
                Truth::Known(holds) => Ok(Value::Truth(!holds)),
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
                self.spend(items.len(), path)?;
                let mut truths = Vec::with_capacity(items.len());
                for item in items.items() {
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
            "sel" => {
                let chosen = truth(&value(0), &at(0))?;
                let then = operand(&value(1), &at(1))?;
                let otherwise = operand(&value(2), &at(2))?;
                match chosen {
                    Truth::Known(holds) => Ok(spoken(if holds { then } else { otherwise })),
                    Truth::Flag(flag) => {
                        let then = self.id(then, path)?;
                        let otherwise = self.id(otherwise, path)?;
                        let made =
                            self.tracer
                                .select(crate::ops::Flag::raised(flag), then, otherwise);
                        self.grown(path)?;
                        Ok(Value::Word(made))
                    }
                }
            }
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
        return Value::list((0..8).map(Value::Word).collect());
    }
    let first = first_plane(program);
    let corners = u32::try_from(CORNERS).unwrap_or(8);
    Value::list(
        (0..corners)
            .map(|corner| {
                Value::list(
                    (0..program.channels)
                        .map(|plane| match plane {
                            0 => Value::Word(corner),
                            _ => Value::Word(first + (plane - 1) * corners + corner),
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

pub fn expand(program: &Program<'_>) -> Result<Expanded, Refusal> {
    let planes = program.channels.max(1);
    let extra = usize::try_from(planes - 1).unwrap_or(0) * CORNERS;
    let tracer = Tracer::with_inputs(program.params + extra);
    let inputs = tracer.nodes.len();
    let mut globals: HashMap<&str, Value> = HashMap::new();
    globals.insert("X", Value::Int(1));
    globals.insert("Z", Value::Int(2));
    globals.insert("Y", Value::Int(4));
    globals.insert("cells", cells_of(program));
    globals.insert("frame", Value::Word(FRAME));
    globals.insert(
        "at",
        Value::list([AT, AT + 1, AT + 2].into_iter().map(Value::Word).collect()),
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
        site: 0,
        sites: vec![0; inputs],
        halo: Halo::NONE,
        halo_inputs: HashMap::new(),
        looks: Vec::new(),
    };
    let root = Path::root();
    let step = root.key("step");
    let result = expander.eval(program.step, &step, 0)?;
    expander.site = address(program.step);
    let Value::List(halves) = &result else {
        return Err(shaped(&step, kind(&result)));
    };
    let [out, counts] = halves.items() else {
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
        for (index, one) in out.items().iter().enumerate() {
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
            for (plane, word) in words.items().iter().enumerate() {
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
        for (slot, (index, one)) in ids.iter_mut().zip(out.items().iter().enumerate()) {
            let made = operand(one, &step)
                .map_err(|_| shaped(&step, format!("{} at out[{index}]", kind(one))))?;
            *slot = expander.id(made, &step)?;
        }
    }
    let mut tallied = Vec::with_capacity(counts.len());
    for (index, one) in counts.items().iter().enumerate() {
        let made = operand(one, &step)
            .map_err(|_| shaped(&step, format!("{} at counts[{index}]", kind(one))))?;
        tallied.push(expander.id(made, &step)?);
    }
    looked(program, &expander.looks, (&ids, &upper))?;
    let mut sites = std::mem::take(&mut expander.sites);
    sites.resize(expander.tracer.nodes.len(), address(program.step));
    Ok(Expanded {
        sites,
        dag: Dag::new(
            expander.tracer.nodes,
            ids,
            tallied,
            program.params,
            program.tally,
        )
        .with_planes(upper)
        .with_halo(expander.halo),
        work: expander.work,
        depth: expander.deepest,
    })
}

fn looked(
    program: &Program<'_>,
    looks: &[(u32, String)],
    (out, upper): (&[u32; CORNERS], &[[u32; CORNERS]]),
) -> Result<(), Refusal> {
    let root = Path::root();
    let looking = !looks.is_empty();
    if looking != (program.halo == 1) {
        let (expected, found) = match looks.first() {
            Some((_, at)) => (
                String::from("halo 1, for a law that looks past its own oct"),
                format!("halo {}, and a look at {at}", program.halo),
            ),
            None => (
                String::from("halo 0, for a law whose looks all stay inside its oct"),
                format!("halo {}, and no look past the oct", program.halo),
            ),
        };
        return Err(refused(
            Code::HaloDeclared,
            &root.key("halo"),
            expected,
            found,
        ));
    }
    let corners = u32::try_from(CORNERS).unwrap_or(8);
    let first = first_plane(program);
    for (plane, at) in looks {
        let input = |corner: u32| match *plane {
            0 => corner,
            _ => first + (*plane - 1) * corners + corner,
        };
        let outs = match *plane {
            0 => Some(out),
            _ => upper.get(usize::try_from(*plane - 1).unwrap_or(usize::MAX)),
        };
        let written = outs.map_or(Some(0), |words| {
            words
                .iter()
                .zip(0..corners)
                .find(|(id, corner)| **id != input(*corner))
                .map(|(_, corner)| corner)
        });
        if let Some(corner) = written {
            return Err(Refusal {
                code: Code::HaloWritten,
                path: at.clone(),
                expected: format!(
                    "a look only at a plane the law leaves unchanged: out must give plane {plane} its own 8 words, or the look must read another plane"
                ),
                found: format!(
                    "a look at plane {plane}, which the law writes: out changes corner {corner} of plane {plane}"
                ),
                limit: None,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Place {
    Root,
    Key(Box<str>),
    Index(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    steps: Vec<(usize, Place)>,
    nodes: Vec<usize>,
}

const NOWHERE: usize = usize::MAX;

struct Locating<'j> {
    wanted: HashMap<usize, usize>,
    steps: Vec<(usize, Place)>,
    trail: Vec<(Step<'j>, usize)>,
}

impl<'j> Locating<'j> {
    fn interned(&mut self) -> usize {
        let mut parent = 0;
        for (step, id) in &mut self.trail {
            if *id == NOWHERE {
                let place = match *step {
                    Step::Key(key) => Place::Key(key.into()),
                    Step::Index(index) => Place::Index(index),
                    Step::Root => Place::Root,
                };
                *id = self.steps.len();
                self.steps.push((parent, place));
            }
            parent = *id;
        }
        parent
    }

    fn visit(&mut self, json: &'j Json) {
        if let Some(slot) = self.wanted.get(&address(json)).copied()
            && slot == NOWHERE
        {
            let id = self.interned();
            self.wanted.insert(address(json), id);
        }
        match json {
            Json::List(items) => {
                for (index, one) in items.iter().enumerate() {
                    self.trail.push((Step::Index(index), NOWHERE));
                    self.visit(one);
                    self.trail.pop();
                }
            }
            Json::Map(fields) => {
                for (key, one) in fields {
                    self.trail.push((Step::Key(key), NOWHERE));
                    self.visit(one);
                    self.trail.pop();
                }
            }
            Json::Null | Json::Bool(_) | Json::Int(_) | Json::Text(_) => {}
        }
    }
}

impl Places {
    #[must_use]
    pub(crate) fn of(json: &Json, sites: &[usize]) -> Self {
        let wanted: HashMap<usize, usize> = sites
            .iter()
            .filter(|&&site| site != 0)
            .map(|&site| (site, NOWHERE))
            .collect();
        let mut locating = Locating {
            wanted,
            steps: vec![(NOWHERE, Place::Root)],
            trail: Vec::new(),
        };
        locating.visit(json);
        let nodes = sites
            .iter()
            .map(|site| locating.wanted.get(site).copied().unwrap_or(NOWHERE))
            .collect();
        Self {
            steps: locating.steps,
            nodes,
        }
    }

    #[must_use]
    pub fn path(&self, node: u32) -> Option<String> {
        let mut here = usize::try_from(node)
            .ok()
            .and_then(|index| self.nodes.get(index))
            .copied()
            .filter(|&id| id != NOWHERE)?;
        let mut trail = Vec::new();
        while let Some((parent, place)) = self.steps.get(here) {
            trail.push(match place {
                Place::Root => Step::Root,
                Place::Key(key) => Step::Key(key),
                Place::Index(index) => Step::Index(*index),
            });
            here = *parent;
        }
        trail.reverse();
        Some(shown(&trail))
    }
}
