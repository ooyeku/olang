//! Pattern matching: the single matcher used by `match` arms, `let`
//! destructuring, and function-call binding. Binds into the provided map on
//! success; a failed match binds nothing.

use super::{Interpreter, InterpreterError};
use crate::ast::{EnumVariantData, Pattern, Value};
use std::collections::HashMap;

impl Interpreter {
    pub(crate) fn pattern_matches_bind(
        &self,
        pattern: &Pattern,
        value: &Value,
        bindings: &mut HashMap<String, Value>,
    ) -> Result<bool, InterpreterError> {
        match (pattern, value) {
            (Pattern::Literal(lit), val) => Ok(lit == val),
            (Pattern::Identifier(name), val) => {
                // A bare name that is a *declared* unit enum variant is a
                // variant pattern — match it by equality — rather than a fresh
                // binding that captures everything. Without this,
                // `match dir { North => .., East => .. }` would have `North`
                // bind and shadow every other arm. The declared-variant check
                // (not "does some in-scope value happen to be a unit variant")
                // is essential: a binding sub-pattern such as `b` in
                // `Concat(a, b)` must still bind even when a `b` already in
                // scope holds a unit variant.
                if self.unit_variant_names.contains(name)
                    && let Some(variant @ Value::Enum { .. }) = self.environment.get(name)
                    && matches!(
                        variant,
                        Value::Enum {
                            variant_data: EnumVariantData::Unit,
                            ..
                        }
                    )
                {
                    return Ok(&variant == val);
                }
                bindings.insert(name.clone(), val.clone());
                Ok(true)
            }
            (Pattern::Wildcard, _) => Ok(true),
            (Pattern::List { patterns, rest }, Value::List(values)) => {
                if let Some(rest_name) = rest {
                    // Rest pattern: [a, b, ...rest]
                    if patterns.len() > values.len() {
                        return Ok(false); // Not enough values for required patterns
                    }

                    // Match the explicit patterns
                    for (i, pattern) in patterns.iter().enumerate() {
                        if !self.pattern_matches_bind(pattern, &values[i], bindings)? {
                            return Ok(false);
                        }
                    }

                    // Bind the rest of the values to the rest variable
                    let rest_values: Vec<Value> = values[patterns.len()..].to_vec();
                    bindings.insert(rest_name.clone(), Value::List(rest_values.into()));
                    Ok(true)
                } else {
                    // No rest pattern: exact length match required
                    if patterns.len() != values.len() {
                        return Ok(false);
                    }
                    for (p, v) in patterns.iter().zip(values.iter()) {
                        if !self.pattern_matches_bind(p, v, bindings)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
            (Pattern::Tuple(patterns), Value::Tuple(values)) => {
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (p, v) in patterns.iter().zip(values.iter()) {
                    if !self.pattern_matches_bind(p, v, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // Result pattern matching
            (Pattern::Ok(inner_pattern), Value::Ok(inner_value)) => {
                self.pattern_matches_bind(inner_pattern, inner_value, bindings)
            }
            (Pattern::Err(inner_pattern), Value::Err(inner_value)) => {
                self.pattern_matches_bind(inner_pattern, inner_value, bindings)
            }
            (Pattern::Ok(_), _) => Ok(false), // Ok pattern doesn't match non-Ok values
            (Pattern::Err(_), _) => Ok(false), // Err pattern doesn't match non-Err values
            // Enum variant patterns - now with proper enum value handling
            (
                Pattern::EnumVariant {
                    variant_name,
                    patterns,
                },
                Value::Enum {
                    variant_name: val_variant,
                    variant_data,
                    ..
                },
            ) => {
                // Check if variant names match
                if variant_name != val_variant {
                    return Ok(false);
                }

                // Match based on the variant data type
                match variant_data {
                    EnumVariantData::Unit => {
                        // Unit variants should have no patterns
                        Ok(patterns.is_empty())
                    }
                    EnumVariantData::Tuple(values) => {
                        // Tuple variants should match against the contained values
                        if patterns.len() != values.len() {
                            return Ok(false);
                        }
                        for (p, v) in patterns.iter().zip(values.iter()) {
                            if !self.pattern_matches_bind(p, v, bindings)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    EnumVariantData::Struct(fields) => {
                        // For struct variants, patterns should match field values
                        if patterns.len() != fields.len() {
                            return Ok(false);
                        }
                        // Positional patterns carry no field names, so match in
                        // field-name order — HashMap iteration order would make
                        // multi-field matches succeed or fail nondeterministically
                        let mut field_values: Vec<(&String, &Value)> = fields.iter().collect();
                        field_values.sort_by_key(|(name, _)| name.as_str());
                        for (p, (_, v)) in patterns.iter().zip(field_values.iter()) {
                            if !self.pattern_matches_bind(p, v, bindings)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                }
            }
            // Fallback for old tuple-based enum handling (for compatibility)
            (
                Pattern::EnumVariant {
                    variant_name: _,
                    patterns,
                },
                Value::Tuple(values),
            ) => {
                // Keep backward compatibility with tuple-based enum handling
                if patterns.len() != values.len() {
                    return Ok(false);
                }
                for (p, v) in patterns.iter().zip(values.iter()) {
                    if !self.pattern_matches_bind(p, v, bindings)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // Struct patterns
            (
                Pattern::Struct {
                    type_name: _,
                    field_patterns,
                },
                Value::Struct { fields, .. },
            ) => {
                for (field_name, pattern) in field_patterns {
                    if let Some(field_value) = fields.get(field_name) {
                        if !self.pattern_matches_bind(pattern, field_value, bindings)? {
                            return Ok(false);
                        }
                    } else {
                        return Ok(false); // Field not found
                    }
                }
                Ok(true)
            }
            // Anonymous struct patterns
            (Pattern::AnonymousStruct { field_patterns }, Value::Struct { fields, .. }) => {
                for (field_name, pattern) in field_patterns {
                    if let Some(field_value) = fields.get(field_name) {
                        if !self.pattern_matches_bind(pattern, field_value, bindings)? {
                            return Ok(false);
                        }
                    } else {
                        return Ok(false); // Field not found
                    }
                }
                Ok(true)
            }
            // Range patterns for integers
            (
                Pattern::Range {
                    start,
                    end,
                    inclusive,
                },
                Value::Integer(n),
            ) => {
                let start_val = match start.as_ref() {
                    Pattern::Literal(Value::Integer(s)) => *s,
                    _ => return Ok(false),
                };
                let end_val = match end.as_ref() {
                    Pattern::Literal(Value::Integer(e)) => *e,
                    _ => return Ok(false),
                };

                if *inclusive {
                    Ok(*n >= start_val && *n <= end_val)
                } else {
                    Ok(*n >= start_val && *n < end_val)
                }
            }
            // Range patterns for single-character strings
            (
                Pattern::Range {
                    start,
                    end,
                    inclusive,
                },
                Value::String(s),
            ) => {
                let start_char = match start.as_ref() {
                    Pattern::Literal(Value::String(sv)) => sv.chars().next().unwrap_or('\0'),
                    _ => return Ok(false),
                };
                let end_char = match end.as_ref() {
                    Pattern::Literal(Value::String(ev)) => ev.chars().next().unwrap_or('\0'),
                    _ => return Ok(false),
                };
                let ch = s.chars().next().unwrap_or('\0');

                if *inclusive {
                    Ok(ch >= start_char && ch <= end_char)
                } else {
                    Ok(ch >= start_char && ch < end_char)
                }
            }
            // Or patterns
            (Pattern::Or { alternatives }, val) => {
                for alt_pattern in alternatives {
                    let mut alt_bindings = HashMap::new();
                    if self.pattern_matches_bind(alt_pattern, val, &mut alt_bindings)? {
                        // Merge bindings from the matching alternative
                        bindings.extend(alt_bindings);
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // Guarded patterns (guards are handled at a higher level)
            (Pattern::Guarded { pattern, .. }, val) => {
                // For guarded patterns, just check if the inner pattern matches
                // The guard will be evaluated separately in eval_match
                self.pattern_matches_bind(pattern, val, bindings)
            }
            // Rest patterns (standalone rest patterns should not appear in normal matching)
            (Pattern::Rest(_), _) => {
                // This should not happen in well-formed patterns as rest patterns
                // are only valid inside list patterns
                Ok(false)
            }
            _ => Ok(false),
        }
    }
}
