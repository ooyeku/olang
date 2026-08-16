//! Slot resolution: rewrite identifiers in function bodies to frame-slot
//! references at declaration time, so the hot path indexes instead of
//! probing names.
//!
//! ## Contract
//!
//! Resolution is an optimization, never a semantics change. Every resolved
//! reference keeps its name, and the runtime verifies the slot holds that
//! name before using it, falling back to an ordinary lookup otherwise
//! (`Environment::get_slot` / `set_slot`). A stale or wrong static model
//! therefore costs speed, not correctness.
//!
//! ## Scope model
//!
//! The resolver mirrors the evaluator's environment topology exactly:
//!
//! - A function call creates one frame holding, in push order: the
//!   function's own name (if named), then its parameters, then every
//!   `let`/binding executed directly in the body.
//! - A block that binds anything creates a child frame (depth +1) for the
//!   duration of the block; a block that binds nothing runs in the
//!   enclosing frame and adds no depth. This mirrors the evaluator's
//!   `statement_binds` test exactly — the two must not drift.
//! - `for` loops, `match` arms, and `try/catch` catch blocks each create a
//!   child frame (depth +1) for the duration of their body.
//!
//! Slots are assigned in textual order. A `let` inside a conditional can
//! shift the runtime indices of later pushes; those references then fail
//! the name check at the slot and fall back — correct, just unresolved.
//! Bindings whose runtime position is not knowable this way (a name bound
//! more than once at different positions, destructuring patterns) simply
//! stay as `Identifier`.

use crate::ast::{Argument, Expr, MatchArm, Pattern, Statement, TemplatePart};

/// Per-frame resolver scope: names in push order, mirroring runtime locals.
struct Scope {
    slots: Vec<String>,
    /// Names whose slot position became uncertain (bound inside a
    /// conditional, or shadow-rebound): resolvable only via fallback, so we
    /// stop resolving them.
    poisoned: Vec<String>,
    /// Inside a conditional construct (if branch, while body): new `let`s
    /// get poisoned slots because their runtime push is not guaranteed.
    conditional_depth: u32,
}

impl Scope {
    fn new() -> Self {
        Scope {
            slots: Vec::new(),
            poisoned: Vec::new(),
            conditional_depth: 0,
        }
    }

    fn bind(&mut self, name: &str) {
        if self.conditional_depth > 0 {
            // Runtime push not guaranteed → every later slot in this frame
            // is uncertain too. Poison the name; leave existing slots
            // (already-pushed ones keep their indices).
            if !self.poisoned.iter().any(|n| n == name) {
                self.poisoned.push(name.to_string());
            }
            return;
        }
        if self.poisoned.iter().any(|n| n == name) {
            return;
        }
        if !self.slots.iter().any(|n| n == name) {
            // A rebind of an existing name overwrites its slot in place at
            // runtime (define()'s overwrite path), so only first binds push
            self.slots.push(name.to_string());
        }
    }

    fn lookup(&self, name: &str) -> Option<u16> {
        if self.poisoned.iter().any(|n| n == name) {
            return None;
        }
        self.slots
            .iter()
            .position(|n| n == name)
            .map(|index| index as u16)
    }
}

pub struct Resolver {
    /// Innermost frame last. depth = frames.len() - 1 - index.
    frames: Vec<Scope>,
}

impl Resolver {
    /// Resolve a function body given its parameter names and (for named
    /// functions) its own name, which the call frame binds for recursion.
    pub fn resolve_function_body(
        body: &Expr,
        function_name: Option<&str>,
        parameters: &[String],
    ) -> Expr {
        let mut scope = Scope::new();
        if let Some(name) = function_name {
            scope.bind(name);
        }
        for param in parameters {
            scope.bind(param);
        }
        let mut resolver = Resolver {
            frames: vec![scope],
        };
        resolver.resolve_expr(body)
    }

    fn current(&mut self) -> &mut Scope {
        self.frames.last_mut().expect("resolver has no frame")
    }

    /// Find (depth, slot) for a name, innermost frame first.
    fn lookup(&self, name: &str) -> Option<(u16, u16)> {
        for (hops, scope) in self.frames.iter().rev().enumerate() {
            if let Some(slot) = scope.lookup(name) {
                return Some((hops as u16, slot));
            }
            // A poisoned name in an inner frame may still shadow an outer
            // slot at runtime; resolving to the outer slot would then read
            // the wrong binding's... name — which the runtime check catches,
            // but don't resolve through a known-uncertain shadow.
            if scope.poisoned.iter().any(|n| n == name) {
                return None;
            }
        }
        None
    }

