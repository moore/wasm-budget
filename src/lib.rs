/*!

[![](https://docs.rs/wasm-snip/badge.svg)](https://docs.rs/wasm-snip/)
[![](https://img.shields.io/crates/v/wasm-snip.svg)](https://crates.io/crates/wasm-snip)
[![](https://img.shields.io/crates/d/wasm-snip.png)](https://crates.io/crates/wasm-snip)
[![Build Status](https://travis-ci.org/rustwasm/wasm-snip.png?branch=master)](https://travis-ci.org/rustwasm/wasm-snip)

`wasm-snip` replaces a WebAssembly function's body with an `unreachable`.

Maybe you know that some function will never be called at runtime, but the
compiler can't prove that at compile time? Snip it! All the functions it
transitively called &mdash; which weren't called by anything else and therefore
could also never be called at runtime &mdash; will get removed too.

Very helpful when shrinking the size of WebAssembly binaries!

This functionality relies on the "name" section being present in the `.wasm`
file, so build with debug symbols:

```toml
[profile.release]
debug = true
```

* [Executable](#executable)
* [Library](#library)
* [License](#license)
* [Contributing](#contributing)

## Executable

To install the `wasm-snip` executable, run

```text
$ cargo install wasm-snip
```

You can use `wasm-snip` to remove the `annoying_space_waster`
function from `input.wasm` and put the new binary in `output.wasm` like this:

```text
$ wasm-snip input.wasm -o output.wasm annoying_space_waster
```

For information on using the `wasm-snip` executable, run

```text
$ wasm-snip --help
```

And you'll get the most up-to-date help text, like:

```text
Replace a wasm function with an `unreachable`.

USAGE:
wasm-snip [FLAGS] [OPTIONS] <input> [--] [function]...

FLAGS:
-h, --help                    Prints help information
--snip-rust-fmt-code          Snip Rust's `std::fmt` and `core::fmt` code.
--snip-rust-panicking-code    Snip Rust's `std::panicking` and `core::panicking` code.
-V, --version                 Prints version information

OPTIONS:
-o, --output <output>         The path to write the output wasm file to. Defaults to stdout.
-p, --pattern <pattern>...    Snip any function that matches the given regular expression.

ARGS:
<input>          The input wasm file containing the function(s) to snip.
<function>...    The specific function(s) to snip. These must match exactly. Use the -p flag for fuzzy matching.
```

## Library

To use `wasm-snip` as a library, add this to your `Cargo.toml`:

```toml
[dependencies.wasm-snip]
# Do not build the executable.
default-features = false
```

See [docs.rs/wasm-snip][docs] for API documentation.

[docs]: https://docs.rs/wasm-snip

## License

Licensed under either of

 * [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)

 * [MIT license](http://opensource.org/licenses/MIT)

at your option.

## Contributing

See
[CONTRIBUTING.md](https://github.com/rustwasm/wasm-snip/blob/master/CONTRIBUTING.md)
for hacking.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

 */

#![deny(missing_docs)]
#![deny(missing_debug_implementations)]

use failure::ResultExt;
use rayon::prelude::*;
use std::path;
use walrus::ir::VisitorMut;

/// Options for controlling which functions in what `.wasm` file should be
/// snipped.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The input `.wasm` file that should have its functions snipped.
    pub input: path::PathBuf,

    /// The functions that should be snipped from the `.wasm` file.
    pub functions: Vec<String>,

    /// The regex patterns whose matches should be snipped from the `.wasm`
    /// file.
    pub patterns: Vec<String>,

    /// Should Rust `std::fmt` and `core::fmt` functions be snipped?
    pub snip_rust_fmt_code: bool,

    /// Should Rust `std::panicking` and `core::panicking` functions be snipped?
    pub snip_rust_panicking_code: bool,
}

/// Snip the functions from the input file described by the options.
pub fn snip(options: Options) -> Result<walrus::Module, failure::Error> {
    let mut module = walrus::Module::from_file(&options.input)
        .with_context(|_| format!("failed to parse wasm from: {}", options.input.display()))?;

    inject_counting(&mut module);
    
    walrus::passes::gc::run(&mut module);

    Ok(module)
}


