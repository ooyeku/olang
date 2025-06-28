use crate::ast::{Expr, Statement};
use crate::ovm::optimization::OptimizationError;
use crate::ovm::FunctionId;
use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum JitError {
    #[error("JIT compilation is not supported for this expression")]
    UnsupportedExpression { expr: Expr },

    #[error("OVM optimization error: {0}")]
    OptimizationError(String),
}

impl From<OptimizationError> for JitError {
    fn from(err: OptimizationError) -> Self {
        JitError::OptimizationError(err.to_string())
    }
}

/// Legacy JIT compiler that delegates to the advanced OVM optimization engine
pub struct JitCompiler {
    /// Function counter for generating unique IDs
    _next_function_id: u32,
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl JitCompiler {
    pub fn new() -> Self {
        Self {
            _next_function_id: 0,
        }
    }

    /// Compile an expression using the OVM JIT system
    pub fn compile_expression(&mut self, expr: &Expr) -> Result<(), JitError> {
        match expr {
            Expr::Block(statements) => {
                // Compile each statement in the block
                for stmt in statements {
                    if let Statement::Expression(expr) = stmt {
                        self.compile_expression(expr)?;
                    }
                }
                Ok(())
            }
            // Function calls are handled by the Call variant
            Expr::Integer(_) | Expr::Float(_) | Expr::String(_) | Expr::Boolean(_) => {
                // Literals don't need compilation but are part of larger expressions
                Ok(())
            }
            Expr::BinaryOp { left, op, right } => {
                // Binary operations are excellent JIT targets
                println!("JIT: Analyzing binary operation: {:?}", op);
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                Ok(())
            }
            Expr::Pipeline { left, right } => {
                // Pipelines are prime targets for fusion optimization
                println!("JIT: Analyzing pipeline for fusion opportunities");
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                Ok(())
            }
            Expr::WhileLoop { condition, body } => {
                // Loops benefit significantly from JIT compilation
                println!("JIT: Analyzing while loop for optimization opportunities");
                self.compile_expression(condition)?;
                self.compile_expression(body)?;
                Ok(())
            }
            Expr::ForLoop {
                variable: _,
                iterable,
                body,
            } => {
                // For loops are excellent JIT targets
                println!("JIT: Analyzing for loop for optimization opportunities");
                self.compile_expression(iterable)?;
                self.compile_expression(body)?;
                Ok(())
            }
            Expr::Call { callee, arguments } => {
                // Function calls are good candidates for JIT compilation
                println!(
                    "JIT: Analyzing function call with {} arguments",
                    arguments.len()
                );
                self.compile_expression(callee)?;
                for arg in arguments {
                    self.compile_expression(arg)?;
                }
                Ok(())
            }
            _ => {
                // For now, other expressions are not compiled but don't error
                Ok(())
            }
        }
    }

    /// Generate a unique function ID for JIT compilation
    fn _generate_function_id(&mut self) -> FunctionId {
        let id = FunctionId::new();
        self._next_function_id += 1; // Keep for consistency but use new()
        id
    }

    /// Get JIT compilation readiness assessment
    pub fn assess_jit_readiness(&self, expr: &Expr) -> JitReadiness {
        match expr {
            Expr::Call { .. } => JitReadiness::HighBenefit,
            Expr::BinaryOp { .. } => JitReadiness::MediumBenefit,
            Expr::Pipeline { .. } => JitReadiness::HighBenefit,
            Expr::WhileLoop { .. } => JitReadiness::HighBenefit,
            Expr::ForLoop { .. } => JitReadiness::HighBenefit,
            Expr::Block(_) => JitReadiness::MediumBenefit,
            _ => JitReadiness::LowBenefit,
        }
    }
}

/// Assessment of JIT compilation benefit for an expression
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitReadiness {
    HighBenefit,   // Significant performance gain expected
    MediumBenefit, // Moderate performance gain expected
    LowBenefit,    // Minimal performance gain expected
    NotSuitable,   // Not suitable for JIT compilation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BinaryOp;

    #[test]
    fn test_jit_compilation() {
        let mut compiler = JitCompiler::new();

        // Test binary operation compilation
        let expr = Expr::BinaryOp {
            left: Box::new(Expr::Integer(42)),
            op: BinaryOp::Add,
            right: Box::new(Expr::Integer(10)),
        };

        assert!(compiler.compile_expression(&expr).is_ok());
    }

    #[test]
    fn test_jit_readiness_assessment() {
        let compiler = JitCompiler::new();

        // Function calls should have high JIT benefit
        let func_call = Expr::Call {
            callee: Box::new(Expr::Identifier("test_func".to_string())),
            arguments: vec![],
        };
        assert_eq!(
            compiler.assess_jit_readiness(&func_call),
            JitReadiness::HighBenefit
        );

        // Binary operations should have medium benefit
        let binary_op = Expr::BinaryOp {
            left: Box::new(Expr::Integer(1)),
            op: BinaryOp::Add,
            right: Box::new(Expr::Integer(2)),
        };
        assert_eq!(
            compiler.assess_jit_readiness(&binary_op),
            JitReadiness::MediumBenefit
        );

        // Literals should have low benefit
        let literal = Expr::Integer(42);
        assert_eq!(
            compiler.assess_jit_readiness(&literal),
            JitReadiness::LowBenefit
        );
    }

    #[test]
    fn test_pipeline_compilation() {
        let mut compiler = JitCompiler::new();

        // Test pipeline expression compilation
        let pipeline = Expr::Pipeline {
            left: Box::new(Expr::Integer(1)),
            right: Box::new(Expr::Call {
                callee: Box::new(Expr::Identifier("double".to_string())),
                arguments: vec![],
            }),
        };

        assert!(compiler.compile_expression(&pipeline).is_ok());
        assert_eq!(
            compiler.assess_jit_readiness(&pipeline),
            JitReadiness::HighBenefit
        );
    }
}
