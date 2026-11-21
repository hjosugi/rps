//! #3: memory management.
//!
//! PowerShell objects form cycles (`$h.self = $h`, a note property pointing
//! at its object, a script block stored in the scope it closes over), and
//! they are shared between threads: `ForEach-Object -Parallel` with
//! `$using:` and `Start-ThreadJob -ArgumentList` hand the same object to
//! another runspace, which may mutate it. So reference counting alone leaks,
//! and a thread-local `Rc` cannot express what scripts do.
//!
//! Measured here:
//!
//! 1. the price of thread-safe sharing: `Rc` against `Arc` for clone and
//!    drop, and `RefCell` against `Mutex` / `RwLock` for touching a
//!    container;
//! 2. a cycle collector prototype in the style of CPython's: reference
//!    counts stay the primary mechanism, and a collector over container
//!    objects finds groups that are referenced only from inside themselves
//!    (trial deletion) and breaks them. Timed per collection against the
//!    number of live containers, and run over a loop that makes cyclic
//!    garbage, with and without collection, watching RSS.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::hint::black_box;
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock, Weak};

use rpsh_research::{Report, iterations, measure};

/// A value as the prototype collector sees it: a scalar or a reference to a
/// container (a hashtable, array, PSObject or scope in the real engine).
#[derive(Clone)]
enum Val {
    Int(i64),
    Obj(Arc<Container>),
}

struct Container {
    slots: Mutex<Vec<Val>>,
}

/// Every container ever allocated, weakly, so that the collector can find
/// them; and the allocation count that triggers a collection.
struct Heap {
    registry: Vec<Weak<Container>>,
    allocated_since_collection: usize,
    threshold: Option<usize>,
    collections: usize,
    freed: usize,
}

impl Heap {
    fn new(threshold: Option<usize>) -> Self {
        Heap {
            registry: Vec::new(),
            allocated_since_collection: 0,
            threshold,
            collections: 0,
            freed: 0,
        }
    }

    fn alloc(&mut self, slots: Vec<Val>) -> Arc<Container> {
        let container = Arc::new(Container {
            slots: Mutex::new(slots),
        });
        self.registry.push(Arc::downgrade(&container));
        self.allocated_since_collection += 1;
        container
    }

    /// Collects if the threshold is reached. Called at a safe point, where
    /// every reference the engine holds is in a variable, a register or a
    /// container, never only in a Rust local.
    fn safe_point(&mut self) {
        if let Some(threshold) = self.threshold
            && self.allocated_since_collection >= threshold
        {
            self.collect();
        }
    }

    /// Trial deletion: a container whose reference count is fully explained
    /// by references from other registered containers is not reachable
    /// from outside, unless a reachable container points at it.
    fn collect(&mut self) -> usize {
        self.collections += 1;
        self.allocated_since_collection = 0;
        let objects: Vec<Arc<Container>> = self
            .registry
            .drain(..)
            .filter_map(|weak| weak.upgrade())
            .collect();
        let index: HashMap<*const Container, usize> = objects
            .iter()
            .enumerate()
            .map(|(position, object)| (Arc::as_ptr(object), position))
            .collect();
        // Minus one: the reference `objects` itself holds.
        let mut external: Vec<isize> = objects
            .iter()
            .map(|object| Arc::strong_count(object) as isize - 1)
            .collect();
        for object in &objects {
            for slot in object.slots.lock().unwrap().iter() {
                if let Val::Obj(child) = slot
                    && let Some(&position) = index.get(&Arc::as_ptr(child))
                {
                    external[position] -= 1;
                }
            }
        }
        let mut reachable = vec![false; objects.len()];
        let mut stack: Vec<usize> = (0..objects.len())
            .filter(|&position| external[position] > 0)
            .collect();
        for &position in &stack {
            reachable[position] = true;
        }
        while let Some(position) = stack.pop() {
            for slot in objects[position].slots.lock().unwrap().iter() {
                if let Val::Obj(child) = slot
                    && let Some(&child_position) = index.get(&Arc::as_ptr(child))
                    && !reachable[child_position]
                {
                    reachable[child_position] = true;
                    stack.push(child_position);
                }
            }
        }
        let mut freed = 0;
        for (position, object) in objects.iter().enumerate() {
            if reachable[position] {
                self.registry.push(Arc::downgrade(object));
            } else {
                // Breaking the cycle; the containers are freed when
                // `objects` is dropped below.
                object.slots.lock().unwrap().clear();
                freed += 1;
            }
        }
        self.freed += freed;
        freed
    }
}

fn rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

