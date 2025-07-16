// Module A with functions that will create naming conflicts
// Used to test conflict resolution in transitive sharing

share fn common_function() = "From Module A"

share fn process_data(data: String) = "A: " + data

share fn unique_a_function() = "Only in A" 