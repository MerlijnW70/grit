use std::collections::HashMap;

/// The bindings a compute module may write, read from its SPIR-V.
///
/// `Every` stands for a module the analysis cannot follow: it is taken to
/// write every binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    Only(Vec<usize>),
    Every,
}

impl Written {
    #[must_use]
    pub fn may_write(&self, binding: usize) -> bool {
        match self {
            Self::Only(bindings) => bindings.binary_search(&binding).is_ok(),
            Self::Every => true,
        }
    }
}

const MAGIC: u32 = 0x0723_0203;
const HEADER: usize = 5;

const NAME: u32 = 5;
const MEMBER_NAME: u32 = 6;
const STRING: u32 = 7;
const LINE: u32 = 8;
const SOURCE_CONTINUED: u32 = 2;
const SOURCE: u32 = 3;
const SOURCE_EXTENSION: u32 = 4;
const EXTENSION: u32 = 10;
const EXT_INST_IMPORT: u32 = 11;
const EXT_INST: u32 = 12;
const MEMORY_MODEL: u32 = 14;
const ENTRY_POINT: u32 = 15;
const EXECUTION_MODE: u32 = 16;
const CAPABILITY: u32 = 17;
const TYPE_POINTER: u32 = 32;
const FUNCTION: u32 = 54;
const FUNCTION_END: u32 = 56;
const FUNCTION_CALL: u32 = 57;
const VARIABLE: u32 = 59;
const LOAD: u32 = 61;
const STORE: u32 = 62;
const COPY_MEMORY: u32 = 63;
const COPY_MEMORY_SIZED: u32 = 64;
const ACCESS_CHAIN: u32 = 65;
const IN_BOUNDS_ACCESS_CHAIN: u32 = 66;
const PTR_ACCESS_CHAIN: u32 = 67;
const ARRAY_LENGTH: u32 = 68;
const IN_BOUNDS_PTR_ACCESS_CHAIN: u32 = 70;
const DECORATE: u32 = 71;
const MEMBER_DECORATE: u32 = 72;
const DECORATION_GROUP: u32 = 73;
const GROUP_DECORATE: u32 = 74;
const GROUP_MEMBER_DECORATE: u32 = 75;
const VECTOR_SHUFFLE: u32 = 79;
const COMPOSITE_EXTRACT: u32 = 81;
const COMPOSITE_INSERT: u32 = 82;
const COPY_OBJECT: u32 = 83;
const ATOMIC_LOAD: u32 = 227;
const ATOMIC_STORE: u32 = 228;
const ATOMIC_EXCHANGE: u32 = 229;
const ATOMIC_XOR: u32 = 242;
const ATOMIC_FLAG_TEST_AND_SET: u32 = 318;
const ATOMIC_FLAG_CLEAR: u32 = 319;
const ATOMIC_F_MIN: u32 = 5614;
const ATOMIC_F_MAX: u32 = 5615;
const ATOMIC_F_ADD: u32 = 6035;
const LOOP_MERGE: u32 = 246;
const SELECTION_MERGE: u32 = 247;
const LABEL: u32 = 248;
const BRANCH: u32 = 249;
const BRANCH_CONDITIONAL: u32 = 250;
const SWITCH: u32 = 251;
const NO_LINE: u32 = 317;
const MODULE_PROCESSED: u32 = 330;
const EXECUTION_MODE_ID: u32 = 331;
const DECORATE_ID: u32 = 332;
const GROUP_BALLOT_BIT_COUNT: u32 = 342;
const GROUP_I_ADD: u32 = 349;
const GROUP_BITWISE_XOR: u32 = 361;
const DECORATE_STRING: u32 = 5632;
const MEMBER_DECORATE_STRING: u32 = 5633;

const BINDING: u32 = 33;
const DESCRIPTOR_SET: u32 = 34;

const UNIFORM_CONSTANT: u32 = 0;
const UNIFORM: u32 = 2;
const IMAGE: u32 = 11;
const STORAGE_BUFFER: u32 = 12;
const PHYSICAL_STORAGE_BUFFER: u32 = 5349;

