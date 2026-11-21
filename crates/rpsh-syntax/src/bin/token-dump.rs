//! Dumps `tokenize_full` as `kind<TAB>start<TAB>end`, one token per line.
//! Used by scripts/token-diff.py to compare with the reference tokenizer.

use std::io::Read;

fn main() {
    let mut source = String::new();
    std::io::stdin()
        .read_to_string(&mut source)
        .expect("token-dump: stdin is not UTF-8");
    for token in rpsh_syntax::tokenize_full(&source).expect("tokenize") {
        println!(
            "{}\t{}\t{}",
            token.kind.name(),
            token.start_char,
            token.end_char
        );
    }
}
