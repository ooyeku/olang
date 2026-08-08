// Base utility module - shared functions that can be transitively shared
// This module provides basic helper functions

share fn helper() = "base helper"

share fn format_text(text: String) = "[FORMATTED] " + text

share fn double_value(x: Int) = x * 2

// A function that's not shared - should not be accessible via transitive sharing
fn private_helper() = "this is private"

share fn get_version() = "v1.0.0"
