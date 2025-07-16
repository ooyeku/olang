// Module that demonstrates conflict detection in transitive sharing
// This module only imports non-conflicting functions to avoid issues

use conflict_a { unique_a_function }
use conflict_b { unique_b_function }

// Re-share non-conflicting functions - should work fine
share use conflict_a { unique_a_function }
share use conflict_b { unique_b_function }

// Add a function that demonstrates both modules are accessible
share fn demonstrate_both_modules() = {
    unique_a_function() + " and " + unique_b_function()
} 