const ADDRESSES: u32 = 4;
const VARIABLE_POINTERS_STORAGE_BUFFER: u32 = 4441;
const VARIABLE_POINTERS: u32 = 4442;
const PHYSICAL_STORAGE_BUFFER_ADDRESSES: u32 = 5347;

struct Instruction<'words> {
    code: u32,
    operands: &'words [u32],
}

fn instructions(words: &[u32]) -> Option<Vec<Instruction<'_>>> {
    if words.first() != Some(&MAGIC) {
        return None;
    }
    let mut rest = words.get(HEADER..)?;
    let mut out = Vec::new();
    while let Some(&first) = rest.first() {
        let count = usize::try_from(first >> 16).ok()?;
        if count == 0 {
            return None;
        }
        let whole = rest.get(..count)?;
        out.push(Instruction {
            code: first & 0xFFFF,
            operands: whole.get(1..)?,
        });
        rest = rest.get(count..)?;
    }
    Some(out)
}

const fn declares(code: u32) -> bool {
    matches!(
        code,
        SOURCE_CONTINUED
            | SOURCE
            | SOURCE_EXTENSION
            | NAME
            | MEMBER_NAME
            | STRING
            | LINE
            | EXTENSION
            | EXT_INST_IMPORT
            | MEMORY_MODEL
            | ENTRY_POINT
            | EXECUTION_MODE
            | CAPABILITY
            | DECORATE
            | MEMBER_DECORATE
            | DECORATION_GROUP
            | GROUP_DECORATE
            | GROUP_MEMBER_DECORATE
            | NO_LINE
            | MODULE_PROCESSED
            | EXECUTION_MODE_ID
            | DECORATE_ID
            | DECORATE_STRING
            | MEMBER_DECORATE_STRING
            | 19..=39
    )
}

const fn resource(class: u32) -> bool {
    matches!(
        class,
        UNIFORM_CONSTANT | UNIFORM | IMAGE | STORAGE_BUFFER | PHYSICAL_STORAGE_BUFFER
    )
}

const fn atomic_at_result(code: u32) -> bool {
    matches!(
        code,
        ATOMIC_LOAD | ATOMIC_EXCHANGE
            ..=ATOMIC_XOR | ATOMIC_FLAG_TEST_AND_SET | ATOMIC_F_MIN | ATOMIC_F_MAX | ATOMIC_F_ADD
    )
}

const fn chains(code: u32) -> bool {
    matches!(
        code,
        ACCESS_CHAIN | IN_BOUNDS_ACCESS_CHAIN | PTR_ACCESS_CHAIN | IN_BOUNDS_PTR_ACCESS_CHAIN
    )
}

#[derive(Default)]
struct Found {
    sets: HashMap<u32, u32>,
    bindings: HashMap<u32, u32>,
    pointers: HashMap<u32, u32>,
    places: HashMap<u32, u32>,
    roots: HashMap<u32, usize>,
}

fn word(operands: &[u32], at: usize) -> Option<u32> {
    operands.get(at).copied()
}

