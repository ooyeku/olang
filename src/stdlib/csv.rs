use crate::ast::Value;
use csv::{ReaderBuilder, WriterBuilder};
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for CSV operations
#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    #[error("Parse error: {message}")]
    ParseError { message: String },
    #[error("Invalid CSV: {message}")]
    InvalidCsv { message: String },
    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Type error: {message}")]
    TypeError { message: String },
    #[error("Index out of bounds: {index}")]
    IndexOutOfBounds { index: usize },
    #[error("IO error: {message}")]
    IoError { message: String },
}

impl From<csv::Error> for CsvError {
    fn from(err: csv::Error) -> Self {
        match err.kind() {
            csv::ErrorKind::Io(_) => CsvError::IoError {
                message: err.to_string(),
            },
            _ => CsvError::ParseError {
                message: err.to_string(),
            },
        }
    }
}

/// Creates the csv module with all CSV functions
pub fn create_csv_module() -> Value {
    let mut module = HashMap::new();

    // Core parsing and serialization
    module.insert("parse".to_string(), create_builtin_function("parse", 1));
    module.insert(
        "parse_with_headers".to_string(),
        create_builtin_function("parse_with_headers", 1),
    );
    module.insert(
        "stringify".to_string(),
        create_builtin_function("stringify", 1),
    );
    module.insert(
        "stringify_with_headers".to_string(),
        create_builtin_function("stringify_with_headers", 2),
    );

    // Reading operations
    module.insert(
        "read_row".to_string(),
        create_builtin_function("read_row", 2),
    );
    module.insert(
        "read_column".to_string(),
        create_builtin_function("read_column", 2),
    );
    module.insert(
        "read_cell".to_string(),
        create_builtin_function("read_cell", 3),
    );
    module.insert(
        "get_headers".to_string(),
        create_builtin_function("get_headers", 1),
    );

    // Writing operations
    module.insert("add_row".to_string(), create_builtin_function("add_row", 2));
    module.insert(
        "add_column".to_string(),
        create_builtin_function("add_column", 2),
    );
    module.insert(
        "set_cell".to_string(),
        create_builtin_function("set_cell", 4),
    );
    module.insert(
        "set_headers".to_string(),
        create_builtin_function("set_headers", 2),
    );

    // Utility operations
    module.insert(
        "row_count".to_string(),
        create_builtin_function("row_count", 1),
    );
    module.insert(
        "column_count".to_string(),
        create_builtin_function("column_count", 1),
    );
    module.insert(
        "filter_rows".to_string(),
        create_builtin_function("filter_rows", 3),
    );
    module.insert(
        "sort_by_column".to_string(),
        create_builtin_function("sort_by_column", 2),
    );

    // Conversion operations
    module.insert("to_json".to_string(), create_builtin_function("to_json", 1));
    module.insert(
        "from_json".to_string(),
        create_builtin_function("from_json", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("csv.{}", name),
        arity,
    })
}

/// Main dispatcher for CSV function calls
pub fn call_csv_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "parse" => csv_parse(args),
        "parse_with_headers" => csv_parse_with_headers(args),
        "stringify" => csv_stringify(args),
        "stringify_with_headers" => csv_stringify_with_headers(args),
        "read_row" => csv_read_row(args),
        "read_column" => csv_read_column(args),
        "read_cell" => csv_read_cell(args),
        "get_headers" => csv_get_headers(args),
        "add_row" => csv_add_row(args),
        "add_column" => csv_add_column(args),
        "set_cell" => csv_set_cell(args),
        "set_headers" => csv_set_headers(args),
        "row_count" => csv_row_count(args),
        "column_count" => csv_column_count(args),
        "filter_rows" => csv_filter_rows(args),
        "sort_by_column" => csv_sort_by_column(args),
        "to_json" => csv_to_json(args),
        "from_json" => csv_from_json(args),
        _ => Err(format!("Unknown csv function: {}", name).into()),
    }
}

