// Enhanced String Literals & Template Interpolation Examples

// Basic template interpolation
let name = "Sarah"
let score = 95
let message = `Hello ${name}, your score is ${score}%`
println(message)  // Output: Hello Sarah, your score is 95%

// Math expressions in templates
let width = 10
let height = 20
let area_info = `Rectangle: ${width} x ${height} = ${width * height} square units`
println(area_info)  // Output: Rectangle: 10 x 20 = 200 square units

// Nested object access in templates
let person = {
    name: "John",
    address: {
        city: "New York",
        zip: "10001"
    }
}
let location = `${person.name} lives in ${person.address.city}, ${person.address.zip}`
println(location)  // Output: John lives in New York, 10001

// Advanced escape sequences
let unicode_text = "Greek letters: \u{03B1}\u{03B2}\u{03B3}"  // αβγ
let hex_colors = "RGB: \xFF\x00\x00 is red"  // RGB: ...
let special_chars = "Quote: \" Backslash: \\ Newline: \n Tab: \t"

// Raw strings (no escape processing)
let windows_path = r"C:\Users\Documents\file.txt"
let regex = r"^\d{3}-\d{2}-\d{4}$"  // SSN pattern
let sql_query = r"SELECT * FROM users WHERE name = '${name}'"

// Multi-line templates
let grade = if score >= 90 => "A" else => if score >= 80 => "B" else => "C"
let status = if score >= 70 => "PASS" else => "FAIL"
let report = `
Performance Report
==================
Name: ${person.name}
Score: ${score}/100
Grade: ${grade}
Status: ${status}
`

println(report)