/// `$h = @{ n = $i; data = ... }; $h.self = $h` in a loop: each iteration
/// leaves one cyclic container behind.
fn cyclic_garbage(heap: &mut Heap, count: usize) -> i64 {
    let mut total = 0;
    for n in 0..count {
        let mut slots = vec![Val::Int(0); 8];
        slots[0] = Val::Int(n as i64);
        let container = heap.alloc(slots);
        container
            .slots
            .lock()
            .unwrap()
            .push(Val::Obj(container.clone()));
        if let Val::Int(value) = container.slots.lock().unwrap()[0] {
            total += value;
        }
        drop(container);
        heap.safe_point();
    }
    total
}

fn main() {
    let mut report = Report::new(
        "memory",
        "thread-safe reference counting and a cycle collector prototype (#3)",
    );
    report.header();

    // 1. The price of sharing.
    let rc = Rc::new(42u64);
    report.add(measure(
        "clone + drop",
        "Rc",
        iterations(10_000_000),
        || {
            let copy = Rc::clone(&rc);
            black_box(&copy);
            drop(copy);
        },
    ));
    let arc = Arc::new(42u64);
    report.add(measure(
        "clone + drop",
        "Arc",
        iterations(10_000_000),
        || {
            let copy = Arc::clone(&arc);
            black_box(&copy);
            drop(copy);
        },
    ));

    let cell = Rc::new(RefCell::new(vec![0i64; 64]));
    report.add(measure(
        "element read + write",
        "Rc<RefCell<Vec>>",
        iterations(10_000_000),
        || {
            let mut slots = cell.borrow_mut();
            slots[7] = slots[7].wrapping_add(1);
            slots[7]
        },
    ));
    let mutex = Arc::new(Mutex::new(vec![0i64; 64]));
    report.add(measure(
        "element read + write",
        "Arc<Mutex<Vec>>",
        iterations(10_000_000),
        || {
            let mut slots = mutex.lock().unwrap();
            slots[7] = slots[7].wrapping_add(1);
            slots[7]
        },
    ));
    let rwlock = Arc::new(RwLock::new(vec![0i64; 64]));
    report.add(measure(
        "element read",
        "Arc<RwLock<Vec>> (read)",
        iterations(10_000_000),
        || rwlock.read().unwrap()[7],
    ));
    report.add(measure(
        "element read",
        "Arc<Mutex<Vec>>",
        iterations(10_000_000),
        || mutex.lock().unwrap()[7],
    ));
    report.add(measure(
        "element read",
        "Rc<RefCell<Vec>>",
        iterations(10_000_000),
        || cell.borrow()[7],
    ));

    // 2a. Collection pause against the live heap: `live` reachable
    // containers in chains of 10, plus as many cyclic garbage containers.
    for live in [1_000usize, 10_000, 100_000] {
        let live = if rpsh_research::quick() {
            live / 10
        } else {
            live
        };
        let mut heap = Heap::new(None);
        let mut roots = Vec::new();
        for chain in 0..live / 10 {
            let mut next = Val::Int(chain as i64);
            for _ in 0..10 {
                next = Val::Obj(heap.alloc(vec![next, Val::Int(1)]));
            }
            roots.push(next);
        }
        report.add(measure(
            &format!("collect, {live} live"),
            "trial deletion",
            if live >= 100_000 { 3 } else { 30 },
            || {
                for n in 0..live {
                    let garbage = heap.alloc(vec![Val::Int(n as i64)]);
                    garbage
                        .slots
                        .lock()
                        .unwrap()
                        .push(Val::Obj(garbage.clone()));
                }
                heap.collect()
            },
        ));
        assert_eq!(heap.registry.len(), live, "a live container was collected");
        black_box(&roots);
    }

    // 2b. A loop that makes cyclic garbage: memory grows without a
    // collector and stays flat with one.
    let count = if rpsh_research::quick() {
        20_000
    } else {
        1_000_000
    };
    for (candidate, threshold) in [
        ("no collector", None),
        ("collect every 10,000", Some(10_000)),
        ("collect every 100,000", Some(100_000)),
    ] {
        let before = rss_kib();
        let mut heap = Heap::new(threshold);
        let start = std::time::Instant::now();
        black_box(cyclic_garbage(&mut heap, count));
        let elapsed = start.elapsed();
        let after = rss_kib();
        report.fact(
            &format!("{count} cyclic containers, {candidate}"),
            format!(
                "{:.1} ns per iteration, RSS +{} KiB, {} collections, {} freed",
                elapsed.as_nanos() as f64 / count as f64,
                after.saturating_sub(before),
                heap.collections,
                heap.freed
            ),
        );
        heap.collect();
    }
    report.save();
}
