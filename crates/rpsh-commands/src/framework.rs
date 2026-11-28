//! Built-in cmdlets construct an instance per invocation. A runtime controls
//! output enumeration and decides how to route non-terminating error records.
use crate::signature::{Signature, WRITE_OUTPUT};
use rpsh_runtime::{Value, ops};

#[derive(Debug, Default)]
pub struct BoundParameters(Vec<(&'static str, Value)>);

impl BoundParameters {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
    pub fn set(&mut self, name: &'static str, value: Value) {
        if let Some((_, slot)) = self.0.iter_mut().find(|(n, _)| *n == name) {
            *slot = value;
        } else {
            self.0.push((name, value));
        }
    }
}

pub trait CommandRuntime<E> {
    fn write_object(&mut self, value: Value, enumerate: bool) -> Result<(), E>;
    fn write_error(&mut self, record: Value) -> Result<(), E>;
}

pub trait Cmdlet<E> {
    fn begin(
        &mut self,
        _args: &BoundParameters,
        _runtime: &mut dyn CommandRuntime<E>,
    ) -> Result<(), E> {
        Ok(())
    }
    fn process(
        &mut self,
        args: &BoundParameters,
        runtime: &mut dyn CommandRuntime<E>,
    ) -> Result<(), E>;
    fn end(&mut self, _runtime: &mut dyn CommandRuntime<E>) -> Result<(), E> {
        Ok(())
    }
    fn stop(&mut self, _runtime: &mut dyn CommandRuntime<E>) {}
}

#[derive(Debug)]
pub struct Registration<E: 'static> {
    pub signature: &'static Signature,
    pub implementation: &'static str,
    pub construct: fn() -> Box<dyn Cmdlet<E>>,
}

impl<E: 'static> Registration<E> {
    pub const BUILTINS: &'static [Self] = &[Self {
        signature: &WRITE_OUTPUT,
        implementation: "Microsoft.PowerShell.Commands.WriteOutputCommand",
        construct: || Box::new(WriteOutput),
    }];

    pub fn lookup(name: &str) -> Option<&'static Self> {
        Self::BUILTINS
            .iter()
            .find(|r| r.signature.name.eq_ignore_ascii_case(name))
    }
}

#[derive(crate::CmdletParameters)]
#[parameter(InputObject: PsObject, position(0), mandatory, pipeline, remaining, allow_null, allow_empty_collection)]
#[parameter(NoEnumerate: Switch)]
pub(crate) struct WriteOutput;

impl<E> Cmdlet<E> for WriteOutput {
    fn process(
        &mut self,
        args: &BoundParameters,
        runtime: &mut dyn CommandRuntime<E>,
    ) -> Result<(), E> {
        let input = args.get("InputObject").expect("mandatory input was bound");
        let enumerate = !args.get("NoEnumerate").is_some_and(ops::is_true);
        runtime.write_object(input.clone(), enumerate)
    }
}
