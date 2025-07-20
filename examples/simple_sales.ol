// Simple Sales Analyzer - Memory-Safe Version
// Demonstrates basic business intelligence without complex operations

println("=== Simple Sales Analyzer ===")
println()

// Create a small dataset with simplified structure
share fn create_sales_data() = {
    [
        { product: "Laptop", amount: 2000.0, category: "Electronics", rep: "Alice" },
        { product: "Mouse", amount: 50.0, category: "Electronics", rep: "Bob" },
        { product: "Chair", amount: 300.0, category: "Furniture", rep: "Carol" },
        { product: "Desk", amount: 800.0, category: "Furniture", rep: "Alice" },
        { product: "Monitor", amount: 400.0, category: "Electronics", rep: "Bob" }
    ]
}

// Simple total calculation using basic operations
share fn calculate_total_revenue(sales_data) = {
    let amounts = sales_data |> map((record) => record.amount)
    sum(amounts)
}

// Count total transactions
share fn count_transactions(sales_data) = {
    len(sales_data)
}

// Find electronics revenue using simple filter
share fn electronics_revenue(sales_data) = {
    let electronics = sales_data |> filter((record) => record.category == "Electronics")
    let amounts = electronics |> map((record) => record.amount)
    sum(amounts)
}

// Find furniture revenue
share fn furniture_revenue(sales_data) = {
    let furniture = sales_data |> filter((record) => record.category == "Furniture")
    let amounts = furniture |> map((record) => record.amount)
    sum(amounts)
}

// Main analysis function
share fn run_analysis() = {
    println("Starting analysis...")
    
    let sales = create_sales_data()
    println("Created " + to_string(len(sales)) + " sales records")
    
    let total = calculate_total_revenue(sales)
    println("Total Revenue: $" + to_string(total))
    
    let transactions = count_transactions(sales)
    println("Total Transactions: " + to_string(transactions))
    
    let electronics_total = electronics_revenue(sales)
    println("Electronics Revenue: $" + to_string(electronics_total))
    
    let furniture_total = furniture_revenue(sales)
    println("Furniture Revenue: $" + to_string(furniture_total))
    
    let avg_transaction = total / transactions
    println("Average Transaction: $" + to_string(avg_transaction))
    
    println()
    println("Analysis complete!")
    
    {
        total_revenue: total,
        total_transactions: transactions,
        electronics_revenue: electronics_total,
        furniture_revenue: furniture_total,
        average_transaction: avg_transaction
    }
}

// Run the analysis
run_analysis() 