#!/bin/bash

# OVM Performance Benchmark Runner
echo "=== OVM Performance Benchmark Runner ==="
echo "This script runs performance tests for SIMD, Pipeline, and other optimizations"
echo ""

# Check if olang binary exists
if [ ! -f "../target/release/olang" ] && [ ! -f "../target/debug/olang" ]; then
    echo "Building Olang..."
    cd ..
    cargo build --release
    cd examples
fi

# Determine which binary to use
OLANG_BIN="../target/release/olang"
if [ ! -f "$OLANG_BIN" ]; then
    OLANG_BIN="../target/debug/olang"
fi

if [ ! -f "$OLANG_BIN" ]; then
    echo "Error: Could not find olang binary. Please run 'cargo build' first."
    exit 1
fi

echo "Using binary: $OLANG_BIN"
echo ""

# Run the SIMD benchmark
echo "Running SIMD and Pipeline Performance Benchmark..."
echo "======================================================="
time $OLANG_BIN simd_benchmark.ol

echo ""
echo "======================================================="
echo "Benchmark completed!"
echo ""
echo "Performance Analysis:"
echo "- Large computation results indicate SIMD vectorization is working"
echo "- Fast execution with large datasets shows pipeline optimization"
echo "- Consistent performance across runs indicates JIT compilation"
echo ""
echo "To see more detailed performance:"
echo "1. Run multiple times to see JIT compilation effects"
echo "2. Increase data sizes in the .ol file for more pronounced SIMD benefits"
echo "3. Monitor memory usage to see GC optimization effects"