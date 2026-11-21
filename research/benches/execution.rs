//! #4: how rpsh should execute a script.
//!
//! Four evaluators run the two programs pwsh is timed on in
//! `bench/cases/language`, built here as ASTs (no parser is involved):
//!
//! ```powershell
//! # loop-sum.ps1
//! $sum = 0
//! for ($i = 0; $i -lt 1000000; $i++) { $sum += $i }
//! $sum
//!
//! # function-calls.ps1
//! function Fib($n) { if ($n -lt 2) { return $n }; return (Fib ($n - 1)) + (Fib ($n - 2)) }
//! Fib 18
//! ```
//!
//! Candidates:
//!
//! * `ast-names`: walk the AST; every variable is looked up by name in a
//!   chain of hash-map scopes, PowerShell's dynamic scoping done naively.
//! * `ast-slots`: walk an AST whose variables were resolved to frame slots
//!   beforehand, falling back to a lookup by name where a variable is not
//!   definitely assigned (upstream's `VariableAnalysis` does the same).
//! * `stack-vm`: bytecode for an operand stack, same slots.
//! * `register-vm`: instructions that name their source and destination
//!   registers; variables live in registers, as in Lua and Nushell's IR.
//! * `register-vm+call-cache`: the register VM with an inline cache at each
//!   call site that remembers which function a name resolved to, validated
//!   by a generation number.
//!
//! What every candidate shares, so that only the execution model differs:
//!
//! * the same `Value` (16 bytes, with a reference-counted variant that these
//!   programs never create, so that a clone has to look at the tag) and the
//!   same dynamically typed operators: `Int32` overflow promotes to `Double`
//!   exactly as `IntOps.Add` does upstream, so `$sum` ends as a `Double`;
//! * a function call looks the function up by name (command discovery is
//!   dynamic in PowerShell: a function can be redefined at any time), creates
//!   a new scope whose parent is the caller's, and binds parameters by
//!   position;
//! * every result is checked: loop-sum must print the Double 499999500000 and
//!   Fib 18 the Int32 2584, as pwsh does.
//!
//! Not modelled: pipelines and output collection, parameter binding beyond
//! position, error records, `$?`, debugger sequence points, strict mode.
//! They cost the same in every candidate, or belong to other issues.

use std::cell::Cell;
use std::collections::HashMap;
use std::hint::black_box;
use std::mem::size_of;
use std::rc::Rc;

use rpsh_research::{Report, iterations, measure};

const LOOP_COUNT: i32 = 1_000_000;
const FIB_ARGUMENT: i32 = 18;

// ---------------------------------------------------------------- values

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Null,
    Bool(bool),
    Int32(i32),
    Double(f64),
    /// Never created by these programs. It is here so that `Value` has drop
    /// glue and a clone has to check the tag, as rpsh's value will.
    #[expect(
        dead_code,
        reason = "present for its effect on Value's layout and clone"
    )]
    Text(Rc<String>),
}

const NULL: &Value = &Value::Null;

impl Value {
    /// PowerShell's conversion to `bool` (`LanguagePrimitives.IsTrue`), for
    /// the types that exist here.
    fn is_true(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::Int32(value) => *value != 0,
            Value::Double(value) => *value != 0.0,
            Value::Text(text) => !text.is_empty(),
        }
    }
}

#[derive(Debug)]
enum Error {
    Unsupported(&'static str),
    UnknownCommand(Rc<str>),
}

impl Error {
    fn message(&self) -> String {
        match self {
            Error::Unsupported(what) => format!("not supported: {what}"),
            Error::UnknownCommand(name) => format!("unknown command: {name}"),
        }
    }
}

type Outcome<T> = Result<T, Error>;

#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Add,
    Subtract,
    LessThan,
}

/// The operators, dispatched on both operand types at run time.
mod ops {
    use super::{BinaryOp, Error, Outcome, Value};

    pub(super) fn apply(op: BinaryOp, left: &Value, right: &Value) -> Outcome<Value> {
        match op {
            BinaryOp::Add => add(left, right),
            BinaryOp::Subtract => subtract(left, right),
            BinaryOp::LessThan => less_than(left, right),
        }
    }

    /// `IntOps.Add` (`engine/runtime/Operations/NumericOps.cs`): the sum is
    /// computed in 64 bits and becomes a `Double` when it leaves `Int32`.
    pub(super) fn add(left: &Value, right: &Value) -> Outcome<Value> {
        Ok(match (left, right) {
            (Value::Int32(a), Value::Int32(b)) => match a.checked_add(*b) {
                Some(sum) => Value::Int32(sum),
                None => Value::Double(f64::from(*a) + f64::from(*b)),
            },
            (Value::Int32(a), Value::Double(b)) => Value::Double(f64::from(*a) + b),
            (Value::Double(a), Value::Int32(b)) => Value::Double(a + f64::from(*b)),
            (Value::Double(a), Value::Double(b)) => Value::Double(a + b),
            (Value::Null, other) | (other, Value::Null) => other.clone(),
            (Value::Bool(a), _) => return add(&Value::Int32(i32::from(*a)), right),
            (_, Value::Bool(b)) => return add(left, &Value::Int32(i32::from(*b))),
            _ => return Err(Error::Unsupported("+ on these operand types")),
        })
    }

    pub(super) fn subtract(left: &Value, right: &Value) -> Outcome<Value> {
        Ok(match (left, right) {
            (Value::Int32(a), Value::Int32(b)) => match a.checked_sub(*b) {
                Some(difference) => Value::Int32(difference),
                None => Value::Double(f64::from(*a) - f64::from(*b)),
            },
            (Value::Int32(a), Value::Double(b)) => Value::Double(f64::from(*a) - b),
            (Value::Double(a), Value::Int32(b)) => Value::Double(a - f64::from(*b)),
            (Value::Double(a), Value::Double(b)) => Value::Double(a - b),
            _ => return Err(Error::Unsupported("- on these operand types")),
        })
    }

    pub(super) fn less_than(left: &Value, right: &Value) -> Outcome<Value> {
        let result = match (left, right) {
            (Value::Int32(a), Value::Int32(b)) => a < b,
            (Value::Int32(a), Value::Double(b)) => f64::from(*a) < *b,
            (Value::Double(a), Value::Int32(b)) => *a < f64::from(*b),
            (Value::Double(a), Value::Double(b)) => a < b,
            _ => return Err(Error::Unsupported("-lt on these operand types")),
        };
        Ok(Value::Bool(result))
    }
}

// ------------------------------------------------------------------- AST