fn declared(list: &[Instruction<'_>]) -> Option<Found> {
    let mut found = Found::default();
    for one in list {
        let ops = one.operands;
        match one.code {
            CAPABILITY => {
                if matches!(
                    word(ops, 0)?,
                    ADDRESSES
                        | VARIABLE_POINTERS_STORAGE_BUFFER
                        | VARIABLE_POINTERS
                        | PHYSICAL_STORAGE_BUFFER_ADDRESSES
                ) {
                    return None;
                }
            }
            DECORATE => match word(ops, 1)? {
                BINDING => {
                    found.bindings.insert(word(ops, 0)?, word(ops, 2)?);
                }
                DESCRIPTOR_SET => {
                    found.sets.insert(word(ops, 0)?, word(ops, 2)?);
                }
                _ => {}
            },
            TYPE_POINTER => {
                found.pointers.insert(word(ops, 0)?, word(ops, 1)?);
            }
            _ => {}
        }
    }
    for one in list {
        let ops = one.operands;
        if one.code == VARIABLE {
            let (id, class) = (word(ops, 1)?, word(ops, 2)?);
            if class == UNIFORM_CONSTANT {
                return None;
            }
            if !resource(class) {
                continue;
            }
            let set = found.sets.get(&id).copied();
            let binding = found.bindings.get(&id).copied();
            match (set, binding) {
                (Some(0), Some(binding)) => {
                    found.roots.insert(id, usize::try_from(binding).ok()?);
                }
                _ => return None,
            }
        } else if chains(one.code) || one.code == COPY_OBJECT {
            let (id, base) = (word(ops, 1)?, word(ops, 2)?);
            if let Some(&root) = found.roots.get(&base) {
                found.roots.insert(id, root);
            }
        }
    }
    for one in list {
        if declares(one.code) {
            continue;
        }
        let ops = one.operands;
        let Some(&class) = word(ops, 0).and_then(|kind| found.pointers.get(&kind)) else {
            continue;
        };
        let id = word(ops, 1)?;
        if resource(class) && !found.roots.contains_key(&id) {
            return None;
        }
        found.places.insert(id, class);
    }
    Some(found)
}

/// What `words` may write, or [`Written::Every`] when anything about its
/// pointers into bindings is unclear.
#[must_use]
pub fn written(words: &[u32]) -> Written {
    match follow(words) {
        Some(mut bindings) => {
            bindings.sort_unstable();
            bindings.dedup();
            Written::Only(bindings)
        }
        None => Written::Every,
    }
}

fn follow(words: &[u32]) -> Option<Vec<usize>> {
    let list = instructions(words)?;
    let found = declared(&list)?;
    let mut out = Vec::new();
    let held = |id: u32| found.roots.get(&id).copied();
    let clear = |ids: &[u32]| ids.iter().all(|&id| held(id).is_none());
    let mut into = |id: u32| match held(id) {
        Some(root) => {
            out.push(root);
            true
        }
        None => found.places.get(&id).is_some_and(|&class| !resource(class)),
    };

    for one in &list {
        let ops = one.operands;
        let tail = |from: usize| ops.get(from..).unwrap_or(&[]);
        let fine = match one.code {
            code if declares(code) => true,
            VARIABLE | FUNCTION | FUNCTION_END | LABEL | BRANCH => true,
            41..=46 | 48..=52 => true,
            LOAD | ATOMIC_LOAD | ARRAY_LENGTH => {
                word(ops, 2)?;
                true
            }
            STORE | ATOMIC_STORE | ATOMIC_FLAG_CLEAR => {
                into(word(ops, 0)?) && (one.code != STORE || clear(&[word(ops, 1)?]))
            }
            COPY_MEMORY | COPY_MEMORY_SIZED => {
                into(word(ops, 0)?)
                    && word(ops, 1).is_some()
                    && (one.code == COPY_MEMORY || clear(&[word(ops, 2)?]))
            }
            code if atomic_at_result(code) => into(word(ops, 2)?) && clear(tail(3)),
            code if chains(code) || code == COPY_OBJECT => {
                word(ops, 2)?;
                clear(tail(3))
            }
            COMPOSITE_EXTRACT => clear(ops.get(2..3)?),
            COMPOSITE_INSERT | VECTOR_SHUFFLE => clear(ops.get(2..4)?),
            EXT_INST => clear(tail(4)),
            FUNCTION_CALL => clear(tail(3)),
            GROUP_BALLOT_BIT_COUNT | GROUP_I_ADD..=GROUP_BITWISE_XOR => {
                clear(ops.get(2..3)?) && clear(tail(4))
            }
            LOOP_MERGE | SELECTION_MERGE => true,
            BRANCH_CONDITIONAL | SWITCH => clear(ops.get(..1)?),
            _ => clear(ops),
        };
        if !fine {
            return None;
        }
    }
    Some(out)
}