/// Parse a CSV string into a 2D array (list of lists)
/// Usage: csv.parse("name,age\nAlice,30\nBob,25") -> Result<[[String]], Error>
fn csv_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "parse expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let csv_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "parse: argument must be a string".to_string(),
            )))))
        }
    };

    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_str.as_bytes());

    let mut rows = Vec::new();
    for result in reader.records() {
        match result {
            Ok(record) => {
                let row: Vec<Value> = record
                    .iter()
                    .map(|field| Value::String(Arc::new(field.to_string())))
                    .collect();
                rows.push(Value::List(row.into()));
            }
            Err(e) => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "CSV parse error: {}",
                    e
                ))))));
            }
        }
    }

    Ok(Value::Ok(Box::new(Value::List(rows.into()))))
}

/// Parse a CSV string with headers into objects
/// Usage: csv.parse_with_headers("name,age\nAlice,30\nBob,25") -> Result<[{name: "Alice", age: "30"}], Error>
fn csv_parse_with_headers(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "parse_with_headers expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let csv_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "parse_with_headers: argument must be a string".to_string(),
            )))))
        }
    };

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(csv_str.as_bytes());

    let headers: Vec<String> = match reader.headers() {
        Ok(headers) => headers.iter().map(|h| h.to_string()).collect(),
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "CSV header error: {}",
                e
            ))))));
        }
    };

    let mut objects = Vec::new();
    for result in reader.records() {
        match result {
            Ok(record) => {
                let mut obj = HashMap::new();
                for (i, field) in record.iter().enumerate() {
                    if i < headers.len() {
                        obj.insert(
                            headers[i].clone(),
                            Value::String(Arc::new(field.to_string())),
                        );
                    }
                }
                objects.push(Value::Struct {
                    type_name: "CsvRow".to_string(),
                    fields: obj,
                });
            }
            Err(e) => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "CSV parse error: {}",
                    e
                ))))));
            }
        }
    }

    Ok(Value::Ok(Box::new(Value::List(objects.into()))))
}

/// Convert a 2D array to CSV string
/// Usage: csv.stringify([["name", "age"], ["Alice", "30"], ["Bob", "25"]]) -> String
fn csv_stringify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "stringify expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "stringify: argument must be a list of lists".to_string(),
            )))))
        }
    };

    let mut writer = WriterBuilder::new().from_writer(Vec::new());

    for row in rows.iter() {
        match row {
            Value::List(fields) => {
                let string_fields: Result<Vec<String>, _> = fields
                    .iter()
                    .map(|field| match field {
                        Value::String(s) => Ok(s.as_ref().clone()),
                        Value::Integer(i) => Ok(i.to_string()),
                        Value::Float(f) => Ok(f.to_string()),
                        Value::Boolean(b) => Ok(b.to_string()),
                        _ => Err("Field must be a string, number, or boolean"),
                    })
                    .collect();

                match string_fields {
                    Ok(fields) => {
                        if let Err(e) = writer.write_record(&fields) {
                            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                                "CSV write error: {}",
                                e
                            ))))));
                        }
                    }
                    Err(e) => {
                        return Ok(Value::Err(Box::new(Value::String(Arc::new(e.to_string())))));
                    }
                }
            }
            _ => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "Each row must be a list".to_string(),
                )))));
            }
        }
    }

    match writer.into_inner() {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(csv_string) => Ok(Value::String(Arc::new(csv_string))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "UTF-8 conversion error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "CSV finalization error: {}",
            e
        )))))),
    }
}

