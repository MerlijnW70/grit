use std::rc::Rc;

#[derive(Clone)]
pub enum Value {
    Int(u32),
    Word(u32),
    Truth(bool),
    Flag(u32),
    List(Items),
}

#[derive(Clone)]
pub struct Items {
    depth: usize,
    held: Rc<[Value]>,
}

impl Value {
    #[must_use]
    pub fn list(items: Vec<Self>) -> Self {
        Self::List(Items::new(items))
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        match self {
            Self::List(items) => items.depth,
            Self::Int(_) | Self::Word(_) | Self::Truth(_) | Self::Flag(_) => 0,
        }
    }
}

impl Items {
    #[must_use]
    pub fn new(items: Vec<Value>) -> Self {
        let deepest = items.iter().map(Value::depth).max().unwrap_or(0);
        Self {
            depth: deepest.saturating_add(1),
            held: items.into(),
        }
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.held.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    #[must_use]
    pub fn items(&self) -> &[Value] {
        &self.held
    }
}

fn harvest(held: &mut Rc<[Value]>, pending: &mut Vec<Items>) {
    let Some(items) = Rc::get_mut(held) else {
        return;
    };
    for one in items.iter_mut() {
        if matches!(one, Value::List(_))
            && let Value::List(inner) = std::mem::replace(one, Value::Int(0))
        {
            pending.push(inner);
        }
    }
}

impl Drop for Items {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        harvest(&mut self.held, &mut pending);
        while let Some(mut one) = pending.pop() {
            harvest(&mut one.held, &mut pending);
        }
    }
}
