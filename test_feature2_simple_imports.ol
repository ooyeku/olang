// Simple test file for Feature 2 cross-file imports
// This imports from test_feature2_simple_share_use.ol

// Test basic imports
use test_feature2_simple_share_use { add, PI }

fn test_imports() = {
    let result = add(10, 20)
    println("add(10, 20) = " + result)
    println("PI from import = " + PI)
}

test_imports() 