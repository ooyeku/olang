// Olang Dates Library Demo - Simplified Version
// Demonstrates basic date and time functionality

println("=== Olang Dates Library Demo ===")
println()

// Current date and time functions
println("--- Current Date/Time ---")
let current_local = dates.now()
let current_utc = dates.utc_now()
let today = dates.today()

println("Current local time: ", current_local)
println("Current UTC time: ", current_utc)
println("Today's date: ", today)
println()

// Simple date creation
println("--- Date Creation ---")
let birthday = dates.date(90, 5, 15)
let lunch_time = dates.time(12, 30, 0)

println("Birthday: ", birthday)
println("Lunch time: ", lunch_time)
println()

// Date parsing
println("--- Date Parsing ---")
let parsed_date = dates.parse_date("24-07-04")
let parsed_time = dates.parse_time("16:00:00")

println("Parsed date: ", parsed_date)
println("Parsed time: ", parsed_time)
println()

// Date arithmetic
println("--- Date Arithmetic ---")
let start_date = "24-06-15"
let plus_week = dates.add_days(start_date, 7)
let plus_month = dates.add_months(start_date, 1)

println("Start date: ", start_date)
println("Plus 7 days: ", plus_week)
println("Plus 1 month: ", plus_month)
println()

// Date difference
let date1 = "24-06-22"
let date2 = "24-06-15"
let diff = dates.diff_days(date1, date2)
println("Days between ", date1, " and ", date2, ": ", diff)
println()

// Component extraction
println("--- Date Components ---")
let sample_date = "24-06-15"

println("Date: ", sample_date)
println("Year: ", dates.year(sample_date))
println("Month: ", dates.month(sample_date))
println("Day: ", dates.day(sample_date))
println()

// Utility functions
println("--- Utility Functions ---")
let year24 = 24
let leap_check = dates.is_leap_year(year24)

println("Is ", year24, " a leap year? ", leap_check)
println()

println("=== Demo Complete ===")
println("The dates library provides comprehensive date/time functionality!") 