fn inject_counting(
    module: &mut walrus::Module,
) {
    struct Injector<'a> {
        func: &'a mut walrus::LocalFunction,
        count: i64,
        counter_id: walrus::GlobalId,
        budget_id: walrus::GlobalId,
    }

    
    impl Injector<'_> {
        fn inject ( &mut self, block: &mut walrus::ir::Block ) {

            let count   = block.exprs.len() as i64;
            //println!("count is {:?}", count); //BOOG
            let counter = self.counter_id;
            let budget  = self.budget_id;
            
            let set_op = {
                let builder = self.func.builder_mut();
                
                let get_op      = builder.global_get(counter);
                let count_op    = builder.i64_const(count);
                let add_op      = builder.binop(walrus::ir::BinaryOp::I64Add, get_op, count_op);
                builder.global_set(counter, add_op)
            };

            let if_block = {
                let builder = self.func.builder_mut();

                let budget_op   = builder.global_get(budget);
                let get_op      = builder.global_get(counter);
                let less_op     = builder.binop(walrus::ir::BinaryOp::I64LtS, budget_op, get_op);
                let bail_op     = builder.unreachable();

                let bail_block  = {
                    let mut bail_builder = builder.block(Box::new([]), Box::new([])); 

                    bail_builder.expr(bail_op);
                    bail_builder.id()
                };

                let empty_block = {
                    builder.block(Box::new([]), Box::new([])).id()
                };

                builder.if_else(less_op, bail_block, empty_block)
            };
            
            block.exprs.insert(0, set_op);
            block.exprs.insert(1, if_block);

            
        }
    }
     
    
    impl VisitorMut for Injector<'_> {
        fn local_function_mut(&mut self) -> &mut walrus::LocalFunction {
            self.func
        }


        
        fn visit_expr_id_mut(&mut self, expr_id: &mut walrus::ir::ExprId) {
            use walrus::ir::VisitMut;

            self.count += 1;
            (*expr_id).visit_mut(self);
        }
        

        /*
        fn visit_block_mut(&mut self, expr: &mut walrus::ir::Block) {
            use walrus::ir::VisitMut;
        
            self.inject(expr);

            
            for sub_expr in &mut expr.exprs {
                sub_expr.visit_mut(self);
            }
            

        }
        */
        
    }

    use walrus::ValType::I64;
    use walrus::ir::Value::I64 as I64Val;
    use walrus::InitExpr::Value;
    
    let counter = module.globals.add_local(I64, true, Value(I64Val(0)));
    let budget  = module.globals.add_local(I64, true, Value(I64Val(100)));
    
    module.funcs.par_iter_local_mut().for_each(|(id, func)| {

        let count = {
            let mut entry = func.entry_block();
            let v = &mut Injector { func, count: 0, counter_id: counter, budget_id: budget };
            v.visit_block_id_mut(&mut entry);
            v.count
        };
        
        let set_op = {
            let builder = func.builder_mut();
        
            let get_op      = builder.global_get(counter);
            let count_op    = builder.i64_const(count);
            let add_op      = builder.binop(walrus::ir::BinaryOp::I64Add, get_op, count_op);
            builder.global_set(counter, add_op)
        };

        let if_block = {
            let builder = func.builder_mut();

            let budget_op   = builder.global_get(budget);
            let get_op      = builder.global_get(counter);
            let less_op     = builder.binop(walrus::ir::BinaryOp::I64LtS, budget_op, get_op);
            let bail_op     = builder.unreachable();

            let bail_block  = {
                let mut bail_builder = builder.block(Box::new([]), Box::new([])); 

                bail_builder.expr(bail_op);
                bail_builder.id()
            };

            let empty_block = {
                builder.block(Box::new([]), Box::new([])).id()
            };

            builder.if_else(less_op, bail_block, empty_block)
        };
        
        let entry_id = func.entry_block();
        let block = func.block_mut(entry_id);
        block.exprs.insert(0, set_op);
        block.exprs.insert(1, if_block);
        
        println!("Func {:?} has {:?} ops", id, count);
    });

    
}