#[derive(Debug, Clone)]
enum Expr {
    Constant(Value),
    Variable(Rc<str>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// `(Name arg ...)`: a function invoked with positional arguments, its
    /// output taken as a value.
    Call(Rc<str>, Vec<Expr>),
}

#[derive(Debug, Clone)]
enum Stmt {
    Assign(Rc<str>, Expr),
    AddAssign(Rc<str>, Expr),
    /// `$name++` as a statement.
    Increment(Rc<str>),
    For {
        init: Box<Stmt>,
        condition: Expr,
        step: Box<Stmt>,
        body: Vec<Stmt>,
    },
    If {
        condition: Expr,
        then: Vec<Stmt>,
    },
    Return(Expr),
    /// An expression statement: its value goes to the output.
    Output(Expr),
}

#[derive(Debug)]
struct Function {
    /// Lowercased: PowerShell names are case-insensitive.
    name: Rc<str>,
    params: Vec<Rc<str>>,
    body: Vec<Stmt>,
}

#[derive(Debug)]
struct Program {
    functions: Vec<Function>,
    main: Vec<Stmt>,
}

fn name(text: &str) -> Rc<str> {
    Rc::from(text.to_ascii_lowercase())
}

fn var(text: &str) -> Expr {
    Expr::Variable(name(text))
}

fn int(value: i32) -> Expr {
    Expr::Constant(Value::Int32(value))
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary(op, Box::new(left), Box::new(right))
}

fn loop_sum(count: i32) -> Program {
    Program {
        functions: Vec::new(),
        main: vec![
            Stmt::Assign(name("sum"), int(0)),
            Stmt::For {
                init: Box::new(Stmt::Assign(name("i"), int(0))),
                condition: binary(BinaryOp::LessThan, var("i"), int(count)),
                step: Box::new(Stmt::Increment(name("i"))),
                body: vec![Stmt::AddAssign(name("sum"), var("i"))],
            },
            Stmt::Output(var("sum")),
        ],
    }
}

fn fib_function(function_name: &str) -> Function {
    let call = |delta| {
        Expr::Call(
            name(function_name),
            vec![binary(BinaryOp::Subtract, var("n"), int(delta))],
        )
    };
    Function {
        name: name(function_name),
        params: vec![name("n")],
        body: vec![
            Stmt::If {
                condition: binary(BinaryOp::LessThan, var("n"), int(2)),
                then: vec![Stmt::Return(var("n"))],
            },
            Stmt::Return(binary(BinaryOp::Add, call(1), call(2))),
        ],
    }
}

fn fib(argument: i32) -> Program {
    Program {
        functions: vec![fib_function("Fib")],
        main: vec![Stmt::Output(Expr::Call(name("Fib"), vec![int(argument)]))],
    }
}

/// A script of `copies` functions and `copies` loops, to time lowering on
/// something the size of a real profile or module rather than on two lines.
fn synthetic(copies: usize) -> Program {
    let mut main = Vec::new();
    for index in 0..copies {
        let total = format!("s{index}");
        main.push(Stmt::Assign(name(&total), int(0)));
        main.push(Stmt::For {
            init: Box::new(Stmt::Assign(name("i"), int(0))),
            condition: binary(BinaryOp::LessThan, var("i"), int(10)),
            step: Box::new(Stmt::Increment(name("i"))),
            body: vec![Stmt::AddAssign(name(&total), var("i"))],
        });
        main.push(Stmt::Output(var(&total)));
    }
    Program {
        functions: (0..copies)
            .map(|index| fib_function(&format!("f{index}")))
            .collect(),
        main,
    }
}

fn count_expr(expr: &Expr) -> usize {
    1 + match expr {
        Expr::Constant(_) | Expr::Variable(_) => 0,
        Expr::Binary(_, left, right) => count_expr(left) + count_expr(right),
        Expr::Call(_, args) => args.iter().map(count_expr).sum(),
    }
}

fn count_stmt(stmt: &Stmt) -> usize {
    1 + match stmt {
        Stmt::Assign(_, expr) | Stmt::AddAssign(_, expr) => count_expr(expr),
        Stmt::Increment(_) => 0,
        Stmt::For {
            init,
            condition,
            step,
            body,
        } => {
            count_stmt(init)
                + count_expr(condition)
                + count_stmt(step)
                + body.iter().map(count_stmt).sum::<usize>()
        }
        Stmt::If { condition, then } => {
            count_expr(condition) + then.iter().map(count_stmt).sum::<usize>()
        }
        Stmt::Return(expr) | Stmt::Output(expr) => count_expr(expr),
    }
}

fn count_nodes(program: &Program) -> usize {
    program
        .functions
        .iter()
        .flat_map(|function| &function.body)
        .chain(&program.main)
        .map(count_stmt)
        .sum()
}

// ----------------------------------------------- candidate a: AST + names

mod by_name {
    use super::{Error, Expr, HashMap, Outcome, Program, Rc, Stmt, Value, ops};

    enum Flow {
        Next,
        Return(Value),
    }

    struct Interpreter<'p> {
        program: &'p Program,
        functions: HashMap<Rc<str>, usize>,
        /// Innermost last. A function's scope is pushed on top of its
        /// caller's: PowerShell scoping is dynamic.
        scopes: Vec<HashMap<Rc<str>, Value>>,
        output: Vec<Value>,
    }

    pub(super) fn run(program: &Program) -> Outcome<Vec<Value>> {
        let mut interpreter = Interpreter {
            program,
            functions: program
                .functions
                .iter()
                .enumerate()
                .map(|(index, function)| (function.name.clone(), index))
                .collect(),
            scopes: vec![HashMap::new()],
            output: Vec::new(),
        };
        interpreter.block(&program.main)?;
        Ok(interpreter.output)
    }

    impl Interpreter<'_> {
        fn block(&mut self, stmts: &[Stmt]) -> Outcome<Flow> {
            for stmt in stmts {
                if let Flow::Return(value) = self.stmt(stmt)? {
                    return Ok(Flow::Return(value));
                }
            }
            Ok(Flow::Next)
        }

