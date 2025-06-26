use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Error types for HTTP operations
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("Request failed: {message}")]
    RequestFailed { message: String },
    #[error("Invalid URL: {url}")]
    InvalidUrl { url: String },
    #[error("Connection timeout: {url}")]
    Timeout { url: String },
    #[error("Server error: {message}")]
    ServerError { message: String },
    #[error("JSON parsing error: {message}")]
    JsonError { message: String },
}

impl From<reqwest::Error> for HttpError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            HttpError::Timeout {
                url: err.url().map(|u| u.to_string()).unwrap_or_default(),
            }
        } else {
            HttpError::RequestFailed {
                message: err.to_string(),
            }
        }
    }
}

impl From<url::ParseError> for HttpError {
    fn from(err: url::ParseError) -> Self {
        HttpError::InvalidUrl {
            url: err.to_string(),
        }
    }
}

/// Creates the http module with all HTTP functions
pub fn create_http_module() -> Value {
    let mut module = HashMap::new();

    // HTTP Client operations
    module.insert("get".to_string(), create_builtin_function("get", 1));
    module.insert("post".to_string(), create_builtin_function("post", 2));
    module.insert("put".to_string(), create_builtin_function("put", 2));
    module.insert("delete".to_string(), create_builtin_function("delete", 1));
    module.insert("request".to_string(), create_builtin_function("request", 3));

    // HTTP Server operations
    module.insert("serve".to_string(), create_builtin_function("serve", 2));
    module.insert(
        "response".to_string(),
        create_builtin_function("response", 2),
    );
    module.insert(
        "response_with_headers".to_string(),
        create_builtin_function("response_with_headers", 3),
    );

    // Utility functions
    module.insert(
        "parse_url".to_string(),
        create_builtin_function("parse_url", 1),
    );
    module.insert(
        "encode_query".to_string(),
        create_builtin_function("encode_query", 1),
    );
    module.insert(
        "decode_query".to_string(),
        create_builtin_function("decode_query", 1),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: module,
    }
}

/// Helper function to create builtin function values
fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("http.{}", name),
        arity,
    })
}

/// Main dispatcher for HTTP function calls
pub fn call_http_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "get" => http_get(args),
        "post" => http_post(args),
        "put" => http_put(args),
        "delete" => http_delete(args),
        "request" => http_request(args),
        "serve" => http_serve(args),
        "response" => http_response(args),
        "response_with_headers" => http_response_with_headers(args),
        "parse_url" => parse_url(args),
        "encode_query" => encode_query(args),
        "decode_query" => decode_query(args),
        _ => Err(format!("Unknown http function: {}", name).into()),
    }
}

/// Make a GET request
/// Usage: http.get("https://api.example.com/data") -> Result<HttpResponse, Error>
fn http_get(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "get expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let url = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "get: URL must be a string".to_string(),
            )))))
        }
    };

    match reqwest::blocking::get(url) {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            match response.text() {
                Ok(body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(body)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );

                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: response_map,
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "GET request failed: {}",
            e
        )))))),
    }
}

/// Make a POST request
/// Usage: http.post("https://api.example.com/data", "request body") -> Result<HttpResponse, Error>
fn http_post(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "post expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let url = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "post: URL must be a string".to_string(),
            )))))
        }
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "post: body must be a string".to_string(),
            )))))
        }
    };

    let client = reqwest::blocking::Client::new();
    match client.post(url).body(body.to_string()).send() {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            match response.text() {
                Ok(response_body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(response_body)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );

                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: response_map,
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "POST request failed: {}",
            e
        )))))),
    }
}

/// Make a PUT request
/// Usage: http.put("https://api.example.com/data", "request body") -> Result<HttpResponse, Error>
fn http_put(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "put expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let url = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "put: URL must be a string".to_string(),
            )))))
        }
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "put: body must be a string".to_string(),
            )))))
        }
    };

    let client = reqwest::blocking::Client::new();
    match client.put(url).body(body.to_string()).send() {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            match response.text() {
                Ok(response_body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(response_body)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );

                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: response_map,
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "PUT request failed: {}",
            e
        )))))),
    }
}

