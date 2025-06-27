// Olang Dates Library Demo
// Demonstrates comprehensive date and time functionality

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

// Date creation
println("--- Date Creation ---")
let birthday = dates.date(1990, 5, 15)
let meeting_time = dates.datetime(2024, 12, 25, 14, 30, 0)
let lunch_time = dates.time(12, 30, 0)

println("Birthday: ", birthday)
println("Meeting datetime: ", meeting_time)
println("Lunch time: ", lunch_time)
println()

// Date parsing
println("--- Date Parsing ---")
let parsed_date = dates.parse_date("2024-07-04")
let parsed_datetime = dates.parse_datetime("2024-07-04T16:00:00")
let parsed_time = dates.parse_time("16:00:00")

println("Parsed date: ", parsed_date)
println("Parsed datetime: ", parsed_datetime)
println("Parsed time: ", parsed_time)
println()

// Date formatting
println("--- Date Formatting ---")
let formatted_date = dates.format_date("2024-07-04", "%B %d, %Y")
let formatted_datetime = dates.format_datetime("2024-07-04T16:00:00", "%B %d, %Y at %I:%M %p")
let formatted_time = dates.format_time("16:00:00", "%I:%M %p")

println("Formatted date: ", formatted_date)
println("Formatted datetime: ", formatted_datetime)
println("Formatted time: ", formatted_time)
println()

// Date arithmetic
println("--- Date Arithmetic ---")
let start_date = "2024-06-15"
let plus_week = dates.add_days(start_date, 7)
let plus_month = dates.add_months(start_date, 1)
let plus_year = dates.add_years(start_date, 1)
let minus_days = dates.add_days(start_date, -10)

println("Start date: ", start_date)
println("Plus 7 days: ", plus_week)
println("Plus 1 month: ", plus_month)
println("Plus 1 year: ", plus_year)
println("Minus 10 days: ", minus_days)

// Date difference
let date1 = "2024-06-22"
let date2 = "2024-06-15"
let diff = dates.diff_days(date1, date2)
println("Days between ", date1, " and ", date2, ": ", diff)
println()

// Component extraction
println("--- Date Component Extraction ---")
let sample_date = "2024-06-15"
let sample_datetime = "2024-06-15T14:30:45"

println("Date: ", sample_date)
println("Year: ", dates.year(sample_date))
println("Month: ", dates.month(sample_date))
println("Day: ", dates.day(sample_date))
println("Weekday: ", dates.weekday(sample_date), " (0=Sunday, 6=Saturday)")
println()

println("DateTime: ", sample_datetime)
println("Hour: ", dates.hour(sample_datetime))
println("Minute: ", dates.minute(sample_datetime))
println("Second: ", dates.second(sample_datetime))
println()

// Utility functions
println("--- Utility Functions ---")
let year2024 = 2024
let year2023 = 2023
let leap_check_2024 = dates.is_leap_year(year2024)
let leap_check_2023 = dates.is_leap_year(year2023)

println("Is ", year2024, " a leap year? ", leap_check_2024)
println("Is ", year2023, " a leap year? ", leap_check_2023)

let days_feb_2024 = dates.days_in_month(2024, 2)
let days_feb_2023 = dates.days_in_month(2023, 2)
println("Days in February 2024: ", days_feb_2024)
println("Days in February 2023: ", days_feb_2023)
println()

// Timestamp conversion
println("--- Timestamp Conversion ---")
let sample_dt = "2024-06-15T14:30:00"
let timestamp = dates.timestamp(sample_dt)
let back_to_datetime = dates.from_timestamp(timestamp)

println("DateTime: ", sample_dt)
println("As timestamp: ", timestamp)
println("Back to datetime: ", back_to_datetime)
println()

println("=== Demo Complete ===")
println("The dates library provides comprehensive date/time functionality")
println("with robust error handling and intuitive APIs!") 