/// Convert objects to CSV string with headers
/// Usage: csv.stringify_with_headers([{name: "Alice", age: 30}], ["name", "age"]) -> Result<String, Error>
fn csv_stringify_with_headers(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "stringify_with_headers expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let objects = match &args[0] {
        Value::List(objects) => objects,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "stringify_with_headers: first argument must be a list of objects".to_string(),
            )))))
        }
    };

    let headers = match &args[1] {
        Value::List(headers) => headers,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "stringify_with_headers: second argument must be a list of strings".to_string(),
            )))))
        }
    };

    let header_strings: Result<Vec<String>, _> = headers
        .iter()
        .map(|h| match h {
            Value::String(s) => Ok(s.as_ref().clone()),
            _ => Err("Headers must be strings"),
        })
        .collect();

    let header_strings = match header_strings {
        Ok(hs) => hs,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(e.to_string())))));
        }
    };

    let mut writer = WriterBuilder::new().from_writer(Vec::new());

    // Write headers
    if let Err(e) = writer.write_record(&header_strings) {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "CSV header write error: {}",
            e
        ))))));
    }

    // Write data rows
    for obj in objects.iter() {
        match obj {
            Value::Struct { fields, .. } => {
                let row_data: Vec<String> = header_strings
                    .iter()
                    .map(|header| {
                        fields
                            .get(header)
                            .map(|value| match value {
                                Value::String(s) => s.as_ref().clone(),
                                Value::Integer(i) => i.to_string(),
                                Value::Float(f) => f.to_string(),
                                Value::Boolean(b) => b.to_string(),
                                _ => "".to_string(),
                            })
                            .unwrap_or_else(|| "".to_string())
                    })
                    .collect();

                if let Err(e) = writer.write_record(&row_data) {
                    return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                        "CSV row write error: {}",
                        e
                    ))))));
                }
            }
            _ => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "Each object must be a struct".to_string(),
                )))));
            }
        }
    }

    match writer.into_inner() {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(csv_string) => Ok(Value::Ok(Box::new(Value::String(Arc::new(csv_string))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "UTF-8 conversion error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "CSV finalization error: {}",
            e
        )))))),
    }
}

/// Read a specific row from CSV data
/// Usage: csv.read_row(csv_data, 1) -> Result<[String], Error>
fn csv_read_row(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "read_row expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_row: first argument must be a list".to_string(),
            )))))
        }
    };

    let index = match &args[1] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_row: second argument must be an integer".to_string(),
            )))))
        }
    };

    if index >= rows.len() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Index {} out of bounds for {} rows",
            index,
            rows.len()
        ))))));
    }

    Ok(Value::Ok(Box::new(rows[index].clone())))
}

/// Read a specific column from CSV data
/// Usage: csv.read_column(csv_data, 0) -> Result<[String], Error>
fn csv_read_column(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "read_column expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_column: first argument must be a list".to_string(),
            )))))
        }
    };

    let col_index = match &args[1] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_column: second argument must be an integer".to_string(),
            )))))
        }
    };

    let mut column = Vec::new();
    for row in rows.iter() {
        match row {
            Value::List(fields) => {
                if col_index < fields.len() {
                    column.push(fields[col_index].clone());
                } else {
                    column.push(Value::String(Arc::new("".to_string())));
                }
            }
            _ => {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(
                    "Each row must be a list".to_string(),
                )))));
            }
        }
    }

    Ok(Value::Ok(Box::new(Value::List(column.into()))))
}

/// Read a specific cell from CSV data
/// Usage: csv.read_cell(csv_data, 1, 0) -> Result<String, Error>
fn csv_read_cell(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "read_cell expects 3 arguments, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_cell: first argument must be a list".to_string(),
            )))))
        }
    };

    let row_index = match &args[1] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_cell: second argument must be an integer".to_string(),
            )))))
        }
    };

    let col_index = match &args[2] {
        Value::Integer(i) => *i as usize,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "read_cell: third argument must be an integer".to_string(),
            )))))
        }
    };

    if row_index >= rows.len() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Row index {} out of bounds for {} rows",
            row_index,
            rows.len()
        ))))));
    }

    match &rows[row_index] {
        Value::List(fields) => {
            if col_index >= fields.len() {
                return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Column index {} out of bounds for {} columns",
                    col_index,
                    fields.len()
                ))))));
            }
            Ok(Value::Ok(Box::new(fields[col_index].clone())))
        }
        _ => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "Row must be a list".to_string(),
        ))))),
    }
}

/// Get headers from CSV data (first row)
/// Usage: csv.get_headers(csv_data) -> Result<[String], Error>
fn csv_get_headers(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "get_headers expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "get_headers: argument must be a list".to_string(),
            )))))
        }
    };

    if rows.is_empty() {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "CSV data is empty".to_string(),
        )))));
    }

    Ok(Value::Ok(Box::new(rows[0].clone())))
}

