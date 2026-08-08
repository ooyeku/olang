// Extended utility module - demonstrates transitive sharing
// This module imports functions from base_utils and re-shares some of them

use base_utils { helper, format_text, double_value }

// Transitive sharing: re-share helper and format_text from base_utils
share use base_utils { helper, format_text }

// Add new functionality specific to this module
share fn extended_helper() = helper() + " extended"

share fn enhanced_format(text: String) = format_text(text) + " [ENHANCED]"

// Use imported function internally but don't re-share it
fn internal_calculation(x: Int) = double_value(x) + 10
