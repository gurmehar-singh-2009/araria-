//! Cycle collector.
//!
//! Heap objects are reference counted, so acyclic garbage is freed
//! immediately. Cycles (a closure captured by its own upvalue, objects that
//! point at each other, ...) are found with the scheme CPython uses:
//!
//! 1. For every tracked object, count the references that come from *other
//!    tracked objects*.
//! 2. An object whose strong count exceeds that number is referenced from
//!    outside the heap (the interpreter stack, the host, a native closure) and
//!    is therefore a root.
//! 3. Everything not reachable from a root is cyclic garbage; its contents are
//!    cleared, which breaks the cycles and lets reference counting finish.
//!
//! Because roots are discovered rather than registered, host code may hold on
//! to any Lua value without telling the collector about it.

use std::rc::{Rc, Weak};

use crate::value::{CellBox, CellRef, Closure, TableBox, TableRef, Thread, ThreadRef, ThreadStatus, Value};
use crate::vm::ThreadState;

const MIN_THRESHOLD: usize = 16_384;

#[derive(Default)]
pub(crate) struct Gc {
    tables: Vec<Weak<TableBox>>,
    closures: Vec<Weak<Closure>>,
    cells: Vec<Weak<CellBox>>,
    threads: Vec<Weak<Thread>>,
    allocated: usize,
    threshold: usize,
    stats: GcStats,
}

/// Counters describing the Lua heap, reported by [`crate::Lua::gc_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcStats {
    pub tables: usize,
    pub closures: usize,
    pub threads: usize,
    pub collections: u64,
    /// Objects reclaimed by the most recent cycle collection.
    pub last_freed: usize,
}

enum Obj {
    Table(TableRef),
    Closure(Rc<Closure>),
    Cell(CellRef),
    Thread(ThreadRef),
}

fn visit_value(v: &Value, f: &mut impl FnMut(Obj)) {
    match v {
        Value::Table(t) => f(Obj::Table(t.clone())),
        Value::Function(c) => f(Obj::Closure(c.clone())),
        Value::Thread(t) => f(Obj::Thread(t.clone())),
        Value::Cell(c) => f(Obj::Cell(c.clone())),
        _ => {}
    }
}

fn visit_state(state: &ThreadState, f: &mut impl FnMut(Obj)) {
    for v in &state.stack {
        visit_value(v, f);
    }
    for frame in &state.frames {
        f(Obj::Closure(frame.closure.clone()));
        for v in &frame.varargs {
            visit_value(v, f);
        }
        if let Some(Some(handler)) = &frame.protect {
            visit_value(handler, f);
        }
    }
}

fn visit_children(obj: &Obj, f: &mut impl FnMut(Obj)) {
    match obj {
        Obj::Table(t) => {
            let t = t.0.t.borrow();
            for v in &t.arr {
                visit_value(v, f);
            }
            for (k, v) in &t.map {
                visit_value(k, f);
                visit_value(v, f);
            }
            if let Some(meta) = &t.meta {
                f(Obj::Table(meta.clone()));
            }
        }
        Obj::Closure(c) => {
            for cell in &c.upvals {
                f(Obj::Cell(cell.clone()));
            }
            f(Obj::Table(c.env.clone()));
        }
        Obj::Cell(c) => visit_value(&c.0.v.borrow(), f),
        Obj::Thread(t) => {
            if let Some(state) = t.0.state.borrow().as_deref() {
                visit_state(state, f);
            }
        }
    }
}

impl Obj {
    fn header(&self) -> &crate::value::GcHeader {
        match self {
            Obj::Table(t) => &t.0.gc,
            Obj::Closure(c) => &c.gc,
            Obj::Cell(c) => &c.0.gc,
            Obj::Thread(t) => &t.0.gc,
        }
    }

    fn strong_count(&self) -> usize {
        match self {
            Obj::Table(t) => Rc::strong_count(&t.0),
            Obj::Closure(c) => Rc::strong_count(c),
            Obj::Cell(c) => Rc::strong_count(&c.0),
            Obj::Thread(t) => Rc::strong_count(&t.0),
        }
    }
}