/// Make a DELETE request
/// Usage: http.delete("https://api.example.com/data") -> Result<HttpResponse, Error>
fn http_delete(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "delete expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let url = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "delete: URL must be a string".to_string(),
            )))))
        }
    };

    let client = reqwest::blocking::Client::new();
    match client.delete(url).send() {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            match response.text() {
                Ok(response_body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(response_body)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );

                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: response_map,
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "DELETE request failed: {}",
            e
        )))))),
    }
}

/// Make a custom HTTP request
/// Usage: http.request("GET", "https://api.example.com", "body") -> Result<HttpResponse, Error>
fn http_request(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "request expects 3 arguments, got {}",
            args.len()
        ))))));
    }

    let method = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "request: method must be a string".to_string(),
            )))))
        }
    };

    let url = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "request: URL must be a string".to_string(),
            )))))
        }
    };

    let body = match &args[2] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "request: body must be a string".to_string(),
            )))))
        }
    };

    let client = reqwest::blocking::Client::new();
    let request_builder = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "Unsupported HTTP method: {}",
                method
            ))))))
        }
    };

    let request_builder =
        if !body.is_empty() && method.to_uppercase() != "GET" && method.to_uppercase() != "HEAD" {
            request_builder.body(body.to_string())
        } else {
            request_builder
        };

    match request_builder.send() {
        Ok(response) => {
            let status = response.status().as_u16() as i64;
            match response.text() {
                Ok(response_body) => {
                    let mut response_map = HashMap::new();
                    response_map.insert("status".to_string(), Value::Integer(status));
                    response_map.insert("body".to_string(), Value::String(Arc::new(response_body)));
                    response_map.insert(
                        "success".to_string(),
                        Value::Boolean((200..300).contains(&status)),
                    );

                    Ok(Value::Ok(Box::new(Value::Struct {
                        type_name: "HttpResponse".to_string(),
                        fields: response_map,
                    })))
                }
                Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                    "Failed to read response body: {}",
                    e
                )))))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "{} request failed: {}",
            method, e
        )))))),
    }
}

/// Start an HTTP server (simplified implementation for basic use)
/// Usage: http.serve(8080, handler_function) -> Result<Unit, Error>
fn http_serve(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "serve expects 2 arguments, got {}",
            args.len()
        ))))));
    }

    let port = match &args[0] {
        Value::Integer(p) => *p,
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "serve: port must be an integer".to_string(),
            )))))
        }
    };

    // For now, return a placeholder since implementing a full HTTP server
    // requires async runtime integration with the interpreter
    Ok(Value::Ok(Box::new(Value::String(Arc::new(format!(
        "HTTP server would start on port {} (placeholder implementation)",
        port
    ))))))
}

/// Create an HTTP response struct
/// Usage: http.response(200, "Hello World") -> HttpResponse
fn http_response(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("response expects 2 arguments, got {}", args.len()).into());
    }

    let status = match &args[0] {
        Value::Integer(s) => *s,
        _ => return Err("response: status must be an integer".into()),
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("response: body must be a string".into()),
    };

    let mut response_map = HashMap::new();
    response_map.insert("status".to_string(), Value::Integer(status));
    response_map.insert(
        "body".to_string(),
        Value::String(Arc::new(body.to_string())),
    );
    response_map.insert(
        "headers".to_string(),
        Value::Struct {
            type_name: "Headers".to_string(),
            fields: HashMap::new(),
        },
    );

    Ok(Value::Struct {
        type_name: "HttpResponse".to_string(),
        fields: response_map,
    })
}

