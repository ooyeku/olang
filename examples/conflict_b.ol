// Module B with functions that create naming conflicts with Module A
// Used to test conflict resolution in transitive sharing

share fn common_function() = "From Module B"

share fn process_data(data: String) = "B: " + data

share fn unique_b_function() = "Only in B"