        fn stmt(&mut self, stmt: &Stmt) -> Outcome<Flow> {
            match stmt {
                Stmt::Assign(name, expr) => {
                    let value = self.expr(expr)?;
                    self.set(name, value);
                }
                Stmt::AddAssign(name, expr) => {
                    let right = self.expr(expr)?;
                    let value = ops::add(&self.get(name), &right)?;
                    self.set(name, value);
                }
                Stmt::Increment(name) => {
                    let value = ops::add(&self.get(name), &Value::Int32(1))?;
                    self.set(name, value);
                }
                Stmt::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    self.stmt(init)?;
                    while self.expr(condition)?.is_true() {
                        if let Flow::Return(value) = self.block(body)? {
                            return Ok(Flow::Return(value));
                        }
                        self.stmt(step)?;
                    }
                }
                Stmt::If { condition, then } => {
                    if self.expr(condition)?.is_true() {
                        return self.block(then);
                    }
                }
                Stmt::Return(expr) => return Ok(Flow::Return(self.expr(expr)?)),
                Stmt::Output(expr) => {
                    let value = self.expr(expr)?;
                    self.output.push(value);
                }
            }
            Ok(Flow::Next)
        }

        fn expr(&mut self, expr: &Expr) -> Outcome<Value> {
            match expr {
                Expr::Constant(value) => Ok(value.clone()),
                Expr::Variable(name) => Ok(self.get(name)),
                Expr::Binary(op, left, right) => {
                    let left = self.expr(left)?;
                    let right = self.expr(right)?;
                    ops::apply(*op, &left, &right)
                }
                Expr::Call(name, args) => self.call(name, args),
            }
        }

        fn get(&self, name: &str) -> Value {
            for scope in self.scopes.iter().rev() {
                if let Some(value) = scope.get(name) {
                    return value.clone();
                }
            }
            Value::Null
        }

        /// Assignment creates or updates the variable in the current scope.
        fn set(&mut self, name: &Rc<str>, value: Value) {
            let scope = self.scopes.last_mut().expect("there is always a scope");
            match scope.get_mut(name) {
                Some(slot) => *slot = value,
                None => {
                    scope.insert(name.clone(), value);
                }
            }
        }

        fn call(&mut self, name: &Rc<str>, args: &[Expr]) -> Outcome<Value> {
            let program = self.program;
            let &index = self
                .functions
                .get(name)
                .ok_or_else(|| Error::UnknownCommand(name.clone()))?;
            let function = &program.functions[index];
            let mut scope = HashMap::with_capacity(function.params.len());
            for (position, param) in function.params.iter().enumerate() {
                let value = match args.get(position) {
                    Some(arg) => self.expr(arg)?,
                    None => Value::Null,
                };
                scope.insert(param.clone(), value);
            }
            self.scopes.push(scope);
            let flow = self.block(&function.body);
            self.scopes.pop();
            match flow? {
                Flow::Return(value) => Ok(value),
                Flow::Next => Ok(Value::Null),
            }
        }
    }
}

// ------------------------------------------------- slots and frames, shared

/// Which variables of a function get slots, and where a read may use one.
///
/// Parameters come first, then every variable the body assigns. A read uses
/// the slot only where the variable is definitely assigned; elsewhere it is
/// looked up by name, which also finds the caller's variables. Upstream's
/// `VariableAnalysis` makes the same decision with a flow analysis
/// (`engine/parser/VariableAnalysis.cs`, the `assignedBitArray` test).
mod resolve {
    use super::{HashMap, Rc, Stmt};

    #[derive(Debug, Clone)]
    pub(super) enum Read {
        Slot(u16),
        Name(Rc<str>),
    }

    pub(super) struct Resolver {
        pub(super) names: Vec<Rc<str>>,
        index: HashMap<Rc<str>, u16>,
        assigned: Vec<bool>,
    }

    impl Resolver {
        pub(super) fn new(params: &[Rc<str>], body: &[Stmt]) -> Self {
            let mut resolver = Self {
                names: Vec::new(),
                index: HashMap::new(),
                assigned: Vec::new(),
            };
            for param in params {
                resolver.declare(param);
            }
            resolver.assigned.fill(true);
            for stmt in body {
                resolver.collect(stmt);
            }
            resolver
        }

        fn declare(&mut self, name: &Rc<str>) {
            if !self.index.contains_key(name) {
                let slot = u16::try_from(self.names.len()).expect("fewer than 65536 variables");
                self.index.insert(name.clone(), slot);
                self.names.push(name.clone());
                self.assigned.push(false);
            }
        }

        fn collect(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Assign(name, _) | Stmt::AddAssign(name, _) | Stmt::Increment(name) => {
                    self.declare(name);
                }
                Stmt::For {
                    init, step, body, ..
                } => {
                    self.collect(init);
                    self.collect(step);
                    for stmt in body {
                        self.collect(stmt);
                    }
                }
                Stmt::If { then, .. } => {
                    for stmt in then {
                        self.collect(stmt);
                    }
                }
                Stmt::Return(_) | Stmt::Output(_) => {}
            }
        }

        pub(super) fn read(&self, name: &Rc<str>) -> Read {
            match self.index.get(name) {
                Some(&slot) if self.assigned[usize::from(slot)] => Read::Slot(slot),
                _ => Read::Name(name.clone()),
            }
        }

        /// The slot an assignment writes; from here on it is assigned.
        pub(super) fn write(&mut self, name: &Rc<str>) -> u16 {
            let slot = self.index[name];
            self.assigned[usize::from(slot)] = true;
            slot
        }

        /// Assignments inside a branch or a loop body do not survive it.
        pub(super) fn snapshot(&self) -> Vec<bool> {
            self.assigned.clone()
        }

        pub(super) fn restore(&mut self, snapshot: Vec<bool>) {
            self.assigned = snapshot;
        }
    }
}

/// One function invocation's variables: the slots of the callee, with a link
/// to the caller's frame for lookups by name. `None` is a slot not assigned
/// yet (upstream's `MutableTuple` keeps a bit per slot for the same reason).
struct Frame<'a> {
    slots: Vec<Option<Value>>,
    names: &'a [Rc<str>],
    parent: Option<&'a Frame<'a>>,
}

impl Frame<'_> {
    fn new_root(size: usize, names: &[Rc<str>]) -> Frame<'_> {
        Frame {
            slots: vec![None; size],
            names,
            parent: None,
        }
    }

    fn slot(&self, slot: u16) -> &Value {
        self.slots[usize::from(slot)].as_ref().unwrap_or(NULL)
    }

    /// A lookup by name, through this frame and its callers'.
    fn by_name(&self, name: &str) -> Value {
        let mut frame = Some(self);
        while let Some(current) = frame {
            if let Some(position) = current.names.iter().position(|n| &**n == name)
                && let Some(value) = &current.slots[position]
            {
                return value.clone();
            }
            frame = current.parent;
        }
        Value::Null
    }
}

// ------------------------------------------------ candidate b: AST + slots

mod by_slot {
    use super::resolve::{Read, Resolver};
    use super::{BinaryOp, Error, Expr, Frame, HashMap, Outcome, Program, Rc, Stmt, Value, ops};

    #[derive(Debug)]
    enum RExpr {
        Constant(Value),
        Read(Read),
        Binary(BinaryOp, Box<RExpr>, Box<RExpr>),
        Call(Rc<str>, Vec<RExpr>),
    }

