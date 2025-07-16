// test_module_import.ol - Test Feature 1 module importing

use math_utils { area_circle, area_square, PI }

fn main() = {
    println("Circle area (radius 5): " + area_circle(5.0))
    println("Square area (side 4): " + area_square(4.0))
    println("PI constant: " + PI)
} 