    fn resolve_expr(&mut self, expr: &Expr) -> Expr {
        match expr {
            Expr::Identifier(name) => match self.lookup(name) {
                Some((depth, slot)) => Expr::LocalRef {
                    name: name.clone(),
                    depth,
                    slot,
                },
                None => expr.clone(),
            },

            Expr::Assignment { target, value } => {
                let value = Box::new(self.resolve_expr(value));
                match self.lookup(target) {
                    Some((depth, slot)) => Expr::LocalAssign {
                        name: target.clone(),
                        depth,
                        slot,
                        value,
                    },
                    None => Expr::Assignment {
                        target: target.clone(),
                        value,
                    },
                }
            }

            // Already resolved (shouldn't occur on parse output; identity)
            Expr::LocalRef { .. } | Expr::LocalAssign { .. } => expr.clone(),

            // A block that binds gets its own frame, exactly as the
            // evaluator gives it one (`Interpreter::statement_binds`). A
            // block that binds nothing runs in the enclosing frame, and
            // opening one here would put every inner reference one hop
            // too deep.
            Expr::Block(statements) => {
                let binds = statements.iter().any(Self::statement_binds);
                if binds {
                    self.frames.push(Scope::new());
                }
                let resolved = statements
                    .iter()
                    .map(|statement| self.resolve_statement(statement))
                    .collect();
                if binds {
                    self.frames.pop();
                }
                Expr::Block(resolved)
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition = Box::new(self.resolve_expr(condition));
                // Branch bodies execute conditionally: `let`s inside them
                // have unguaranteed runtime pushes
                self.current().conditional_depth += 1;
                let then_branch = Box::new(self.resolve_expr(then_branch));
                let else_branch = else_branch.as_ref().map(|e| Box::new(self.resolve_expr(e)));
                self.current().conditional_depth -= 1;
                Expr::If {
                    condition,
                    then_branch,
                    else_branch,
                }
            }

            Expr::WhileLoop { condition, body } => {
                let condition = Box::new(self.resolve_expr(condition));
                // Zero iterations is possible; first-iteration pushes are
                // conditional from a static viewpoint
                self.current().conditional_depth += 1;
                let body = Box::new(self.resolve_expr(body));
                self.current().conditional_depth -= 1;
                Expr::WhileLoop { condition, body }
            }

            Expr::Loop { body } => {
                // Executes at least once, but `break` can cut a first
                // iteration short mid-body; treat as conditional
                self.current().conditional_depth += 1;
                let body = Box::new(self.resolve_expr(body));
                self.current().conditional_depth -= 1;
                Expr::Loop { body }
            }

            Expr::ForLoop {
                variable,
                iterable,
                body,
            } => {
                let iterable = Box::new(self.resolve_expr(iterable));
                // The evaluator creates a child frame; the loop variable is
                // its first push
                let mut scope = Scope::new();
                scope.bind(variable);
                self.frames.push(scope);
                let body = Box::new(self.resolve_expr(body));
                self.frames.pop();
                Expr::ForLoop {
                    variable: variable.clone(),
                    iterable,
                    body,
                }
            }

            Expr::Match { value, arms } => {
                let value = Box::new(self.resolve_expr(value));
                let arms = arms
                    .iter()
                    .map(|arm| {
                        // Each arm body runs in a child frame holding the
                        // pattern bindings. Binding push order inside the
                        // evaluator follows a HashMap's iteration order, so
                        // slots for them are NOT statically knowable — open
                        // the frame with every binding poisoned. References
                        // to outer frames still resolve through it at the
                        // correct depth.
                        let mut scope = Scope::new();
                        let mut names = std::collections::HashSet::new();
                        Self::pattern_names(&arm.pattern, &mut names);
                        for name in names {
                            scope.poisoned.push(name);
                        }
                        self.frames.push(scope);
                        let guard = arm.guard.as_ref().map(|g| Box::new(self.resolve_expr(g)));
                        let expression = self.resolve_expr(&arm.expression);
                        self.frames.pop();
                        MatchArm {
                            pattern: arm.pattern.clone(),
                            guard,
                            expression,
                        }
                    })
                    .collect();
                Expr::Match { value, arms }
            }

            Expr::Lambda {
                parameters,
                body,
                return_type,
            } => {
                // A lambda body runs in its own future call frame; its free
                // variables resolve from its captured closure by name. Do
                // not resolve into it from here — it is resolved against its
                // own frame when the lambda value is created at runtime.
                Expr::Lambda {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    return_type: return_type.clone(),
                }
            }

            // ---- Structural recursion for everything else ----
            Expr::Call { callee, arguments } => Expr::Call {
                callee: Box::new(self.resolve_expr(callee)),
                arguments: arguments
                    .iter()
                    .map(|argument| match argument {
                        Argument::Positional(e) => Argument::Positional(self.resolve_expr(e)),
                        Argument::Named { name, value } => Argument::Named {
                            name: name.clone(),
                            value: self.resolve_expr(value),
                        },
                    })
                    .collect(),
            },

            Expr::Pipeline { left, right } => Expr::Pipeline {
                left: Box::new(self.resolve_expr(left)),
                right: Box::new(self.resolve_expr(right)),
            },

            Expr::BinaryOp { left, op, right } => Expr::BinaryOp {
                left: Box::new(self.resolve_expr(left)),
                op: op.clone(),
                right: Box::new(self.resolve_expr(right)),
            },

            Expr::BitwiseOp { left, op, right } => Expr::BitwiseOp {
                left: Box::new(self.resolve_expr(left)),
                op: op.clone(),
                right: Box::new(self.resolve_expr(right)),
            },

            Expr::UnaryOp { op, operand } => Expr::UnaryOp {
                op: op.clone(),
                operand: Box::new(self.resolve_expr(operand)),
            },

            Expr::List(items) => Expr::List(
                items
                    .iter()
                    .map(|item| self.resolve_expr(item))
                    .collect::<Vec<_>>()
                    .into(),
            ),

            Expr::Tuple(items) => Expr::Tuple(
                items
                    .iter()
                    .map(|item| self.resolve_expr(item))
                    .collect::<Vec<_>>()
                    .into(),
            ),

            Expr::Range {
                start,
                end,
                inclusive,
            } => Expr::Range {
                start: Box::new(self.resolve_expr(start)),
                end: Box::new(self.resolve_expr(end)),
                inclusive: *inclusive,
            },

            Expr::Index { object, index } => Expr::Index {
                object: Box::new(self.resolve_expr(object)),
                index: Box::new(self.resolve_expr(index)),
            },

            Expr::FieldAccess { object, field } => Expr::FieldAccess {
                object: Box::new(self.resolve_expr(object)),
                field: field.clone(),
            },

            Expr::ResultOk(inner) => Expr::ResultOk(Box::new(self.resolve_expr(inner))),
            Expr::ResultErr(inner) => Expr::ResultErr(Box::new(self.resolve_expr(inner))),
            Expr::Try(inner) => Expr::Try(Box::new(self.resolve_expr(inner))),

            Expr::TemplateString { parts } => Expr::TemplateString {
                parts: parts
                    .iter()
                    .map(|part| match part {
                        TemplatePart::Literal(text) => TemplatePart::Literal(text.clone()),
                        TemplatePart::Interpolation(e) => {
                            TemplatePart::Interpolation(Box::new(self.resolve_expr(e)))
                        }
                    })
                    .collect(),
            },

            // Leaves and forms whose sub-expressions don't reference frame
            // locals in resolvable ways: clone as-is. This is the safe
            // default — an unresolved identifier just probes by name.
            other => other.clone(),
        }
    }

