use failure::ResultExt;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path;
use walrus::ir::*;
use walrus::LocalFunction;

/// Options for controlling which functions in what `.wasm` file should be
/// snipped.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The input `.wasm` file that should have its functions snipped.
    pub input: path::PathBuf,

    /// The functions that should be snipped from the `.wasm` file.
    pub functions: Vec<String>,
}

/// Snip the functions from the input file described by the options.
pub fn snip(options: Options) -> Result<walrus::Module, failure::Error> {
    let mut module = walrus::Module::from_file(&options.input)
        .with_context(|_| format!("failed to parse wasm from: {}", options.input.display()))?;

    inject_counting(&mut module);

    Ok(module)
}

fn inject_counting(module: &mut walrus::Module) {
    use walrus::ir::Value::I64 as I64Val;
    use walrus::InitExpr::Value;
    use walrus::ValType::I64;

    let counter = module.globals.add_local(I64, true, Value(I64Val(0)));
    let budget = module.globals.add_local(I64, true, Value(I64Val(100)));

    module.funcs.par_iter_local_mut().for_each(|(_id, func)| {
        let mut map: HashMap<walrus::ir::BlockId, i64> = HashMap::new();

        inject(func, &mut map);

        for (id, count) in map.iter() {
            println!("Block has {:?} ops", count);
            inject_ops(func, counter, budget, *id, *count);
        }
    });
}

fn inject_ops(
    func: &mut LocalFunction,
    counter: walrus::GlobalId,
    budget: walrus::GlobalId,
    block_id: walrus::ir::BlockId,
    count: i64,
) {
    let set_op = {
        let builder = func.builder_mut();

        let get_op = builder.global_get(counter);
        let count_op = builder.i64_const(count);
        let add_op = builder.binop(walrus::ir::BinaryOp::I64Add, get_op, count_op);
        builder.global_set(counter, add_op)
    };

    let if_block = {
        let builder = func.builder_mut();

        let budget_op = builder.global_get(budget);
        let get_op = builder.global_get(counter);
        let less_op = builder.binop(walrus::ir::BinaryOp::I64LtS, budget_op, get_op);
        let bail_op = builder.unreachable();

        let bail_block = {
            let mut bail_builder = builder.block(Box::new([]), Box::new([]));

            bail_builder.expr(bail_op);
            bail_builder.id()
        };

        let empty_block = { builder.block(Box::new([]), Box::new([])).id() };

        builder.if_else(less_op, bail_block, empty_block)
    };

    let block = func.block_mut(block_id);

    block.exprs.insert(0, set_op);
    block.exprs.insert(1, if_block);
}

fn inject(func: &LocalFunction, map: &mut HashMap<walrus::ir::BlockId, i64>) {
    let mut v = Injector {
        func,
        count: 0,
        map: map,
    };
    v.visit(func.entry_block());
}

struct Injector<'a> {
    // The function we are visiting.
    func: &'a LocalFunction,

    // Count of instructions in current block
    count: i64,

    // map of block ids -> expression count
    map: &'a mut HashMap<walrus::ir::BlockId, i64>,
}

