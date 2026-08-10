//! Ad-hoc speed comparison: interpreter vs bytecode tier on the same function.
use olang::ast::{Statement, Value};
use olang::ovm::bytecode::BytecodeVm;
use olang::ovm::{FunctionId, OvmValue};
use olang::{Interpreter, Parser};
use std::time::Instant;

fn main() {
    let cases = [
        (
            "fib",
            "fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)",
            vec![Value::Integer(20)],
        ),
        (
            "sum_to",
            "fn sum_to(n) = {\n let total = 0\n let i = 0\n while i <= n {\n total = total + i\n i = i + 1\n }\n total\n}",
            vec![Value::Integer(100000)],
        ),
    ];

    for (name, src, args) in cases {
        let parser = Parser::new();
        let program = parser.parse(src).unwrap();

        // Interpreter
        let mut interpreter = Interpreter::new();
        interpreter.eval_program(program.clone()).unwrap();
        let callee = interpreter
            .get_user_variables()
            .get(name)
            .cloned()
            .cloned()
            .unwrap();
        let start = Instant::now();
        let interp_val = interpreter.call_function(callee, args.clone()).unwrap();
        let interp_time = start.elapsed();

        // Bytecode VM
        let mut vm = BytecodeVm::new();
        let mut id = None;
        for st in &program.statements {
            if let Statement::FunctionDecl(f) = st {
                let fid = FunctionId::new();
                vm.register_function(f.name.clone(), fid);
                vm.compile_function(fid, f).unwrap();
                id = Some(fid);
            }
        }
        let ovm_args: Vec<OvmValue> = args.iter().map(|v| OvmValue::from_ast(v.clone())).collect();
        let start = Instant::now();
        let vm_val = vm
            .execute(id.unwrap(), &ovm_args)
            .unwrap()
            .to_ast()
            .unwrap();
        let vm_time = start.elapsed();

        assert_eq!(interp_val, vm_val, "results must agree");
        println!(
            "{}: interpreter {:?}, bytecode {:?} ({:.1}x)",
            name,
            interp_time,
            vm_time,
            interp_time.as_secs_f64() / vm_time.as_secs_f64()
        );
    }
}
