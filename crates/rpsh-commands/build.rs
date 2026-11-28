use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use std::fmt::Write as _;

fn inventory(name: &str) -> String {
    let path = format!("../../inventory/{name}.tsv");
    println!("cargo:rerun-if-changed={path}");
    fs::read_to_string(path).expect("read the checked-in PowerShell inventory")
}

fn cmdlet_names(commands: &str, module: &str) -> Vec<String> {
    let mut cmdlets = BTreeSet::new();
    for line in commands.lines().skip(1) {
        let columns: Vec<_> = line.split('\t').collect();
        if let [owner, name, "Cmdlet", "linux-x64", _] = columns[..]
            && owner == module
        {
            cmdlets.insert(name.to_ascii_lowercase());
        }
    }
    cmdlets.into_iter().collect()
}

fn main() {
    let commands = inventory("commands");
    let aliases = inventory("aliases");
    let mut output = String::from("pub static MODULES: &[BuiltinModule] = &[\n");
    for module in [
        "Microsoft.PowerShell.Management",
        "Microsoft.PowerShell.Utility",
    ] {
        let cmdlets = cmdlet_names(&commands, module);
        let mut exports = BTreeSet::new();
        for line in aliases.lines().skip(1) {
            let columns: Vec<_> = line.split('\t').collect();
            if let [name, definition, owner, "linux-x64", _] = columns[..]
                && owner == module
            {
                exports.insert((name, definition));
            }
        }
        let exports: Vec<_> = exports.into_iter().collect();
        writeln!(output, "BuiltinModule {{ name: {module:?}, version: \"7.0.0.0\", cmdlets: &{cmdlets:?}, aliases: &{exports:?} }},").unwrap();
    }
    output.push_str("];\n");
    let core = cmdlet_names(&commands, "Microsoft.PowerShell.Core");
    writeln!(output, "pub static CORE_CMDLETS: &[&str] = &{core:?};").unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    fs::write(out.join("modules.rs"), output).expect("write the built-in module catalog");
}