    fn resolve_statement(&mut self, statement: &Statement) -> Statement {
        match statement {
            Statement::Expression(e) => Statement::Expression(self.resolve_expr(e)),
            Statement::LetDecl(decl) => {
                let value = decl.value.as_ref().map(|v| self.resolve_expr(v));
                // Bind after resolving the value: `let x = x + 1` reads the
                // outer x
                match &decl.pattern {
                    Pattern::Identifier(name) => self.current().bind(name),
                    other => {
                        // Destructuring binds via a HashMap whose push order
                        // is not statically knowable — poison all names
                        let mut names = std::collections::HashSet::new();
                        Self::pattern_names(other, &mut names);
                        for name in names {
                            let scope = self.current();
                            if !scope.poisoned.contains(&name) {
                                scope.poisoned.push(name);
                            }
                        }
                    }
                }
                let mut resolved = decl.clone();
                resolved.value = value;
                Statement::LetDecl(resolved)
            }
            // Nested function declarations bind their name in this frame,
            // and their bodies are resolved when declared at runtime
            Statement::FunctionDecl(decl) => {
                self.current().bind(&decl.name);
                Statement::FunctionDecl(decl.clone())
            }
            other => other.clone(),
        }
    }

    /// Does this statement introduce a binding? Must agree exactly with
    /// `Interpreter::statement_binds`, which decides whether the evaluator
    /// opens a frame for the block containing it.
    fn statement_binds(statement: &Statement) -> bool {
        match statement {
            Statement::Located { stmt, .. } => Self::statement_binds(stmt),
            Statement::LetDecl(_)
            | Statement::FunctionDecl(_)
            | Statement::UseDecl(_)
            | Statement::ShareDecl(_) => true,
            _ => false,
        }
    }

    fn pattern_names(pattern: &Pattern, names: &mut std::collections::HashSet<String>) {
        match pattern {
            Pattern::Identifier(name) | Pattern::Rest(name) => {
                names.insert(name.clone());
            }
            Pattern::Ok(inner) | Pattern::Err(inner) => Self::pattern_names(inner, names),
            Pattern::Tuple(patterns) => {
                for p in patterns {
                    Self::pattern_names(p, names);
                }
            }
            Pattern::List { patterns, rest } => {
                for p in patterns {
                    Self::pattern_names(p, names);
                }
                if let Some(rest_name) = rest {
                    names.insert(rest_name.clone());
                }
            }
            Pattern::Or { alternatives } => {
                for p in alternatives {
                    Self::pattern_names(p, names);
                }
            }
            Pattern::Guarded { pattern, .. } => Self::pattern_names(pattern, names),
            Pattern::EnumVariant { patterns, .. } => {
                for p in patterns {
                    Self::pattern_names(p, names);
                }
            }
            Pattern::Struct { field_patterns, .. }
            | Pattern::AnonymousStruct { field_patterns } => {
                for (_, p) in field_patterns {
                    Self::pattern_names(p, names);
                }
            }
            Pattern::Literal(_) | Pattern::Wildcard | Pattern::Range { .. } => {}
        }
    }
}