impl Gc {
    pub(crate) fn track_table(&mut self, t: &TableRef) {
        self.tables.push(Rc::downgrade(&t.0));
        self.allocated += 1;
    }

    pub(crate) fn track_closure(&mut self, c: &Rc<Closure>) {
        self.closures.push(Rc::downgrade(c));
        self.allocated += 1;
    }

    pub(crate) fn track_cell(&mut self, c: &CellRef) {
        self.cells.push(Rc::downgrade(&c.0));
        self.allocated += 1;
    }

    pub(crate) fn track_thread(&mut self, t: &ThreadRef) {
        self.threads.push(Rc::downgrade(&t.0));
        self.allocated += 1;
    }

    /// Whether enough allocation has happened to make a collection worthwhile.
    pub(crate) fn should_collect(&self) -> bool {
        self.allocated >= self.threshold.max(MIN_THRESHOLD)
    }

    pub(crate) fn stats(&self) -> GcStats {
        self.stats
    }

    pub(crate) fn collect(&mut self) -> GcStats {
        // Upgrade every live object. Each upgrade adds one strong reference
        // that is accounted for below.
        let mut objects: Vec<Obj> = Vec::with_capacity(
            self.tables.len() + self.closures.len() + self.cells.len() + self.threads.len(),
        );
        self.tables.retain(|w| match w.upgrade() {
            Some(rc) => {
                objects.push(Obj::Table(TableRef(rc)));
                true
            }
            None => false,
        });
        self.closures.retain(|w| match w.upgrade() {
            Some(rc) => {
                objects.push(Obj::Closure(rc));
                true
            }
            None => false,
        });
        self.cells.retain(|w| match w.upgrade() {
            Some(rc) => {
                objects.push(Obj::Cell(CellRef(rc)));
                true
            }
            None => false,
        });
        self.threads.retain(|w| match w.upgrade() {
            Some(rc) => {
                objects.push(Obj::Thread(ThreadRef(rc)));
                true
            }
            None => false,
        });

        for obj in &objects {
            obj.header().refs.set(0);
            obj.header().marked.set(false);
        }
        // Count heap-internal references. The clones made while visiting are
        // dropped immediately, so strong counts are unchanged afterwards.
        for obj in &objects {
            visit_children(obj, &mut |child| {
                let header = child.header();
                header.refs.set(header.refs.get() + 1);
            });
        }
        // Roots: referenced from somewhere other than tracked objects.
        let mut work: Vec<Obj> = Vec::new();
        for obj in &objects {
            let external = obj.strong_count() - 1 > obj.header().refs.get() as usize;
            if external && !obj.header().marked.replace(true) {
                work.push(match obj {
                    Obj::Table(t) => Obj::Table(t.clone()),
                    Obj::Closure(c) => Obj::Closure(c.clone()),
                    Obj::Cell(c) => Obj::Cell(c.clone()),
                    Obj::Thread(t) => Obj::Thread(t.clone()),
                });
            }
        }
        while let Some(obj) = work.pop() {
            visit_children(&obj, &mut |child| {
                if !child.header().marked.replace(true) {
                    work.push(child);
                }
            });
        }
        // Sweep: clear unreachable containers to break cycles.
        let mut freed = 0;
        for obj in &objects {
            if obj.header().marked.get() {
                continue;
            }
            freed += 1;
            match obj {
                Obj::Table(t) => t.0.t.borrow_mut().clear_all(),
                Obj::Cell(c) => c.set(Value::Nil),
                Obj::Thread(t) => {
                    t.0.state.borrow_mut().take();
                    t.0.status.set(ThreadStatus::Dead);
                }
                Obj::Closure(_) => {}
            }
        }
        let live = objects.len() - freed;
        drop(objects);
        self.allocated = 0;
        self.threshold = live * 2;
        self.stats = GcStats {
            tables: self.tables.iter().filter(|w| w.strong_count() > 0).count(),
            closures: self.closures.iter().filter(|w| w.strong_count() > 0).count(),
            threads: self.threads.iter().filter(|w| w.strong_count() > 0).count(),
            collections: self.stats.collections + 1,
            last_freed: freed,
        };
        self.stats
    }
}