/// Create an HTTP response with headers
/// Usage: http.response_with_headers(200, "Hello", headers) -> HttpResponse
fn http_response_with_headers(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!(
            "response_with_headers expects 3 arguments, got {}",
            args.len()
        )
        .into());
    }

    let status = match &args[0] {
        Value::Integer(s) => *s,
        _ => return Err("response_with_headers: status must be an integer".into()),
    };

    let body = match &args[1] {
        Value::String(s) => s.as_ref(),
        _ => return Err("response_with_headers: body must be a string".into()),
    };

    let headers = match &args[2] {
        Value::Struct { fields, .. } => fields.clone(),
        _ => return Err("response_with_headers: headers must be a struct".into()),
    };

    let mut response_map = HashMap::new();
    response_map.insert("status".to_string(), Value::Integer(status));
    response_map.insert(
        "body".to_string(),
        Value::String(Arc::new(body.to_string())),
    );
    response_map.insert(
        "headers".to_string(),
        Value::Struct {
            type_name: "Headers".to_string(),
            fields: headers,
        },
    );

    Ok(Value::Struct {
        type_name: "HttpResponse".to_string(),
        fields: response_map,
    })
}

/// Parse a URL into components
/// Usage: http.parse_url("https://example.com/path?query=value") -> Result<UrlInfo, Error>
fn parse_url(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "parse_url expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let url_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "parse_url: URL must be a string".to_string(),
            )))))
        }
    };

    match url::Url::parse(url_str) {
        Ok(url) => {
            let mut url_map = HashMap::new();
            url_map.insert(
                "scheme".to_string(),
                Value::String(Arc::new(url.scheme().to_string())),
            );
            url_map.insert(
                "host".to_string(),
                Value::String(Arc::new(url.host_str().unwrap_or("").to_string())),
            );
            url_map.insert(
                "port".to_string(),
                Value::Integer(url.port().unwrap_or(0) as i64),
            );
            url_map.insert(
                "path".to_string(),
                Value::String(Arc::new(url.path().to_string())),
            );
            url_map.insert(
                "query".to_string(),
                Value::String(Arc::new(url.query().unwrap_or("").to_string())),
            );
            url_map.insert(
                "fragment".to_string(),
                Value::String(Arc::new(url.fragment().unwrap_or("").to_string())),
            );

            Ok(Value::Ok(Box::new(Value::Struct {
                type_name: "UrlInfo".to_string(),
                fields: url_map,
            })))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "Failed to parse URL: {}",
            e
        )))))),
    }
}

/// Encode query parameters from a struct
/// Usage: http.encode_query(params_struct) -> Result<String, Error>
fn encode_query(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "encode_query expects 1 argument, got {}",
            args.len()
        ))))));
    }

    match &args[0] {
        Value::Struct { fields, .. } => {
            let mut query_pairs = Vec::new();
            for (key, value) in fields {
                let value_str = match value {
                    Value::String(s) => s.as_ref().clone(),
                    Value::Integer(n) => n.to_string(),
                    Value::Float(f) => f.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => continue, // Skip non-primitive values
                };
                query_pairs.push(format!(
                    "{}={}",
                    urlencoding::encode(key),
                    urlencoding::encode(&value_str)
                ));
            }
            Ok(Value::Ok(Box::new(Value::String(Arc::new(
                query_pairs.join("&"),
            )))))
        }
        _ => Ok(Value::Err(Box::new(Value::String(Arc::new(
            "encode_query: argument must be a struct".to_string(),
        ))))),
    }
}