    #[derive(Debug)]
    enum RStmt {
        Assign(u16, RExpr),
        /// slot, read of the old value, right-hand side
        AddAssign(u16, Read, RExpr),
        Increment(u16, Read),
        For {
            init: Box<RStmt>,
            condition: RExpr,
            step: Box<RStmt>,
            body: Vec<RStmt>,
        },
        If {
            condition: RExpr,
            then: Vec<RStmt>,
        },
        Return(RExpr),
        Output(RExpr),
    }

    #[derive(Debug)]
    struct RFunction {
        params: usize,
        names: Vec<Rc<str>>,
        body: Vec<RStmt>,
    }

    #[derive(Debug)]
    pub(super) struct Resolved {
        functions: Vec<RFunction>,
        index: HashMap<Rc<str>, usize>,
        main: RFunction,
    }

    pub(super) fn lower(program: &Program) -> Resolved {
        let function = |params: &[Rc<str>], body: &[Stmt]| {
            let mut resolver = Resolver::new(params, body);
            let body = body.iter().map(|stmt| stmt_(&mut resolver, stmt)).collect();
            RFunction {
                params: params.len(),
                names: resolver.names,
                body,
            }
        };
        Resolved {
            functions: program
                .functions
                .iter()
                .map(|f| function(&f.params, &f.body))
                .collect(),
            index: program
                .functions
                .iter()
                .enumerate()
                .map(|(index, f)| (f.name.clone(), index))
                .collect(),
            main: function(&[], &program.main),
        }
    }

    fn expr_(resolver: &Resolver, expr: &Expr) -> RExpr {
        match expr {
            Expr::Constant(value) => RExpr::Constant(value.clone()),
            Expr::Variable(name) => RExpr::Read(resolver.read(name)),
            Expr::Binary(op, left, right) => RExpr::Binary(
                *op,
                Box::new(expr_(resolver, left)),
                Box::new(expr_(resolver, right)),
            ),
            Expr::Call(name, args) => RExpr::Call(
                name.clone(),
                args.iter().map(|arg| expr_(resolver, arg)).collect(),
            ),
        }
    }

    fn stmt_(resolver: &mut Resolver, stmt: &Stmt) -> RStmt {
        match stmt {
            Stmt::Assign(name, expr) => {
                let value = expr_(resolver, expr);
                RStmt::Assign(resolver.write(name), value)
            }
            Stmt::AddAssign(name, expr) => {
                let old = resolver.read(name);
                let value = expr_(resolver, expr);
                RStmt::AddAssign(resolver.write(name), old, value)
            }
            Stmt::Increment(name) => {
                let old = resolver.read(name);
                RStmt::Increment(resolver.write(name), old)
            }
            Stmt::For {
                init,
                condition,
                step,
                body,
            } => {
                let init = Box::new(stmt_(resolver, init));
                let condition = expr_(resolver, condition);
                let before = resolver.snapshot();
                let body = body.iter().map(|stmt| stmt_(resolver, stmt)).collect();
                let step = Box::new(stmt_(resolver, step));
                resolver.restore(before);
                RStmt::For {
                    init,
                    condition,
                    step,
                    body,
                }
            }
            Stmt::If { condition, then } => {
                let condition = expr_(resolver, condition);
                let before = resolver.snapshot();
                let then = then.iter().map(|stmt| stmt_(resolver, stmt)).collect();
                resolver.restore(before);
                RStmt::If { condition, then }
            }
            Stmt::Return(expr) => RStmt::Return(expr_(resolver, expr)),
            Stmt::Output(expr) => RStmt::Output(expr_(resolver, expr)),
        }
    }

    enum Flow {
        Next,
        Return(Value),
    }

    struct Interpreter<'p> {
        program: &'p Resolved,
        output: Vec<Value>,
    }

    pub(super) fn run(program: &Resolved) -> Outcome<Vec<Value>> {
        let mut interpreter = Interpreter {
            program,
            output: Vec::new(),
        };
        let main = &program.main;
        let mut frame = Frame::new_root(main.names.len(), &main.names);
        interpreter.block(&main.body, &mut frame)?;
        Ok(interpreter.output)
    }

    impl Interpreter<'_> {
        fn block(&mut self, stmts: &[RStmt], frame: &mut Frame<'_>) -> Outcome<Flow> {
            for stmt in stmts {
                if let Flow::Return(value) = self.stmt(stmt, frame)? {
                    return Ok(Flow::Return(value));
                }
            }
            Ok(Flow::Next)
        }

        fn read(frame: &Frame<'_>, read: &Read) -> Value {
            match read {
                Read::Slot(slot) => frame.slot(*slot).clone(),
                Read::Name(name) => frame.by_name(name),
            }
        }

        fn stmt(&mut self, stmt: &RStmt, frame: &mut Frame<'_>) -> Outcome<Flow> {
            match stmt {
                RStmt::Assign(slot, expr) => {
                    let value = self.expr(expr, frame)?;
                    frame.slots[usize::from(*slot)] = Some(value);
                }
                RStmt::AddAssign(slot, old, expr) => {
                    let right = self.expr(expr, frame)?;
                    let value = ops::add(&Self::read(frame, old), &right)?;
                    frame.slots[usize::from(*slot)] = Some(value);
                }
                RStmt::Increment(slot, old) => {
                    let value = ops::add(&Self::read(frame, old), &Value::Int32(1))?;
                    frame.slots[usize::from(*slot)] = Some(value);
                }
                RStmt::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    self.stmt(init, frame)?;
                    while self.expr(condition, frame)?.is_true() {
                        if let Flow::Return(value) = self.block(body, frame)? {
                            return Ok(Flow::Return(value));
                        }
                        self.stmt(step, frame)?;
                    }
                }
                RStmt::If { condition, then } => {
                    if self.expr(condition, frame)?.is_true() {
                        return self.block(then, frame);
                    }
                }
                RStmt::Return(expr) => return Ok(Flow::Return(self.expr(expr, frame)?)),
                RStmt::Output(expr) => {
                    let value = self.expr(expr, frame)?;
                    self.output.push(value);
                }
            }
            Ok(Flow::Next)
        }

        fn expr(&mut self, expr: &RExpr, frame: &mut Frame<'_>) -> Outcome<Value> {
            match expr {
                RExpr::Constant(value) => Ok(value.clone()),
                RExpr::Read(read) => Ok(Self::read(frame, read)),
                RExpr::Binary(op, left, right) => {
                    let left = self.expr(left, frame)?;
                    let right = self.expr(right, frame)?;
                    ops::apply(*op, &left, &right)
                }
                RExpr::Call(name, args) => {
                    let program = self.program;
                    let &index = program
                        .index
                        .get(name)
                        .ok_or_else(|| Error::UnknownCommand(name.clone()))?;
                    let function = &program.functions[index];
                    let mut slots = vec![None; function.names.len()];
                    for (position, slot) in slots.iter_mut().take(function.params).enumerate() {
                        *slot = Some(match args.get(position) {
                            Some(arg) => self.expr(arg, frame)?,
                            None => Value::Null,
                        });
                    }
                    let mut callee = Frame {
                        slots,
                        names: &function.names,
                        parent: Some(&*frame),
                    };
                    match self.block(&function.body, &mut callee)? {
                        Flow::Return(value) => Ok(value),
                        Flow::Next => Ok(Value::Null),
                    }
                }
            }
        }
    }
}

