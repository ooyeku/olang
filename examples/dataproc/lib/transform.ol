// A CsvRow has string cells; parse into a typed record.
share fn to_record(row) = {
    region: row.region,
    product: row.product,
    amount: unwrap(str.parse_float(row.amount)),
    quantity: unwrap(str.parse_int(row.quantity)),
    revenue: unwrap(str.parse_float(row.amount)) * to_float(unwrap(str.parse_int(row.quantity)))
}