impl Injector<'_> {
    fn visit<'a, E: 'a>(&mut self, e: E)
    where
        E: Into<ExprId>,
    {
        self.visit_expr_id(e.into())
    }

    fn visit_expr_id(&mut self, id: ExprId) {
        use self::Expr::*;

        match self.func.get(id) {
            Const(_) => {
                self.count += 1;
            }

            Block(e) => self.visit_block(e, id),
            BrTable(e) => self.visit_br_table(e),
            IfElse(e) => self.visit_if_else(e),

            Drop(e) => {
                self.visit(e.expr);
                self.count += 1;
            }

            Return(e) => {
                for x in e.values.iter() {
                    self.visit(*x);
                }
                self.count += 1;
            }

            WithSideEffects(e) => {
                for x in e.before.iter() {
                    self.visit(*x);
                }
                self.visit(e.value);
                for x in e.after.iter() {
                    self.visit(*x);
                }
            }

            MemorySize(_) => {
                self.count += 1;
            }

            MemoryGrow(e) => {
                self.visit(e.pages);
                self.count += 1;
            }

            MemoryInit(e) => {
                self.visit(e.memory_offset);
                self.visit(e.data_offset);
                self.visit(e.len);
                self.count += 1;
            }

            DataDrop(_) => {
                self.count += 1;
            }

            MemoryCopy(e) => {
                self.visit(e.dst_offset);
                self.visit(e.src_offset);
                self.visit(e.len);
                self.count += 1;
            }

            MemoryFill(e) => {
                self.visit(e.offset);
                self.visit(e.value);
                self.visit(e.len);
                self.count += 1;
            }

            Binop(e) => {
                self.visit(e.lhs);
                self.visit(e.rhs);
                self.count += 1;
            }

            Unop(e) => {
                self.visit(e.expr);
                self.count += 1;
            }

            Select(e) => {
                self.visit(e.alternative);
                self.visit(e.consequent);
                self.visit(e.condition);
                self.count += 1;
            }

            Unreachable(_) => {
                // no charge for this
            }

            Br(e) => {
                for x in e.args.iter() {
                    self.visit(*x);
                }
                self.count += 1;
            }

            BrIf(e) => {
                for x in e.args.iter() {
                    self.visit(*x);
                }
                self.visit(e.condition);
                self.count += 1;
            }

            Call(e) => {
                for x in e.args.iter() {
                    self.visit(*x);
                }
                self.count += 1;
            }

            CallIndirect(e) => {
                for x in e.args.iter() {
                    self.visit(*x);
                }
                self.visit(e.func);
                self.count += 1;
            }

            LocalGet(_) => {
                self.count += 1;
            }

            LocalSet(e) => {
                self.visit(e.value);
                self.count += 1;
            }

            LocalTee(e) => {
                self.visit(e.value);
                self.count += 1;
            }

            GlobalGet(_) => {
                self.count += 1;
            }

            GlobalSet(e) => {
                self.visit(e.value);
                self.count += 1;
            }

            Load(e) => {
                self.visit(e.address);
                self.count += 1;
            }

            Store(e) => {
                self.visit(e.address);
                self.visit(e.value);
                self.count += 1;
            }

            AtomicRmw(e) => {
                self.visit(e.address);
                self.visit(e.value);
                self.count += 1;
            }

            Cmpxchg(e) => {
                self.visit(e.address);
                self.visit(e.expected);
                self.visit(e.replacement);
                self.count += 1;
            }

            AtomicNotify(e) => {
                self.visit(e.address);
                self.visit(e.count);
                self.count += 1;
            }

            AtomicWait(e) => {
                self.visit(e.address);
                self.visit(e.expected);
                self.visit(e.timeout);
                self.count += 1;
            }

            TableGet(e) => {
                self.visit(e.index);
                self.count += 1;
            }
            TableSet(e) => {
                self.visit(e.index);
                self.visit(e.value);
                self.count += 1;
            }
            TableGrow(e) => {
                self.visit(e.amount);
                self.visit(e.value);
                self.count += 1;
            }
            TableSize(_) => {
                self.count += 1;
            }
            RefNull(_) => {
                self.count += 1;
            }
            RefIsNull(e) => {
                self.visit(e.value);
                self.count += 1;
            }
        }
    }

    fn visit_block(&mut self, e: &Block, id: ExprId) {
        self.count += 1;

        let save = self.count;
        self.count = 0;

        for x in &e.exprs {
            self.visit(*x);
        }

        self.map.insert(walrus::ir::BlockId::new(id), self.count);

        self.count = save;
    }

    fn visit_if_else(&mut self, e: &IfElse) {
        self.visit(e.condition);
        self.visit(e.consequent);
        self.visit(e.alternative);

        self.count += 3; // If, the, else
    }

    fn visit_br_table(&mut self, e: &BrTable) {
        for x in e.args.iter() {
            self.visit(*x);
        }
        self.visit(e.which);
        self.count += 2; // table + default

        for _b in e.blocks.iter() {
            self.count += 1;
        }
    }
}