// --------------------------------------------------- candidate c: stack VM

mod stack_vm {
    use super::resolve::{Read, Resolver};
    use super::{BinaryOp, Error, Expr, Frame, HashMap, Outcome, Program, Rc, Stmt, Value, ops};

    #[derive(Debug, Clone)]
    pub(super) enum Op {
        Constant(u32),
        Slot(u16),
        Name(u32),
        Store(u16),
        Binary(BinaryOp),
        Jump(u32),
        JumpUnless(u32),
        Call { name: u32, argc: u16 },
        Return,
        Output,
    }

    #[derive(Debug, Default)]
    pub(super) struct Code {
        pub(super) ops: Vec<Op>,
        constants: Vec<Value>,
        names: Vec<Rc<str>>,
        slot_names: Vec<Rc<str>>,
        params: usize,
    }

    #[derive(Debug)]
    pub(super) struct Compiled {
        pub(super) functions: Vec<Code>,
        index: HashMap<Rc<str>, usize>,
        pub(super) main: Code,
    }

    pub(super) fn lower(program: &Program) -> Compiled {
        Compiled {
            functions: program
                .functions
                .iter()
                .map(|f| function(&f.params, &f.body))
                .collect(),
            index: program
                .functions
                .iter()
                .enumerate()
                .map(|(index, f)| (f.name.clone(), index))
                .collect(),
            main: function(&[], &program.main),
        }
    }

    struct Lowering {
        code: Code,
        resolver: Resolver,
    }

    fn function(params: &[Rc<str>], body: &[Stmt]) -> Code {
        let mut lowering = Lowering {
            code: Code {
                params: params.len(),
                ..Code::default()
            },
            resolver: Resolver::new(params, body),
        };
        for stmt in body {
            lowering.stmt(stmt);
        }
        let null = lowering.constant(Value::Null);
        lowering.emit(Op::Constant(null));
        lowering.emit(Op::Return);
        lowering.code.slot_names = lowering.resolver.names;
        lowering.code
    }

    impl Lowering {
        fn emit(&mut self, op: Op) -> u32 {
            self.code.ops.push(op);
            u32::try_from(self.code.ops.len() - 1).expect("fewer than 2^32 ops")
        }

        fn here(&self) -> u32 {
            u32::try_from(self.code.ops.len()).expect("fewer than 2^32 ops")
        }

        fn constant(&mut self, value: Value) -> u32 {
            self.code.constants.push(value);
            u32::try_from(self.code.constants.len() - 1).expect("fewer than 2^32 constants")
        }

        fn name(&mut self, name: &Rc<str>) -> u32 {
            let index = match self.code.names.iter().position(|n| n == name) {
                Some(index) => index,
                None => {
                    self.code.names.push(name.clone());
                    self.code.names.len() - 1
                }
            };
            u32::try_from(index).expect("fewer than 2^32 names")
        }

        fn read(&mut self, read: Read) {
            let op = match read {
                Read::Slot(slot) => Op::Slot(slot),
                Read::Name(name) => Op::Name(self.name(&name)),
            };
            self.emit(op);
        }

        fn expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Constant(value) => {
                    let index = self.constant(value.clone());
                    self.emit(Op::Constant(index));
                }
                Expr::Variable(name) => {
                    let read = self.resolver.read(name);
                    self.read(read);
                }
                Expr::Binary(op, left, right) => {
                    self.expr(left);
                    self.expr(right);
                    self.emit(Op::Binary(*op));
                }
                Expr::Call(name, args) => {
                    for arg in args {
                        self.expr(arg);
                    }
                    let name = self.name(name);
                    let argc = u16::try_from(args.len()).expect("fewer than 65536 arguments");
                    self.emit(Op::Call { name, argc });
                }
            }
        }

        fn stmt(&mut self, stmt: &Stmt) {
            match stmt {
                Stmt::Assign(name, expr) => {
                    self.expr(expr);
                    let slot = self.resolver.write(name);
                    self.emit(Op::Store(slot));
                }
                Stmt::AddAssign(name, expr) => {
                    let old = self.resolver.read(name);
                    self.read(old);
                    self.expr(expr);
                    self.emit(Op::Binary(BinaryOp::Add));
                    let slot = self.resolver.write(name);
                    self.emit(Op::Store(slot));
                }
                Stmt::Increment(name) => {
                    let old = self.resolver.read(name);
                    self.read(old);
                    let one = self.constant(Value::Int32(1));
                    self.emit(Op::Constant(one));
                    self.emit(Op::Binary(BinaryOp::Add));
                    let slot = self.resolver.write(name);
                    self.emit(Op::Store(slot));
                }
                Stmt::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    self.stmt(init);
                    let top = self.here();
                    self.expr(condition);
                    let exit = self.emit(Op::JumpUnless(0));
                    let before = self.resolver.snapshot();
                    for stmt in body {
                        self.stmt(stmt);
                    }
                    self.stmt(step);
                    self.resolver.restore(before);
                    self.emit(Op::Jump(top));
                    let end = self.here();
                    self.code.ops[exit as usize] = Op::JumpUnless(end);
                }
                Stmt::If { condition, then } => {
                    self.expr(condition);
                    let skip = self.emit(Op::JumpUnless(0));
                    let before = self.resolver.snapshot();
                    for stmt in then {
                        self.stmt(stmt);
                    }
                    self.resolver.restore(before);
                    let end = self.here();
                    self.code.ops[skip as usize] = Op::JumpUnless(end);
                }
                Stmt::Return(expr) => {
                    self.expr(expr);
                    self.emit(Op::Return);
                }
                Stmt::Output(expr) => {
                    self.expr(expr);
                    self.emit(Op::Output);
                }
            }
        }
    }

    struct Machine<'p> {
        program: &'p Compiled,
        stack: Vec<Value>,
        output: Vec<Value>,
    }

    pub(super) fn run(program: &Compiled) -> Outcome<Vec<Value>> {
        let mut machine = Machine {
            program,
            stack: Vec::with_capacity(64),
            output: Vec::new(),
        };
        let main = &program.main;
        let mut frame = Frame::new_root(main.slot_names.len(), &main.slot_names);
        machine.execute(main, &mut frame)?;
        Ok(machine.output)
    }

    impl Machine<'_> {
        fn pop(&mut self) -> Value {
            self.stack.pop().expect("the compiler balances the stack")
        }

        fn execute(&mut self, code: &Code, frame: &mut Frame<'_>) -> Outcome<Value> {
            let mut pc = 0;
            loop {
                match &code.ops[pc] {
                    Op::Constant(index) => self.stack.push(code.constants[*index as usize].clone()),
                    Op::Slot(slot) => self.stack.push(frame.slot(*slot).clone()),
                    Op::Name(index) => self.stack.push(frame.by_name(&code.names[*index as usize])),
                    Op::Store(slot) => {
                        let value = self.pop();
                        frame.slots[usize::from(*slot)] = Some(value);
                    }
                    Op::Binary(op) => {
                        let right = self.pop();
                        let left = self.pop();
                        self.stack.push(ops::apply(*op, &left, &right)?);
                    }
                    Op::Jump(target) => {
                        pc = *target as usize;
                        continue;
                    }
                    Op::JumpUnless(target) => {
                        if !self.pop().is_true() {
                            pc = *target as usize;
                            continue;
                        }
                    }
                    Op::Call { name, argc } => {
                        let program = self.program;
                        let name = &code.names[*name as usize];
                        let &index = program
                            .index
                            .get(name)
                            .ok_or_else(|| Error::UnknownCommand(name.clone()))?;
                        let callee = &program.functions[index];
                        let mut slots = vec![None; callee.slot_names.len()];
                        let base = self.stack.len() - usize::from(*argc);
                        for (position, value) in self.stack.drain(base..).enumerate() {
                            if position < callee.params {
                                slots[position] = Some(value);
                            }
                        }
                        for slot in slots.iter_mut().take(callee.params) {
                            slot.get_or_insert(Value::Null);
                        }
                        let mut callee_frame = Frame {
                            slots,
                            names: &callee.slot_names,
                            parent: Some(&*frame),
                        };
                        let result = self.execute(callee, &mut callee_frame)?;
                        self.stack.push(result);
                    }
                    Op::Return => return Ok(self.pop()),
                    Op::Output => {
                        let value = self.pop();
                        self.output.push(value);
                    }
                }
                pc += 1;
            }
        }
    }
}

