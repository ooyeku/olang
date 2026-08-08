// Validation Utilities Module
// This module provides validation functions

share fn is_email(email: String) = {
    true  // Simplified validation
}

share fn is_phone(phone: String) = {
    len(phone) > 5
}

share fn is_not_empty(text: String) = {
    len(text) > 0
}

// Shared type
share type ValidationResult = struct {
    valid: Bool,
    message: String
}
