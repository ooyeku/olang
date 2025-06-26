use crate::ast::Value;
use std::collections::HashMap;

pub mod base64;
pub mod crypto;
pub mod csv;
pub mod dates;
pub mod fs;
pub mod http;
pub mod json;
pub mod math;
pub mod os;
pub mod random;
pub mod testing;

pub fn get_stdlib() -> HashMap<String, Value> {
    let mut stdlib = HashMap::new();
    stdlib.insert("base64".to_string(), base64::create_base64_module());
    stdlib.insert("crypto".to_string(), crypto::create_crypto_module());
    stdlib.insert("csv".to_string(), csv::create_csv_module());
    stdlib.insert("dates".to_string(), dates::create_dates_module());
    stdlib.insert("fs".to_string(), fs::create_fs_module());
    stdlib.insert("http".to_string(), http::create_http_module());
    stdlib.insert("json".to_string(), json::create_json_module());
    stdlib.insert("math".to_string(), math::create_math_module());
    stdlib.insert("os".to_string(), os::create_os_module());
    stdlib.insert("random".to_string(), random::create_random_module());
    stdlib.insert("testing".to_string(), testing::create_testing_module());
    stdlib
}