// ------------------------------------------------ candidate d: register VM

mod register_vm {
    use super::resolve::{Read, Resolver};
    use super::{
        BinaryOp, Cell, Error, Expr, Frame, HashMap, NULL, Outcome, Program, Rc, Stmt, Value, ops,
    };

    #[derive(Debug, Clone)]
    pub(super) enum Ins {
        Constant {
            dst: u16,
            constant: u32,
        },
        Name {
            dst: u16,
            name: u32,
        },
        Move {
            dst: u16,
            src: u16,
        },
        Binary {
            op: BinaryOp,
            dst: u16,
            left: u16,
            right: u16,
        },
        Jump {
            target: u32,
        },
        JumpUnless {
            condition: u16,
            target: u32,
        },
        /// Arguments are in `args .. args + argc`.
        Call {
            dst: u16,
            name: u32,
            args: u16,
            argc: u16,
        },
        /// `Call` with an inline cache: `cache` indexes `Code::call_caches`.
        CallCached {
            dst: u16,
            name: u32,
            args: u16,
            argc: u16,
            cache: u32,
        },
        Return {
            src: u16,
        },
        Output {
            src: u16,
        },
    }

    /// What a call site remembers: the function a name resolved to, valid
    /// while the machine's generation (bumped whenever a function is
    /// defined or removed) has not moved.
    #[derive(Debug, Clone, Copy)]
    pub(super) struct CallCache {
        generation: u64,
        function: u32,
    }

    #[derive(Debug, Default)]
    pub(super) struct Code {
        pub(super) ins: Vec<Ins>,
        constants: Vec<Value>,
        names: Vec<Rc<str>>,
        slot_names: Vec<Rc<str>>,
        params: usize,
        registers: u16,
        call_caches: Vec<Cell<CallCache>>,
    }

    #[derive(Debug)]
    pub(super) struct Compiled {
        pub(super) functions: Vec<Code>,
        index: HashMap<Rc<str>, usize>,
        pub(super) main: Code,
    }

    pub(super) fn lower(program: &Program, cache_calls: bool) -> Compiled {
        Compiled {
            functions: program
                .functions
                .iter()
                .map(|f| function(&f.params, &f.body, cache_calls))
                .collect(),
            index: program
                .functions
                .iter()
                .enumerate()
                .map(|(index, f)| (f.name.clone(), index))
                .collect(),
            main: function(&[], &program.main, cache_calls),
        }
    }

    struct Lowering {
        code: Code,
        resolver: Resolver,
        /// The first register that is not a variable: temporaries start here.
        locals: u16,
        next_temp: u16,
        cache_calls: bool,
    }

    fn function(params: &[Rc<str>], body: &[Stmt], cache_calls: bool) -> Code {
        let resolver = Resolver::new(params, body);
        let locals = u16::try_from(resolver.names.len()).expect("fewer than 65536 variables");
        let mut lowering = Lowering {
            code: Code {
                params: params.len(),
                ..Code::default()
            },
            resolver,
            locals,
            next_temp: locals,
            cache_calls,
        };
        for stmt in body {
            lowering.stmt(stmt);
        }
        let null = lowering.temp();
        let constant = lowering.constant(Value::Null);
        lowering.emit(Ins::Constant {
            dst: null,
            constant,
        });
        lowering.emit(Ins::Return { src: null });
        lowering.code.registers = lowering.code.registers.max(lowering.next_temp);
        lowering.code.slot_names = lowering.resolver.names;
        lowering.code
    }

    impl Lowering {
        fn emit(&mut self, ins: Ins) -> u32 {
            self.code.ins.push(ins);
            u32::try_from(self.code.ins.len() - 1).expect("fewer than 2^32 instructions")
        }

        fn here(&self) -> u32 {
            u32::try_from(self.code.ins.len()).expect("fewer than 2^32 instructions")
        }

        fn temp(&mut self) -> u16 {
            let register = self.next_temp;
            self.next_temp += 1;
            self.code.registers = self.code.registers.max(self.next_temp);
            register
        }

        fn constant(&mut self, value: Value) -> u32 {
            self.code.constants.push(value);
            u32::try_from(self.code.constants.len() - 1).expect("fewer than 2^32 constants")
        }

        fn name(&mut self, name: &Rc<str>) -> u32 {
            let index = match self.code.names.iter().position(|n| n == name) {
                Some(index) => index,
                None => {
                    self.code.names.push(name.clone());
                    self.code.names.len() - 1
                }
            };
            u32::try_from(index).expect("fewer than 2^32 names")
        }