/// Get row count from CSV data
/// Usage: csv.row_count(csv_data) -> Result<Int, Error>
fn csv_row_count(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "row_count expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "row_count: argument must be a list".to_string(),
            )))))
        }
    };

    Ok(Value::Ok(Box::new(Value::Integer(rows.len() as i64))))
}

/// Get column count from CSV data (based on first row)
/// Usage: csv.column_count(csv_data) -> Result<Int, Error>
fn csv_column_count(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "column_count expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let rows = match &args[0] {
        Value::List(rows) => rows,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "column_count: argument must be a list".to_string(),
            )))))
        }
    };

    if rows.is_empty() {
        return Ok(Value::Ok(Box::new(Value::Integer(0))));
    }

    match &rows[0] {
        Value::List(fields) => Ok(Value::Ok(Box::new(Value::Integer(fields.len() as i64)))),
        _ => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "First row must be a list".to_string(),
        ))))),
    }
}

/// Placeholder implementations for remaining functions
fn csv_add_row(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "add_row: Not yet implemented".to_string(),
    )))))
}

fn csv_add_column(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "add_column: Not yet implemented".to_string(),
    )))))
}

fn csv_set_cell(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "set_cell: Not yet implemented".to_string(),
    )))))
}

fn csv_set_headers(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "set_headers: Not yet implemented".to_string(),
    )))))
}

fn csv_filter_rows(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "filter_rows: Not yet implemented".to_string(),
    )))))
}

fn csv_sort_by_column(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "sort_by_column: Not yet implemented".to_string(),
    )))))
}

fn csv_to_json(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "to_json: Not yet implemented".to_string(),
    )))))
}