/// Decode query string into a struct  
/// Usage: http.decode_query("key1=value1&key2=value2") -> Result<Struct, Error>
fn decode_query(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "decode_query expects 1 argument, got {}",
            args.len()
        ))))));
    }

    let query_str = match &args[0] {
        Value::String(s) => s.as_ref(),
        _ => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(
                "decode_query: query string must be a string".to_string(),
            )))))
        }
    };

    let mut params = HashMap::new();

    for pair in query_str.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let decoded_key = urlencoding::decode(key).unwrap_or_default();
            let decoded_value = urlencoding::decode(value).unwrap_or_default();
            params.insert(
                decoded_key.to_string(),
                Value::String(Arc::new(decoded_value.to_string())),
            );
        }
    }

    Ok(Value::Ok(Box::new(Value::Struct {
        type_name: "QueryParams".to_string(),
        fields: params,
    })))
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

    #[test]
    fn test_create_http_module() {
        let module = create_http_module();
        if let Value::Struct { fields, .. } = module {
            // Test that all expected functions are present
            let expected_functions = vec![
                "get",
                "post",
                "put",
                "delete",
                "request",
                "serve",
                "response",
                "response_with_headers",
                "parse_url",
                "encode_query",
                "decode_query",
            ];

            for func_name in expected_functions {
                assert!(
                    fields.contains_key(func_name),
                    "Missing function: {}",
                    func_name
                );
                if let Value::Builtin(builtin) = &fields[func_name] {
                    assert_eq!(builtin.name, format!("http.{}", func_name));
                } else {
                    panic!("Expected builtin function for {}", func_name);
                }
            }

            assert_eq!(fields.len(), 11, "Expected 11 functions in http module");
        } else {
            panic!("Expected struct for http module");
        }
    }

    #[test]
    fn test_http_response_creation() {
        // Test basic response creation
        let result = http_response(vec![int_val(200), string_val("Hello World")]).unwrap();

        if let Value::Struct { type_name, fields } = result {
            assert_eq!(type_name, "HttpResponse");
            assert_eq!(fields["status"], int_val(200));
            assert_eq!(fields["body"], string_val("Hello World"));
            assert!(matches!(fields["headers"], Value::Struct { .. }));
        } else {
            panic!("Expected HttpResponse struct");
        }
    }

    #[test]
    fn test_http_response_with_headers() {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), string_val("application/json"));
        headers.insert("X-Custom-Header".to_string(), string_val("test-value"));
        let headers_struct = struct_val("Headers", headers.clone());

        let result =
            http_response_with_headers(vec![int_val(201), string_val("Created"), headers_struct])
                .unwrap();

        if let Value::Struct { type_name, fields } = result {
            assert_eq!(type_name, "HttpResponse");
            assert_eq!(fields["status"], int_val(201));
            assert_eq!(fields["body"], string_val("Created"));

            if let Value::Struct {
                fields: header_fields,
                ..
            } = &fields["headers"]
            {
                assert_eq!(
                    header_fields["Content-Type"],
                    string_val("application/json")
                );
                assert_eq!(header_fields["X-Custom-Header"], string_val("test-value"));
            } else {
                panic!("Expected headers struct");
            }
        } else {
            panic!("Expected HttpResponse struct");
        }
    }

    #[test]
    fn test_parse_url_success() {
        let test_cases = vec![
            (
                "https://example.com:8080/path/to/resource?query=value&foo=bar#section",
                (
                    "https",
                    "example.com",
                    8080,
                    "/path/to/resource",
                    "query=value&foo=bar",
                    "section",
                ),
            ),
            (
                "http://localhost/api",
                ("http", "localhost", 0, "/api", "", ""),
            ),
            (
                "https://api.github.com/repos/owner/repo",
                ("https", "api.github.com", 0, "/repos/owner/repo", "", ""),
            ),
        ];

        for (url, expected) in test_cases {
            let result = parse_url(vec![string_val(url)]).unwrap();
            let parsed = assert_ok(&result);

            if let Value::Struct { fields, .. } = parsed {
                assert_eq!(fields["scheme"], string_val(expected.0));
                assert_eq!(fields["host"], string_val(expected.1));
                assert_eq!(fields["port"], int_val(expected.2));
                assert_eq!(fields["path"], string_val(expected.3));
                assert_eq!(fields["query"], string_val(expected.4));
                assert_eq!(fields["fragment"], string_val(expected.5));
            } else {
                panic!("Expected UrlInfo struct for URL: {}", url);
            }
        }
    }

    #[test]
    fn test_parse_url_invalid() {
        let invalid_urls = vec![
            "not-a-url",
            "ftp://", // incomplete
            "://missing-scheme",
        ];

        for invalid_url in invalid_urls {
            let result = parse_url(vec![string_val(invalid_url)]).unwrap();
            assert_err(&result); // Should be an error
        }
    }

    #[test]
    fn test_encode_query() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), string_val("John Doe"));
        params.insert("age".to_string(), int_val(30));
        params.insert("active".to_string(), bool_val(true));
        params.insert("score".to_string(), Value::Float(85.5));

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            // Query parameters can be in any order, so check individual parts
            assert!(query.contains("name=John%20Doe"));
            assert!(query.contains("age=30"));
            assert!(query.contains("active=true"));
            assert!(query.contains("score=85.5"));
            assert!(query.chars().filter(|&c| c == '&').count() == 3); // 4 params = 3 separators
        } else {
            panic!("Expected string result from encode_query");
        }
    }

    #[test]
    fn test_decode_query() {
        let query_string = "name=John%20Doe&age=30&city=New%20York&active=true";
        let result = decode_query(vec![string_val(query_string)]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("John Doe"));
            assert_eq!(fields["age"], string_val("30"));
            assert_eq!(fields["city"], string_val("New York"));
            assert_eq!(fields["active"], string_val("true"));
        } else {
            panic!("Expected QueryParams struct from decode_query");
        }
    }

    #[test]
    fn test_decode_query_empty() {
        let result = decode_query(vec![string_val("")]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert!(fields.is_empty());
        } else {
            panic!("Expected empty QueryParams struct");
        }
    }

    #[test]
    fn test_http_serve_placeholder() {
        // Test the placeholder server function
        let result = http_serve(vec![int_val(8080), string_val("handler")]).unwrap();
        let response = assert_ok(&result);

        if let Value::String(message) = response {
            assert!(message.contains("HTTP server would start on port 8080"));
            assert!(message.contains("placeholder implementation"));
        } else {
            panic!("Expected string response from serve");
        }
    }

    #[test]
    fn test_argument_validation_errors() {
        // Test various argument validation errors

        // http_get - wrong number of arguments
        let result = http_get(vec![]).unwrap();
        assert_err(&result);

        let result = http_get(vec![string_val("url"), string_val("extra")]).unwrap();
        assert_err(&result);

        // http_get - wrong argument type
        let result = http_get(vec![int_val(123)]).unwrap();
        assert_err(&result);

        // http_post - wrong number of arguments
        let result = http_post(vec![string_val("url")]).unwrap();
        assert_err(&result);

        // http_post - wrong argument types
        let result = http_post(vec![int_val(123), string_val("body")]).unwrap();
        assert_err(&result);

        let result = http_post(vec![string_val("url"), int_val(123)]).unwrap();
        assert_err(&result);

        // http_request - wrong number of arguments
        let result = http_request(vec![string_val("GET"), string_val("url")]).unwrap();
        assert_err(&result);

        // http_request - unsupported method
        let result = http_request(vec![
            string_val("INVALID"),
            string_val("http://example.com"),
            string_val(""),
        ])
        .unwrap();
        assert_err(&result);

        // parse_url - wrong number of arguments
        let result = parse_url(vec![]).unwrap();
        assert_err(&result);

        // parse_url - wrong argument type
        let result = parse_url(vec![int_val(123)]).unwrap();
        assert_err(&result);

        // encode_query - wrong argument type
        let result = encode_query(vec![string_val("not a struct")]).unwrap();
        assert_err(&result);

        // decode_query - wrong argument type
        let result = decode_query(vec![int_val(123)]).unwrap();
        assert_err(&result);

        // http_serve - wrong number of arguments
        let result = http_serve(vec![int_val(8080)]).unwrap();
        assert_err(&result);

        // http_serve - wrong argument type
        let result = http_serve(vec![string_val("not a port"), string_val("handler")]).unwrap();
        assert_err(&result);
    }

    #[test]
    fn test_response_creation_errors() {
        // Test http_response with wrong arguments
        let result = http_response(vec![string_val("not an int"), string_val("body")]);
        assert!(result.is_err());

        let result = http_response(vec![int_val(200), int_val(123)]);
        assert!(result.is_err());

        let result = http_response(vec![int_val(200)]);
        assert!(result.is_err());

        // Test http_response_with_headers with wrong arguments
        let result = http_response_with_headers(vec![
            string_val("not an int"),
            string_val("body"),
            struct_val("Headers", HashMap::new()),
        ]);
        assert!(result.is_err());

        let result = http_response_with_headers(vec![
            int_val(200),
            int_val(123),
            struct_val("Headers", HashMap::new()),
        ]);
        assert!(result.is_err());

        let result = http_response_with_headers(vec![
            int_val(200),
            string_val("body"),
            string_val("not a struct"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_call_http_function_dispatcher() {
        // Test that the dispatcher correctly routes to functions
        let result = call_http_function("response", vec![int_val(200), string_val("OK")]);
        assert!(result.is_ok());

        let result = call_http_function("parse_url", vec![string_val("http://example.com")]);
        assert!(result.is_ok());

        // Test unknown function
        let result = call_http_function("unknown_function", vec![]);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown http function"));
    }

    #[test]
    fn test_http_request_method_handling() {
        let supported_methods = vec!["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"];

        for method in supported_methods {
            let result = http_request(vec![
                string_val(method),
                string_val("http://httpbin.org/anything"), // This would fail in actual request
                string_val(""),
            ])
            .unwrap();

            // We expect this to be an error due to network, but not due to unsupported method
            if let Value::Err(error) = result {
                if let Value::String(error_msg) = error.as_ref() {
                    // Should not contain "Unsupported HTTP method"
                    assert!(!error_msg.contains("Unsupported HTTP method"));
                }
            }
        }
    }

    #[test]
    fn test_encode_query_with_special_characters() {
        let mut params = HashMap::new();
        params.insert("name".to_string(), string_val("John & Jane"));
        params.insert("email".to_string(), string_val("test@example.com"));
        params.insert("path".to_string(), string_val("/path/with spaces"));

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            assert!(query.contains("John%20%26%20Jane")); // URL-encoded "John & Jane"
            assert!(query.contains("test%40example.com")); // URL-encoded "test@example.com"
            assert!(query.contains("%2Fpath%2Fwith%20spaces")); // URL-encoded "/path/with spaces"
        } else {
            panic!("Expected string result from encode_query");
        }
    }

    #[test]
    fn test_decode_query_with_special_characters() {
        let query_string =
            "name=John%20%26%20Jane&email=test%40example.com&path=%2Fpath%2Fwith%20spaces";
        let result = decode_query(vec![string_val(query_string)]).unwrap();
        let decoded = assert_ok(&result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("John & Jane"));
            assert_eq!(fields["email"], string_val("test@example.com"));
            assert_eq!(fields["path"], string_val("/path/with spaces"));
        } else {
            panic!("Expected QueryParams struct from decode_query");
        }
    }

    #[test]
    fn test_query_encode_decode_roundtrip() {
        let mut original_params = HashMap::new();
        original_params.insert("name".to_string(), string_val("Test User"));
        original_params.insert("age".to_string(), int_val(25));
        original_params.insert("active".to_string(), bool_val(true));

        let params_struct = struct_val("QueryParams", original_params.clone());

        // Encode
        let encoded_result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&encoded_result);

        // Decode
        let decoded_result = decode_query(vec![encoded.clone()]).unwrap();
        let decoded = assert_ok(&decoded_result);

        if let Value::Struct { fields, .. } = decoded {
            assert_eq!(fields["name"], string_val("Test User"));
            assert_eq!(fields["age"], string_val("25")); // Numbers become strings in decode
            assert_eq!(fields["active"], string_val("true")); // Booleans become strings in decode
        } else {
            panic!("Expected QueryParams struct from roundtrip");
        }
    }

    #[test]
    fn test_encode_query_skips_complex_types() {
        let mut params = HashMap::new();
        params.insert("simple_string".to_string(), string_val("hello"));
        params.insert("simple_int".to_string(), int_val(42));
        params.insert(
            "complex_struct".to_string(),
            struct_val("ComplexType", HashMap::new()),
        );
        params.insert(
            "list".to_string(),
            Value::List(Arc::new([string_val("item1"), string_val("item2")])),
        );

        let params_struct = struct_val("QueryParams", params);
        let result = encode_query(vec![params_struct]).unwrap();
        let encoded = assert_ok(&result);

        if let Value::String(query_string) = encoded {
            let query = query_string.as_ref();
            assert!(query.contains("simple_string=hello"));
            assert!(query.contains("simple_int=42"));
            // Complex types should be skipped
            assert!(!query.contains("complex_struct"));
            assert!(!query.contains("list"));
        } else {
            panic!("Expected string result from encode_query");
        }
    }
}