        /// The register holding the variable's current value: its own slot
        /// when definitely assigned, otherwise a temporary loaded by name.
        fn read(&mut self, read: Read) -> u16 {
            match read {
                Read::Slot(slot) => slot,
                Read::Name(name) => {
                    let dst = self.temp();
                    let name = self.name(&name);
                    self.emit(Ins::Name { dst, name });
                    dst
                }
            }
        }

        /// Evaluates `expr` into `dst` if given, otherwise into whatever
        /// register is convenient, and returns that register.
        fn expr(&mut self, expr: &Expr, dst: Option<u16>) -> u16 {
            match expr {
                Expr::Constant(value) => {
                    let dst = dst.unwrap_or_else(|| self.temp());
                    let constant = self.constant(value.clone());
                    self.emit(Ins::Constant { dst, constant });
                    dst
                }
                Expr::Variable(name) => {
                    let read = self.resolver.read(name);
                    let src = self.read(read);
                    match dst {
                        Some(dst) if dst != src => {
                            self.emit(Ins::Move { dst, src });
                            dst
                        }
                        _ => src,
                    }
                }
                Expr::Binary(op, left, right) => {
                    let left = self.expr(left, None);
                    let right = self.expr(right, None);
                    let dst = dst.unwrap_or_else(|| self.temp());
                    self.emit(Ins::Binary {
                        op: *op,
                        dst,
                        left,
                        right,
                    });
                    dst
                }
                Expr::Call(name, args) => {
                    let base = self.next_temp;
                    for _ in args {
                        self.temp();
                    }
                    for (position, arg) in args.iter().enumerate() {
                        let register = base + u16::try_from(position).expect("few arguments");
                        self.expr(arg, Some(register));
                    }
                    let dst = dst.unwrap_or_else(|| self.temp());
                    let name = self.name(name);
                    let argc = u16::try_from(args.len()).expect("fewer than 65536 arguments");
                    if self.cache_calls {
                        self.code.call_caches.push(Cell::new(CallCache {
                            generation: 0,
                            function: u32::MAX,
                        }));
                        let cache = u32::try_from(self.code.call_caches.len() - 1)
                            .expect("fewer than 2^32 call sites");
                        self.emit(Ins::CallCached {
                            dst,
                            name,
                            args: base,
                            argc,
                            cache,
                        });
                    } else {
                        self.emit(Ins::Call {
                            dst,
                            name,
                            args: base,
                            argc,
                        });
                    }
                    dst
                }
            }
        }

        fn stmt(&mut self, stmt: &Stmt) {
            // Temporaries live for one statement.
            self.next_temp = self.locals;
            match stmt {
                Stmt::Assign(name, expr) => {
                    // Reads on the right happen before the slot counts as
                    // assigned, so resolve the slot afterwards.
                    let value = self.expr(expr, None);
                    let slot = self.resolver.write(name);
                    if value != slot {
                        self.emit(Ins::Move {
                            dst: slot,
                            src: value,
                        });
                    }
                }
                Stmt::AddAssign(name, expr) => {
                    let old = self.resolver.read(name);
                    let left = self.read(old);
                    let right = self.expr(expr, None);
                    let slot = self.resolver.write(name);
                    self.emit(Ins::Binary {
                        op: BinaryOp::Add,
                        dst: slot,
                        left,
                        right,
                    });
                }
                Stmt::Increment(name) => {
                    let old = self.resolver.read(name);
                    let left = self.read(old);
                    let one = self.expr(&Expr::Constant(Value::Int32(1)), None);
                    let slot = self.resolver.write(name);
                    self.emit(Ins::Binary {
                        op: BinaryOp::Add,
                        dst: slot,
                        left,
                        right: one,
                    });
                }
                Stmt::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    self.stmt(init);
                    self.next_temp = self.locals;
                    let top = self.here();
                    let condition = self.expr(condition, None);
                    let exit = self.emit(Ins::JumpUnless {
                        condition,
                        target: 0,
                    });
                    let before = self.resolver.snapshot();
                    for stmt in body {
                        self.stmt(stmt);
                    }
                    self.stmt(step);
                    self.resolver.restore(before);
                    self.emit(Ins::Jump { target: top });
                    let end = self.here();
                    self.code.ins[exit as usize] = Ins::JumpUnless {
                        condition,
                        target: end,
                    };
                }
                Stmt::If { condition, then } => {
                    let condition = self.expr(condition, None);
                    let skip = self.emit(Ins::JumpUnless {
                        condition,
                        target: 0,
                    });
                    let before = self.resolver.snapshot();
                    for stmt in then {
                        self.stmt(stmt);
                    }
                    self.resolver.restore(before);
                    let end = self.here();
                    self.code.ins[skip as usize] = Ins::JumpUnless {
                        condition,
                        target: end,
                    };
                }
                Stmt::Return(expr) => {
                    let src = self.expr(expr, None);
                    self.emit(Ins::Return { src });
                }
                Stmt::Output(expr) => {
                    let src = self.expr(expr, None);
                    self.emit(Ins::Output { src });
                }
            }
        }
    }

    struct Machine<'p> {
        program: &'p Compiled,
        /// Bumped whenever the function table changes; never in these
        /// programs, which define their functions before running.
        generation: u64,
        output: Vec<Value>,
    }

    pub(super) fn run(program: &Compiled) -> Outcome<Vec<Value>> {
        let mut machine = Machine {
            program,
            generation: 1,
            output: Vec::new(),
        };
        let main = &program.main;
        let mut frame = Frame::new_root(usize::from(main.registers), &main.slot_names);
        machine.execute(main, &mut frame)?;
        Ok(machine.output)
    }

    impl Machine<'_> {
        fn lookup(&self, name: &Rc<str>) -> Outcome<usize> {
            self.program
                .index
                .get(name)
                .copied()
                .ok_or_else(|| Error::UnknownCommand(name.clone()))
        }

        fn invoke(
            &mut self,
            index: usize,
            frame: &mut Frame<'_>,
            args: u16,
            argc: u16,
        ) -> Outcome<Value> {
            let program = self.program;
            let callee = &program.functions[index];
            let mut slots = vec![None; usize::from(callee.registers)];
            for (position, slot) in slots.iter_mut().take(callee.params).enumerate() {
                *slot = Some(if position < usize::from(argc) {
                    let register = usize::from(args) + position;
                    frame.slots[register].take().unwrap_or(Value::Null)
                } else {
                    Value::Null
                });
            }
            let mut callee_frame = Frame {
                slots,
                names: &callee.slot_names,
                parent: Some(&*frame),
            };
            self.execute(callee, &mut callee_frame)
        }

        fn execute(&mut self, code: &Code, frame: &mut Frame<'_>) -> Outcome<Value> {
            let mut pc = 0;
            loop {
                match &code.ins[pc] {
                    Ins::Constant { dst, constant } => {
                        frame.slots[usize::from(*dst)] =
                            Some(code.constants[*constant as usize].clone());
                    }
                    Ins::Name { dst, name } => {
                        let value = frame.by_name(&code.names[*name as usize]);
                        frame.slots[usize::from(*dst)] = Some(value);
                    }
                    Ins::Move { dst, src } => {
                        let value = frame.slot(*src).clone();
                        frame.slots[usize::from(*dst)] = Some(value);
                    }
                    Ins::Binary {
                        op,
                        dst,
                        left,
                        right,
                    } => {
                        let value = ops::apply(*op, frame.slot(*left), frame.slot(*right))?;
                        frame.slots[usize::from(*dst)] = Some(value);
                    }
                    Ins::Jump { target } => {
                        pc = *target as usize;
                        continue;
                    }
                    Ins::JumpUnless { condition, target } => {
                        if !frame.slot(*condition).is_true() {
                            pc = *target as usize;
                            continue;
                        }
                    }
                    Ins::Call {
                        dst,
                        name,
                        args,
                        argc,
                    } => {
                        let index = self.lookup(&code.names[*name as usize])?;
                        let value = self.invoke(index, frame, *args, *argc)?;
                        frame.slots[usize::from(*dst)] = Some(value);
                    }
                    Ins::CallCached {
                        dst,
                        name,
                        args,
                        argc,
                        cache,
                    } => {
                        let site = &code.call_caches[*cache as usize];
                        let cached = site.get();
                        let index = if cached.generation == self.generation {
                            cached.function as usize
                        } else {
                            let index = self.lookup(&code.names[*name as usize])?;
                            site.set(CallCache {
                                generation: self.generation,
                                function: u32::try_from(index).expect("few functions"),
                            });
                            index
                        };
                        let value = self.invoke(index, frame, *args, *argc)?;
                        frame.slots[usize::from(*dst)] = Some(value);
                    }
                    Ins::Return { src } => {
                        return Ok(frame.slots[usize::from(*src)]
                            .take()
                            .unwrap_or_else(|| NULL.clone()));
                    }
                    Ins::Output { src } => {
                        let value = frame.slot(*src).clone();
                        self.output.push(value);
                    }
                }
                pc += 1;
            }
        }
    }
}