fn csv_from_json(_args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(Value::Err(Box::new(Value::String(Arc::new(
        "from_json: Not yet implemented".to_string(),
    )))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Value;
    use std::collections::HashMap;
    use std::sync::Arc;

    // Helper functions to create test values
    fn string_val(s: &str) -> Value {
        Value::String(Arc::new(s.to_string()))
    }

    fn int_val(n: i64) -> Value {
        Value::Integer(n)
    }

    fn bool_val(b: bool) -> Value {
        Value::Boolean(b)
    }

    fn list_val(items: Vec<Value>) -> Value {
        Value::List(items.into())
    }

    fn struct_val(type_name: &str, fields: HashMap<String, Value>) -> Value {
        Value::Struct {
            type_name: type_name.to_string(),
            fields,
        }
    }

    // Helper to assert Result<T, E> success
    fn assert_ok(result: &Value) -> &Value {
        match result {
            Value::Ok(inner) => inner.as_ref(),
            _ => panic!("Expected Ok result, got: {:?}", result),
        }
    }

    // Helper to assert Result<T, E> error
    fn assert_err(result: &Value) -> &Value {
        match result {
            Value::Err(inner) => inner.as_ref(),
            _ => panic!("Expected Err result, got: {:?}", result),
        }
    }

    // Helper to extract string from Value::String
    fn extract_string(value: &Value) -> &str {
        match value {
            Value::String(s) => s.as_ref(),
            _ => panic!("Expected string value, got: {:?}", value),
        }
    }

    // Helper to extract integer from Value::Integer
    fn extract_int(value: &Value) -> i64 {
        match value {
            Value::Integer(i) => *i,
            _ => panic!("Expected integer value, got: {:?}", value),
        }
    }

    // Helper to extract list from Value::List
    fn extract_list(value: &Value) -> &[Value] {
        match value {
            Value::List(list) => list.as_ref(),
            _ => panic!("Expected list value, got: {:?}", value),
        }
    }

    #[test]
    fn test_csv_module_creation() {
        let module = create_csv_module();

        if let Value::Struct { type_name, fields } = module {
            assert_eq!(type_name, "Module");

            // Check that all expected functions are present
            let expected_functions = vec![
                "parse",
                "parse_with_headers",
                "stringify",
                "stringify_with_headers",
                "read_row",
                "read_column",
                "read_cell",
                "get_headers",
                "add_row",
                "add_column",
                "set_cell",
                "set_headers",
                "row_count",
                "column_count",
                "filter_rows",
                "sort_by_column",
                "to_json",
                "from_json",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );

                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("csv.{}", func_name));
                } else {
                    panic!("Expected builtin function for {}", func_name);
                }
            }
        } else {
            panic!("Expected struct for csv module");
        }
    }

    #[test]
    fn test_csv_parse() {
        // Test successful parsing
        let csv_data = "name,age\nAlice,30\nBob,25";
        let result = csv_parse(vec![string_val(csv_data)]).unwrap();
        let parsed = assert_ok(&result);
        let rows = extract_list(parsed);

        assert_eq!(rows.len(), 3);

        // Check first row (headers)
        let first_row = extract_list(&rows[0]);
        assert_eq!(first_row.len(), 2);
        assert_eq!(extract_string(&first_row[0]), "name");
        assert_eq!(extract_string(&first_row[1]), "age");

        // Check second row
        let second_row = extract_list(&rows[1]);
        assert_eq!(second_row.len(), 2);
        assert_eq!(extract_string(&second_row[0]), "Alice");
        assert_eq!(extract_string(&second_row[1]), "30");

        // Test error conditions
        let result = csv_parse(vec![int_val(42)]).unwrap();
        assert_err(&result);

        let result = csv_parse(vec![]).unwrap();
        assert_err(&result);

        let result = csv_parse(vec![string_val("test"), string_val("extra")]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_parse_with_headers() {
        // Test successful parsing with headers
        let csv_data = "name,age\nAlice,30\nBob,25";
        let result = csv_parse_with_headers(vec![string_val(csv_data)]).unwrap();
        let parsed = assert_ok(&result);
        let rows = extract_list(parsed);

        assert_eq!(rows.len(), 2); // Should exclude header row

        // Check first data row (should be a struct)
        if let Value::Struct { type_name, fields } = &rows[0] {
            assert_eq!(type_name, "CsvRow");
            assert_eq!(extract_string(fields.get("name").unwrap()), "Alice");
            assert_eq!(extract_string(fields.get("age").unwrap()), "30");
        } else {
            panic!("Expected struct for CSV row");
        }

        // Test with empty CSV - should return empty list, not error
        let result = csv_parse_with_headers(vec![string_val("")]).unwrap();
        let parsed = assert_ok(&result);
        let rows = extract_list(parsed);
        assert_eq!(rows.len(), 0);

        // Test error conditions
        let result = csv_parse_with_headers(vec![int_val(42)]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_stringify() {
        // Test successful stringification - should return String directly, not Result
        let data = list_val(vec![
            list_val(vec![string_val("name"), string_val("age")]),
            list_val(vec![string_val("Alice"), string_val("30")]),
            list_val(vec![string_val("Bob"), string_val("25")]),
        ]);

        let result = csv_stringify(vec![data]).unwrap();

        // Should return String directly, not wrapped in Ok()
        let csv_string = extract_string(&result);
        assert!(csv_string.contains("name,age"));
        assert!(csv_string.contains("Alice,30"));
        assert!(csv_string.contains("Bob,25"));

        // Test with different data types
        let mixed_data = list_val(vec![
            list_val(vec![
                string_val("name"),
                string_val("age"),
                string_val("active"),
            ]),
            list_val(vec![string_val("Alice"), int_val(30), bool_val(true)]),
        ]);

        let result = csv_stringify(vec![mixed_data]).unwrap();
        let csv_string = extract_string(&result);
        assert!(csv_string.contains("Alice,30,true"));

        // Test error conditions - should return Err
        let result = csv_stringify(vec![string_val("not a list")]).unwrap();
        assert_err(&result);

        let result = csv_stringify(vec![]).unwrap();
        assert_err(&result);

        // Test with invalid row data
        let invalid_data = list_val(vec![
            list_val(vec![string_val("name")]),
            string_val("not a list"), // Invalid row
        ]);
        let result = csv_stringify(vec![invalid_data]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_stringify_with_headers() {
        // Test successful stringification with headers
        let mut alice_fields = HashMap::new();
        alice_fields.insert("name".to_string(), string_val("Alice"));
        alice_fields.insert("age".to_string(), int_val(30));

        let mut bob_fields = HashMap::new();
        bob_fields.insert("name".to_string(), string_val("Bob"));
        bob_fields.insert("age".to_string(), int_val(25));

        let objects = list_val(vec![
            struct_val("Person", alice_fields),
            struct_val("Person", bob_fields),
        ]);

        let headers = list_val(vec![string_val("name"), string_val("age")]);

        let result = csv_stringify_with_headers(vec![objects, headers]).unwrap();
        let parsed = assert_ok(&result);
        let csv_string = extract_string(parsed);

        assert!(csv_string.contains("name,age"));
        assert!(csv_string.contains("Alice,30"));
        assert!(csv_string.contains("Bob,25"));

        // Test error conditions
        let result = csv_stringify_with_headers(vec![string_val("not a list")]).unwrap();
        assert_err(&result);

        let result = csv_stringify_with_headers(vec![list_val(vec![]), int_val(42)]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_read_operations() {
        let csv_data = list_val(vec![
            list_val(vec![string_val("name"), string_val("age")]),
            list_val(vec![string_val("Alice"), string_val("30")]),
            list_val(vec![string_val("Bob"), string_val("25")]),
        ]);

        // Test read_row
        let result = csv_read_row(vec![csv_data.clone(), int_val(1)]).unwrap();
        let row = assert_ok(&result);
        let row_list = extract_list(row);
        assert_eq!(extract_string(&row_list[0]), "Alice");
        assert_eq!(extract_string(&row_list[1]), "30");

        // Test read_row out of bounds
        let result = csv_read_row(vec![csv_data.clone(), int_val(10)]).unwrap();
        assert_err(&result);

        // Test read_column
        let result = csv_read_column(vec![csv_data.clone(), int_val(0)]).unwrap();
        let column = assert_ok(&result);
        let column_list = extract_list(column);
        assert_eq!(column_list.len(), 3);
        assert_eq!(extract_string(&column_list[0]), "name");
        assert_eq!(extract_string(&column_list[1]), "Alice");
        assert_eq!(extract_string(&column_list[2]), "Bob");

        // Test read_cell
        let result = csv_read_cell(vec![csv_data.clone(), int_val(1), int_val(0)]).unwrap();
        let cell = assert_ok(&result);
        assert_eq!(extract_string(cell), "Alice");

        // Test read_cell out of bounds
        let result = csv_read_cell(vec![csv_data, int_val(1), int_val(10)]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_utility_functions() {
        let csv_data = list_val(vec![
            list_val(vec![string_val("name"), string_val("age")]),
            list_val(vec![string_val("Alice"), string_val("30")]),
            list_val(vec![string_val("Bob"), string_val("25")]),
        ]);

        // Test get_headers
        let result = csv_get_headers(vec![csv_data.clone()]).unwrap();
        let headers = assert_ok(&result);
        let headers_list = extract_list(headers);
        assert_eq!(headers_list.len(), 2);
        assert_eq!(extract_string(&headers_list[0]), "name");
        assert_eq!(extract_string(&headers_list[1]), "age");

        // Test row_count
        let result = csv_row_count(vec![csv_data.clone()]).unwrap();
        let count = assert_ok(&result);
        assert_eq!(extract_int(count), 3);

        // Test column_count
        let result = csv_column_count(vec![csv_data]).unwrap();
        let count = assert_ok(&result);
        assert_eq!(extract_int(count), 2);

        // Test with empty data
        let empty_data = list_val(vec![]);
        let result = csv_row_count(vec![empty_data.clone()]).unwrap();
        let count = assert_ok(&result);
        assert_eq!(extract_int(count), 0);

        let result = csv_column_count(vec![empty_data]).unwrap();
        let count = assert_ok(&result);
        assert_eq!(extract_int(count), 0);
    }

    #[test]
    fn test_csv_error_handling() {
        // Test various error conditions

        // Wrong number of arguments
        let result = csv_parse(vec![]).unwrap();
        assert_err(&result);

        let result = csv_stringify(vec![]).unwrap();
        assert_err(&result);

        // Wrong argument types
        let result = csv_parse(vec![int_val(42)]).unwrap();
        assert_err(&result);

        let result = csv_read_row(vec![string_val("not a list"), int_val(0)]).unwrap();
        assert_err(&result);

        let result = csv_read_row(vec![list_val(vec![]), string_val("not an int")]).unwrap();
        assert_err(&result);

        // Invalid CSV structure
        let invalid_csv_data = list_val(vec![
            string_val("not a row"), // Should be a list
        ]);
        let result = csv_stringify(vec![invalid_csv_data]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_csv_function_dispatcher() {
        // Test that the dispatcher correctly routes function calls
        let csv_data = list_val(vec![
            list_val(vec![string_val("name"), string_val("age")]),
            list_val(vec![string_val("Alice"), string_val("30")]),
        ]);

        // Test parse function
        let result = call_csv_function("parse", vec![string_val("name,age\nAlice,30")]).unwrap();
        assert_ok(&result);

        // Test stringify function
        let result = call_csv_function("stringify", vec![csv_data.clone()]).unwrap();
        // stringify should return String directly, not wrapped in Ok()
        assert!(matches!(result, Value::String(_)));

        // Test row_count function
        let result = call_csv_function("row_count", vec![csv_data]).unwrap();
        let count = assert_ok(&result);
        assert_eq!(extract_int(count), 2);

        // Test unknown function
        let result = call_csv_function("unknown_function", vec![]).unwrap_err();
        assert!(result.to_string().contains("Unknown csv function"));
    }

    #[test]
    fn test_csv_roundtrip() {
        // Test that we can stringify and then parse data back
        let original_data = list_val(vec![
            list_val(vec![
                string_val("name"),
                string_val("age"),
                string_val("city"),
            ]),
            list_val(vec![
                string_val("Alice"),
                string_val("30"),
                string_val("NYC"),
            ]),
            list_val(vec![string_val("Bob"), string_val("25"), string_val("LA")]),
        ]);

        // Stringify the data
        let stringify_result = csv_stringify(vec![original_data.clone()]).unwrap();
        let csv_string = extract_string(&stringify_result);

        // Parse it back
        let parse_result =
            csv_parse(vec![Value::String(Arc::new(csv_string.to_string()))]).unwrap();
        let parsed_data = assert_ok(&parse_result);

        // Compare with original
        let parsed_list = extract_list(parsed_data);
        let original_list = extract_list(&original_data);

        assert_eq!(parsed_list.len(), original_list.len());

        // Check first row
        let parsed_first_row = extract_list(&parsed_list[0]);
        let original_first_row = extract_list(&original_list[0]);
        assert_eq!(parsed_first_row.len(), original_first_row.len());
        assert_eq!(
            extract_string(&parsed_first_row[0]),
            extract_string(&original_first_row[0])
        );
    }

    #[test]
    fn test_csv_special_characters() {
        // Test CSV with special characters (commas, quotes, newlines)
        let data_with_special_chars = list_val(vec![
            list_val(vec![string_val("name"), string_val("description")]),
            list_val(vec![string_val("Alice"), string_val("Works at ABC, Inc.")]),
            list_val(vec![string_val("Bob"), string_val("Likes \"coding\"")]),
        ]);

        let result = csv_stringify(vec![data_with_special_chars]).unwrap();
        let csv_string = extract_string(&result);

        // Should handle special characters properly
        assert!(csv_string.contains("Alice"));
        assert!(csv_string.contains("Bob"));

        // Parse it back to ensure it's valid CSV
        let parse_result =
            csv_parse(vec![Value::String(Arc::new(csv_string.to_string()))]).unwrap();
        assert_ok(&parse_result);
    }

    #[test]
    fn test_unimplemented_functions() {
        // Test that unimplemented functions return appropriate errors
        let placeholder_functions = vec![
            "add_row",
            "add_column",
            "set_cell",
            "set_headers",
            "filter_rows",
            "sort_by_column",
            "to_json",
            "from_json",
        ];

        for func_name in placeholder_functions {
            let result = call_csv_function(func_name, vec![]).unwrap();
            let error = assert_err(&result);
            let error_msg = extract_string(error);
            assert!(error_msg.contains("Not yet implemented"));
        }
    }
}