// ------------------------------------------------------------------- main

fn describe(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| match value {
            Value::Double(number) => format!("{number} (Double)"),
            Value::Int32(number) => format!("{number} (Int32)"),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Runs a candidate once, checks its answer against pwsh's, and returns it.
fn check(candidate: &str, program: &str, outputs: Outcome<Vec<Value>>, expected: &Value) {
    match outputs {
        Ok(values) if values.len() == 1 && &values[0] == expected => {
            println!("{program:<15} {candidate:<24} -> {}", describe(&values));
        }
        Ok(values) => panic!(
            "{candidate} printed {} for {program}; pwsh prints {}",
            describe(&values),
            describe(std::slice::from_ref(expected))
        ),
        Err(error) => panic!("{candidate} failed on {program}: {}", error.message()),
    }
}

fn main() {
    let mut report = Report::new(
        "execution",
        "AST walking, stack bytecode and register IR on loop-sum and Fib 18 (#4)",
    );

    let workloads = [
        (
            "loop-sum",
            loop_sum(LOOP_COUNT),
            // pwsh: Int32 overflow promotes $sum to Double.
            Value::Double(499_999_500_000.0),
        ),
        ("fib-18", fib(FIB_ARGUMENT), Value::Int32(2584)),
    ];

    println!("answers (checked against pwsh before timing):");
    let mut lowered = Vec::new();
    for (label, program, expected) in &workloads {
        let resolved = by_slot::lower(program);
        let stack = stack_vm::lower(program);
        let registers = register_vm::lower(program, false);
        let cached = register_vm::lower(program, true);
        check("ast-names", label, by_name::run(program), expected);
        check("ast-slots", label, by_slot::run(&resolved), expected);
        check("stack-vm", label, stack_vm::run(&stack), expected);
        check("register-vm", label, register_vm::run(&registers), expected);
        check(
            "register-vm+call-cache",
            label,
            register_vm::run(&cached),
            expected,
        );
        lowered.push((label, program, resolved, stack, registers, cached));
    }

    report.header();
    report.fact("size_of Value", size_of::<Value>());
    report.fact("size_of stack-vm Op", size_of::<stack_vm::Op>());
    report.fact("size_of register-vm Ins", size_of::<register_vm::Ins>());
    for (label, program, _, stack, registers, _) in &lowered {
        let stack_ops: usize =
            stack.main.ops.len() + stack.functions.iter().map(|f| f.ops.len()).sum::<usize>();
        let register_ins: usize = registers.main.ins.len()
            + registers
                .functions
                .iter()
                .map(|f| f.ins.len())
                .sum::<usize>();
        report.fact(&format!("{label}: AST nodes"), count_nodes(program));
        report.fact(&format!("{label}: stack-vm ops"), stack_ops);
        report.fact(&format!("{label}: register-vm instructions"), register_ins);
    }

    let runs = iterations(1);
    for (label, program, resolved, stack, registers, cached) in &lowered {
        let group = format!("run {label}");
        report.add(measure(&group, "ast-names", runs, || {
            by_name::run(program).is_ok()
        }));
        report.add(measure(&group, "ast-slots", runs, || {
            by_slot::run(resolved).is_ok()
        }));
        report.add(measure(&group, "stack-vm", runs, || {
            stack_vm::run(stack).is_ok()
        }));
        report.add(measure(&group, "register-vm", runs, || {
            register_vm::run(registers).is_ok()
        }));
        report.add(measure(&group, "register-vm+call-cache", runs, || {
            register_vm::run(cached).is_ok()
        }));
    }

    // Lowering is paid before the first statement runs, for every script,
    // profile and module that is not in the compile cache (#69).
    let large = synthetic(200);
    report.fact("synthetic script: AST nodes", count_nodes(&large));
    let lowerings = iterations(200);
    report.add(measure("lower synthetic", "ast-slots", lowerings, || {
        black_box(by_slot::lower(&large));
    }));
    report.add(measure("lower synthetic", "stack-vm", lowerings, || {
        black_box(stack_vm::lower(&large));
    }));
    report.add(measure("lower synthetic", "register-vm", lowerings, || {
        black_box(register_vm::lower(&large, false));
    }));
    report.add(measure(
        "lower synthetic",
        "register-vm+call-cache",
        lowerings,
        || {
            black_box(register_vm::lower(&large, true));
        },
    ));

    report.save();
}
